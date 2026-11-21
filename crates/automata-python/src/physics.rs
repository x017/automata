//! Python bindings for physics queries and per-frame physics operations.
//!
//! The engine steps the physics world before the update callback. Games
//! interact with physics through these views:
//!
//! - **State queries** (`velocity`, `grounded`, `body_count`, `gravity`)
//!   read a snapshot taken right after the physics step.
//! - **Raycasts** (`cast_ray`) are queued with a result key; the engine runs
//!   them after the callback and the results become readable **next frame**
//!   via [`PyPhysics::ray_result`]. This keeps the Python/Rust borrow story
//!   safe (no live world references cross into Python).
//! - **Mutations** (`apply_impulse`, `set_gravity`, ...) are queued as ops
//!   and applied right after the callback returns.
//!
//! Rapier itself is never exposed.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use automata_physics::{PhysicsWorld2D, PhysicsWorld3D, RayFilter};
use automata_scene::{Entity as EntityId, SceneWorld};
use glam::Vec3;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use crate::scene::entity_from_bits;

/// A queued physics mutation, applied after the update callback.
#[derive(Debug, Clone)]
pub enum PhysicsOp {
    /// Apply an impulse to a 3D dynamic body.
    Impulse3D {
        /// Target entity.
        entity: EntityId,
        /// Impulse vector.
        impulse: Vec3,
    },
    /// Apply a force to a 3D dynamic body.
    Force3D {
        /// Target entity.
        entity: EntityId,
        /// Force vector.
        force: Vec3,
    },
    /// Set a 3D kinematic-velocity body's velocity.
    SetVelocity3D {
        /// Target entity.
        entity: EntityId,
        /// Velocity vector.
        velocity: Vec3,
    },
    /// Set 3D world gravity.
    SetGravity3D {
        /// Gravity vector.
        gravity: Vec3,
    },
    /// Apply an impulse to a 2D dynamic body.
    Impulse2D {
        /// Target entity.
        entity: EntityId,
        /// Impulse vector.
        impulse: Vec3,
    },
}

/// A queued raycast, resolved after the update callback.
#[derive(Debug, Clone)]
pub struct QueuedRay {
    /// Result slot key (auto-incremented).
    pub key: u64,
    /// Ray origin.
    pub origin: Vec3,
    /// Ray direction (need not be normalized).
    pub direction: Vec3,
    /// Maximum distance.
    pub max_distance: f32,
    /// Entity to exclude (if any).
    pub exclude: Option<EntityId>,
}

/// A resolved raycast result.
#[derive(Debug, Clone, Copy)]
pub struct RayResult {
    /// Hit entity (if any).
    pub entity: Option<EntityId>,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Distance along the ray.
    pub distance: f32,
}

/// Physics ops and raycasts queued during one frame, plus the results of
/// last frame's raycasts.
#[derive(Debug, Default)]
pub struct PhysicsFrame {
    /// Mutations to apply after the callback.
    pub ops: Vec<PhysicsOp>,
    /// Raycasts to resolve after the callback.
    pub rays: Vec<QueuedRay>,
    /// Results of the previous frame's raycasts, keyed by result key.
    pub results: HashMap<u64, Option<RayResult>>,
    /// Next result key.
    pub next_key: u64,
}

/// Shared frame state between the physics views and the game loop.
pub type PhysicsQueue = Rc<RefCell<PhysicsFrame>>;

/// Applies queued mutations and resolves queued raycasts.
pub fn apply_physics_ops(
    world3d: &mut PhysicsWorld3D,
    world2d: &mut PhysicsWorld2D,
    queue: &PhysicsQueue,
) {
    let (ops, rays) = {
        let mut q = queue.borrow_mut();
        (std::mem::take(&mut q.ops), std::mem::take(&mut q.rays))
    };
    for op in ops {
        match op {
            PhysicsOp::Impulse3D { entity, impulse } => world3d.apply_impulse(entity, impulse),
            PhysicsOp::Force3D { entity, force } => world3d.apply_force(entity, force),
            PhysicsOp::SetVelocity3D { entity, velocity } => {
                world3d.set_velocity(entity, velocity);
            }
            PhysicsOp::SetGravity3D { gravity } => world3d.set_gravity(gravity),
            PhysicsOp::Impulse2D { entity, impulse } => world2d.apply_impulse(entity, impulse),
        }
    }
    let mut results = HashMap::new();
    for ray in rays {
        let mut filter = RayFilter::everything();
        if let Some(ex) = ray.exclude {
            filter = filter.excluding(ex);
        }
        let hit = world3d
            .cast_ray(ray.origin, ray.direction, ray.max_distance, &filter)
            .map(|h| RayResult {
                entity: Some(h.entity),
                point: h.point,
                normal: h.normal,
                distance: h.distance,
            });
        results.insert(ray.key, hit);
    }
    queue.borrow_mut().results = results;
}

/// Extracts an entity id from an int (entity bits) or an `EntityView`.
fn entity_id(obj: &Bound<'_, PyAny>) -> PyResult<EntityId> {
    if let Ok(bits) = obj.extract::<u64>() {
        return entity_from_bits(bits);
    }
    if let Ok(view) = obj.cast::<crate::scene::EntityView>() {
        return entity_from_bits(view.borrow().id_bits());
    }
    Err(PyTypeError::new_err(format!(
        "expected an entity id (int) or EntityView, got {}",
        obj.get_type().name()?
    )))
}

/// Per-frame 3D physics view: queries against the stepped world plus queued
/// mutations. Obtained via `ctx.physics`.
#[pyclass(name = "Physics", module = "automata._core", unsendable)]
#[derive(Clone)]
pub struct PyPhysics {
    body_count: usize,
    gravity: (f32, f32, f32),
    velocities: Rc<HashMap<u64, (f32, f32, f32)>>,
    grounded: Rc<HashSet<u64>>,
    colliders: Rc<automata_physics::ColliderSnapshot>,
    water: Rc<Vec<automata_renderer::WaterSurface>>,
    queue: PhysicsQueue,
}

impl std::fmt::Debug for PyPhysics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Physics")
            .field("body_count", &self.body_count)
            .finish_non_exhaustive()
    }
}

impl PyPhysics {
    /// Snapshots the facts game code can query this frame.
    pub(crate) fn snapshot(
        world: &PhysicsWorld3D,
        scene: &SceneWorld,
        queue: PhysicsQueue,
    ) -> Self {
        let mut velocities = HashMap::new();
        let mut grounded = HashSet::new();
        for (entity, _) in
            &mut scene.query::<(automata_scene::Entity, &automata_physics::RigidBody)>()
        {
            let v = world.velocity(entity);
            if v != Vec3::ZERO {
                velocities.insert(entity.to_bits().get(), (v.x, v.y, v.z));
            }
            if world.grounded(entity) {
                grounded.insert(entity.to_bits().get());
            }
        }
        let g = world.gravity();
        Self {
            body_count: world.body_count(),
            gravity: (g.x, g.y, g.z),
            velocities: Rc::new(velocities),
            grounded: Rc::new(grounded),
            colliders: Rc::new(world.collider_snapshot()),
            water: Rc::new(world.water_surfaces().to_vec()),
            queue,
        }
    }
}

#[pymethods]
impl PyPhysics {
    /// Casts a ray right now and returns the closest hit as `(entity,
    /// point, normal, distance)`, or `None`. Tests the colliders as they
    /// were at the start of the frame (after the physics step); sensors
    /// are skipped. `exclude` (an entity or id) ignores the caster itself.
    #[pyo3(signature = (origin, direction, max_distance = 1000.0, exclude = None))]
    #[allow(clippy::type_complexity)] // the tuple is the documented Python shape
    fn raycast(
        &self,
        origin: (f32, f32, f32),
        direction: (f32, f32, f32),
        max_distance: f32,
        exclude: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<(u64, (f32, f32, f32), (f32, f32, f32), f32)>> {
        let mut filter = RayFilter::no_sensors();
        if let Some(e) = exclude.map(entity_id).transpose()? {
            filter = filter.excluding(e);
        }
        Ok(self
            .colliders
            .cast_ray(
                Vec3::new(origin.0, origin.1, origin.2),
                Vec3::new(direction.0, direction.1, direction.2),
                max_distance,
                &filter,
            )
            .map(|h| {
                (
                    h.entity.to_bits().get(),
                    (h.point.x, h.point.y, h.point.z),
                    (h.normal.x, h.normal.y, h.normal.z),
                    h.distance,
                )
            }))
    }

    /// The height of the water surface at world `(x, z)`, waves included
    /// (what floating bodies ride), or `None` where there's no water.
    fn water_height(&self, x: f32, z: f32) -> Option<f32> {
        self.water.iter().find_map(|s| s.height_at(x, z))
    }

    /// Queue a raycast; returns a result key. The hit is readable **next
    /// frame** via [`ray_result`](Self::ray_result).
    ///
    /// `direction` need not be normalized.
    #[pyo3(signature = (origin, direction, max_distance = 1000.0, exclude = None))]
    fn cast_ray(
        &mut self,
        origin: (f32, f32, f32),
        direction: (f32, f32, f32),
        max_distance: f32,
        exclude: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<u64> {
        let exclude = exclude.map(entity_id).transpose()?;
        let mut q = self.queue.borrow_mut();
        let key = q.next_key;
        q.next_key += 1;
        q.rays.push(QueuedRay {
            key,
            origin: Vec3::new(origin.0, origin.1, origin.2),
            direction: Vec3::new(direction.0, direction.1, direction.2),
            max_distance,
            exclude,
        });
        Ok(key)
    }

    /// The result of a raycast queued last frame: `(entity, point, normal,
    /// distance)` or `None` (miss, or the key isn't from last frame).
    #[allow(clippy::type_complexity)] // the tuple is the documented Python shape
    fn ray_result(&self, key: u64) -> Option<(u64, (f32, f32, f32), (f32, f32, f32), f32)> {
        let result = *self.queue.borrow().results.get(&key)?;
        result.map(|r| {
            (
                r.entity.map_or(0, |e| e.to_bits().get()),
                (r.point.x, r.point.y, r.point.z),
                (r.normal.x, r.normal.y, r.normal.z),
                r.distance,
            )
        })
    }

    /// Apply an impulse to a dynamic body.
    fn apply_impulse(
        &mut self,
        entity: &Bound<'_, PyAny>,
        impulse: (f32, f32, f32),
    ) -> PyResult<()> {
        let entity = entity_id(entity)?;
        self.queue.borrow_mut().ops.push(PhysicsOp::Impulse3D {
            entity,
            impulse: Vec3::new(impulse.0, impulse.1, impulse.2),
        });
        Ok(())
    }

    /// Apply a force (cleared each step) to a dynamic body.
    fn apply_force(&mut self, entity: &Bound<'_, PyAny>, force: (f32, f32, f32)) -> PyResult<()> {
        let entity = entity_id(entity)?;
        self.queue.borrow_mut().ops.push(PhysicsOp::Force3D {
            entity,
            force: Vec3::new(force.0, force.1, force.2),
        });
        Ok(())
    }

    /// Current linear velocity of an entity's body.
    fn velocity(&self, entity: &Bound<'_, PyAny>) -> PyResult<(f32, f32, f32)> {
        let entity = entity_id(entity)?;
        Ok(self
            .velocities
            .get(&entity.to_bits().get())
            .copied()
            .unwrap_or((0.0, 0.0, 0.0)))
    }

    /// Set a kinematic-velocity body's velocity.
    fn set_velocity(
        &mut self,
        entity: &Bound<'_, PyAny>,
        velocity: (f32, f32, f32),
    ) -> PyResult<()> {
        let entity = entity_id(entity)?;
        self.queue.borrow_mut().ops.push(PhysicsOp::SetVelocity3D {
            entity,
            velocity: Vec3::new(velocity.0, velocity.1, velocity.2),
        });
        Ok(())
    }

    /// Whether the entity's character controller was grounded last step.
    fn grounded(&self, entity: &Bound<'_, PyAny>) -> PyResult<bool> {
        let entity = entity_id(entity)?;
        Ok(self.grounded.contains(&entity.to_bits().get()))
    }

    /// World gravity.
    fn gravity(&self) -> (f32, f32, f32) {
        self.gravity
    }

    /// Set world gravity.
    fn set_gravity(&mut self, gravity: (f32, f32, f32)) {
        self.queue.borrow_mut().ops.push(PhysicsOp::SetGravity3D {
            gravity: Vec3::new(gravity.0, gravity.1, gravity.2),
        });
    }

    /// Number of bodies in the 3D world.
    fn body_count(&self) -> usize {
        self.body_count
    }
}

/// Per-frame 2D physics view. Obtained via `ctx.physics2d`.
#[pyclass(name = "Physics2D", module = "automata._core", unsendable)]
#[derive(Clone)]
pub struct PyPhysics2D {
    body_count: usize,
    grounded: Rc<HashSet<u64>>,
    queue: PhysicsQueue,
}

impl std::fmt::Debug for PyPhysics2D {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Physics2D")
            .field("body_count", &self.body_count)
            .finish_non_exhaustive()
    }
}

impl PyPhysics2D {
    /// Snapshots 2D physics facts for the frame.
    pub(crate) fn snapshot(
        world: &PhysicsWorld2D,
        scene: &SceneWorld,
        queue: PhysicsQueue,
    ) -> Self {
        let mut grounded = HashSet::new();
        for (entity, _) in
            &mut scene.query::<(automata_scene::Entity, &automata_physics::RigidBody)>()
        {
            if world.grounded(entity) {
                grounded.insert(entity.to_bits().get());
            }
        }
        Self {
            body_count: world.body_count(),
            grounded: Rc::new(grounded),
            queue,
        }
    }
}

#[pymethods]
impl PyPhysics2D {
    /// Apply an impulse to a 2D dynamic body.
    fn apply_impulse(&mut self, entity: &Bound<'_, PyAny>, impulse: (f32, f32)) -> PyResult<()> {
        let entity = entity_id(entity)?;
        self.queue.borrow_mut().ops.push(PhysicsOp::Impulse2D {
            entity,
            impulse: Vec3::new(impulse.0, impulse.1, 0.0),
        });
        Ok(())
    }

    /// Whether the entity's 2D character controller was grounded last step.
    fn grounded(&self, entity: &Bound<'_, PyAny>) -> PyResult<bool> {
        let entity = entity_id(entity)?;
        Ok(self.grounded.contains(&entity.to_bits().get()))
    }

    /// Number of bodies in the 2D world.
    fn body_count(&self) -> usize {
        self.body_count
    }
}
