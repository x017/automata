//! 2D physics world implementation (`Rapier2D`).
//!
//! Positions use X/Y; rotation is the angle around Z. Everything else
//! mirrors the 3D world.

use std::collections::HashMap;

use automata_scene::{Entity, SceneWorld, Transform};
use glam::{Quat, Vec3};
use r2::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use r2::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet,
    RigidBodyBuilder, RigidBodySet,
};
use r2::geometry::{
    BroadPhaseBvh, ColliderBuilder, ColliderHandle, ColliderSet, CollisionEvent, NarrowPhase,
};
use r2::math::{Pose, Vector};
use r2::pipeline::ActiveEvents;
use r2::pipeline::{ChannelEventCollector, PhysicsPipeline, QueryFilter, QueryFilterFlags};
use rapier2d as r2;
use tracing::trace;

use crate::components::{
    BodyType, CharacterController, CharacterVelocity, Collider2D, RigidBody, Shape2D,
};
use crate::events::{ContactForce, PhysicsEvent};
use crate::world::{
    entity_to_user_data, user_data_to_entity, CharacterMove, PhysicsTick, RayFilter, RayHit,
};

/// The 2D physics world. Owns the Rapier sets; you own the ECS scene.
pub struct PhysicsWorld2D {
    gravity: Vec3,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    soft_bodies: r2::dynamics::SoftBodySet,
    body_entities: HashMap<r2::dynamics::RigidBodyHandle, Entity>,
    entity_bodies: HashMap<Entity, r2::dynamics::RigidBodyHandle>,
    entity_colliders: HashMap<Entity, ColliderHandle>,
    controllers: HashMap<Entity, KinematicCharacterController>,
    grounded: HashMap<Entity, bool>,
    last_dt: f32,
}

impl std::fmt::Debug for PhysicsWorld2D {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicsWorld2D")
            .field("bodies", &self.entity_bodies.len())
            .field("colliders", &self.entity_colliders.len())
            .field("controllers", &self.controllers.len())
            .finish_non_exhaustive()
    }
}

impl Default for PhysicsWorld2D {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld2D {
    /// An empty 2D world with Earth-like gravity along -Y (`-9.81` m/s²).
    #[must_use]
    pub fn new() -> Self {
        Self::with_gravity(Vec3::new(0.0, -9.81, 0.0))
    }

    /// An empty 2D world with the given gravity (X/Y are used).
    #[must_use]
    pub fn with_gravity(gravity: Vec3) -> Self {
        Self {
            gravity,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            soft_bodies: r2::dynamics::SoftBodySet::new(),
            body_entities: HashMap::new(),
            entity_bodies: HashMap::new(),
            entity_colliders: HashMap::new(),
            controllers: HashMap::new(),
            grounded: HashMap::new(),
            last_dt: 1.0 / 60.0,
        }
    }

    /// The current gravity vector.
    #[must_use]
    pub fn gravity(&self) -> Vec3 {
        self.gravity
    }

    /// Change the gravity for subsequent steps.
    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    /// Number of rigid bodies currently tracked.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.entity_bodies.len()
    }

    /// Number of colliders currently tracked.
    #[must_use]
    pub fn collider_count(&self) -> usize {
        self.entity_colliders.len()
    }

    fn entity_for_collider(&self, handle: ColliderHandle) -> Option<Entity> {
        let collider = self.colliders.get(handle)?;
        user_data_to_entity(collider.user_data)
    }

    /// Synchronise the Rapier world with the ECS scene: spawn bodies for new
    /// [`RigidBody`] entities, update kinematic targets, remove despawned
    /// entities. Called each fixed tick before [`step`](Self::step).
    pub fn sync_from_scene(&mut self, scene: &mut SceneWorld) {
        self.spawn_missing(scene);
        self.update_existing(scene);
        self.despawn_removed(scene);
    }

    fn spawn_missing(&mut self, scene: &mut SceneWorld) {
        let mut to_spawn = Vec::new();
        let mut bodies_query = scene.query::<(hecs::Entity, &RigidBody, Option<&Transform>)>();
        for (entity, body, transform) in &mut bodies_query {
            // 2D world only owns entities that carry a 2D collider; 3D
            // entities (Collider3D or no collider) belong to the 3D world.
            if !body.enabled
                || self.entity_bodies.contains_key(&entity)
                || scene.get::<&Collider2D>(entity).is_err()
            {
                continue;
            }
            let collider = scene.get::<&Collider2D>(entity).ok().map(|c| (*c).clone());
            to_spawn.push((entity, body.clone(), transform.copied(), collider));
        }

        for (entity, body, transform, collider) in to_spawn {
            let mut builder = match body.body_type {
                BodyType::Dynamic => RigidBodyBuilder::dynamic(),
                BodyType::Static => RigidBodyBuilder::fixed(),
                BodyType::KinematicVelocity => RigidBodyBuilder::kinematic_velocity_based(),
                BodyType::KinematicPosition => RigidBodyBuilder::kinematic_position_based(),
            }
            .gravity_scale(body.gravity_scale)
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping);

            if let Some(t) = transform {
                let angle = t.rotation.to_euler(glam::EulerRot::XYZ).2;
                builder = builder
                    .translation(Vector::new(t.position.x, t.position.y))
                    .rotation(angle);
            }

            let handle = self.bodies.insert(builder.build());
            self.entity_bodies.insert(entity, handle);
            self.body_entities.insert(handle, entity);
            trace!(?entity, "physics: spawned 2D body");

            if let Some(c) = collider {
                self.attach_collider(entity, handle, &c);
            }
        }
    }

    fn attach_collider(
        &mut self,
        entity: Entity,
        body: r2::dynamics::RigidBodyHandle,
        collider: &Collider2D,
    ) {
        let mut builder = match collider.shape {
            Shape2D::Ball { radius } => ColliderBuilder::ball(radius),
            Shape2D::Cuboid { extents } => {
                ColliderBuilder::cuboid(extents.x / 2.0, extents.y / 2.0)
            }
            Shape2D::Capsule {
                half_height,
                radius,
            } => ColliderBuilder::capsule_y(half_height, radius),
        }
        .density(collider.density)
        .friction(collider.friction)
        .restitution(collider.restitution)
        .sensor(collider.sensor)
        .active_events(ActiveEvents::COLLISION_EVENTS)
        .user_data(entity_to_user_data(entity));

        if !collider.enabled {
            builder = builder.enabled(false);
        }
        let handle = self
            .colliders
            .insert_with_parent(builder.build(), body, &mut self.bodies);
        self.entity_colliders.insert(entity, handle);
    }

    fn update_existing(&mut self, scene: &mut SceneWorld) {
        let mut updates = Vec::new();
        let mut bodies_query = scene.query::<(hecs::Entity, &RigidBody, Option<&Transform>)>();
        for (entity, body, transform) in &mut bodies_query {
            if !body.enabled {
                continue;
            }
            let Some(&handle) = self.entity_bodies.get(&entity) else {
                continue;
            };
            updates.push((body.clone(), transform.copied(), handle));
        }

        for (body, transform, handle) in updates {
            let Some(rb) = self.bodies.get_mut(handle) else {
                continue;
            };
            rb.set_gravity_scale(body.gravity_scale, true);
            rb.set_linear_damping(body.linear_damping);
            rb.set_angular_damping(body.angular_damping);
            if let (BodyType::KinematicPosition, Some(t)) = (body.body_type, transform) {
                let angle = t.rotation.to_euler(glam::EulerRot::XYZ).2;
                rb.set_next_kinematic_translation(Vector::new(t.position.x, t.position.y));
                rb.set_next_kinematic_rotation(r2::math::Rotation::new(angle));
            }
        }
    }

    fn despawn_removed(&mut self, scene: &mut SceneWorld) {
        let mut to_remove = Vec::new();
        for entity in self.entity_bodies.keys() {
            if scene.get::<&RigidBody>(*entity).is_err() {
                to_remove.push(*entity);
            }
        }
        for entity in to_remove {
            self.remove_entity(entity);
        }
    }

    fn remove_entity(&mut self, entity: Entity) {
        if let Some(body) = self.entity_bodies.remove(&entity) {
            self.body_entities.remove(&body);
            self.bodies.remove(
                body,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.soft_bodies,
                true,
            );
        }
        self.entity_colliders.remove(&entity);
        self.controllers.remove(&entity);
        self.grounded.remove(&entity);
        trace!(?entity, "physics: removed 2D body");
    }

    /// Move all character controllers by their [`CharacterVelocity`] * `dt`,
    /// resolving collisions. Call after [`sync_from_scene`](Self::sync_from_scene)
    /// and before [`step`](Self::step).
    pub fn move_characters(&mut self, scene: &mut SceneWorld, dt: f32) {
        let mut configs = Vec::new();
        for (entity, config) in &mut scene.query::<(hecs::Entity, &CharacterController)>() {
            configs.push((entity, config.clone()));
        }
        for (entity, config) in configs {
            self.controllers
                .entry(entity)
                .or_insert_with(|| controller_from_config(&config));
        }

        let mut moves = Vec::new();
        for (entity, velocity) in &mut scene.query::<(hecs::Entity, &CharacterVelocity)>() {
            if self.controllers.contains_key(&entity) {
                moves.push((entity, velocity.velocity));
            }
        }
        for (entity, velocity) in moves {
            let desired = velocity * dt;
            let mv = self.move_character(scene, entity, desired);
            self.grounded.insert(entity, mv.grounded);
        }
    }

    /// Move a character controller entity by `desired` displacement,
    /// resolving collisions. Returns the actual displacement applied.
    pub fn move_character(
        &mut self,
        scene: &mut SceneWorld,
        entity: Entity,
        desired: Vec3,
    ) -> CharacterMove {
        let Some(controller) = self.controllers.get(&entity).copied() else {
            return CharacterMove::default();
        };
        let Some(&body_handle) = self.entity_bodies.get(&entity) else {
            return CharacterMove::default();
        };
        let Some(&collider_handle) = self.entity_colliders.get(&entity) else {
            return CharacterMove::default();
        };
        let Some(collider) = self.colliders.get(collider_handle) else {
            return CharacterMove::default();
        };
        let character_shape = collider.shared_shape().clone();
        let character_pos = *collider.position();

        let query = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::default().exclude_rigid_body(body_handle),
        );
        let mut collisions = Vec::new();
        let movement = controller.move_shape(
            self.last_dt,
            &query,
            &*character_shape,
            &character_pos,
            Vector::new(desired.x, desired.y),
            |collision| collisions.push(collision),
        );

        let actual = Vec3::new(movement.translation.x, movement.translation.y, 0.0);

        if let Ok(mut transform) = scene.get::<&mut Transform>(entity) {
            transform.position += actual;
        }
        if let Some(body) = self.bodies.get_mut(body_handle) {
            let pos = body.translation();
            body.set_next_kinematic_translation(Vector::new(pos.x + actual.x, pos.y + actual.y));
        }

        CharacterMove {
            translation: actual,
            grounded: movement.grounded,
            sliding: movement.is_sliding_down_slope,
            collisions: collisions
                .iter()
                .filter_map(|c| self.entity_for_collider(c.handle))
                .collect(),
        }
    }

    /// Step the simulation by `dt` seconds and write transforms back.
    pub fn step(&mut self, scene: &mut SceneWorld, dt: f32) -> PhysicsTick {
        self.last_dt = dt;
        let params = IntegrationParameters {
            dt,
            ..IntegrationParameters::default()
        };

        let (collision_send, collision_recv) = std::sync::mpsc::channel();
        let (force_send, force_recv) = std::sync::mpsc::channel();
        let (tear_send, _tear_recv) = std::sync::mpsc::channel();
        let events = ChannelEventCollector::new(collision_send, force_send, tear_send);

        self.pipeline.step(
            Vector::new(self.gravity.x, self.gravity.y),
            &params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &(),
            &events,
        );

        let mut tick = PhysicsTick::default();
        while let Ok(event) = collision_recv.try_recv() {
            let mapped = match event {
                CollisionEvent::Started(a, b, _) => {
                    match (self.entity_for_collider(a), self.entity_for_collider(b)) {
                        (Some(entity_a), Some(entity_b)) => {
                            Some(PhysicsEvent::CollisionStarted { entity_a, entity_b })
                        }
                        _ => None,
                    }
                }
                CollisionEvent::Stopped(a, b, _) => {
                    match (self.entity_for_collider(a), self.entity_for_collider(b)) {
                        (Some(entity_a), Some(entity_b)) => {
                            Some(PhysicsEvent::CollisionStopped { entity_a, entity_b })
                        }
                        _ => None,
                    }
                }
            };
            if let Some(mapped) = mapped {
                tick.collisions.push(mapped);
            }
        }
        while let Ok(event) = force_recv.try_recv() {
            if let (Some(entity_a), Some(entity_b)) = (
                self.entity_for_collider(event.collider1),
                self.entity_for_collider(event.collider2),
            ) {
                tick.contact_forces.push(ContactForce {
                    entity_a,
                    entity_b,
                    total_force: Vec3::new(event.total_force.x, event.total_force.y, 0.0),
                    max_force_magnitude: event.max_force_magnitude,
                });
            }
        }

        self.write_back_transforms(scene);
        tick
    }

    fn write_back_transforms(&mut self, scene: &mut SceneWorld) {
        for (handle, entity) in &self.body_entities {
            let Some(body) = self.bodies.get(*handle) else {
                continue;
            };
            if !body.is_dynamic() {
                continue;
            }
            let pos = body.translation();
            let angle = body.rotation().angle();
            if let Ok(mut transform) = scene.get::<&mut Transform>(*entity) {
                transform.position = Vec3::new(pos.x, pos.y, transform.position.z);
                transform.rotation = Quat::from_rotation_z(angle);
            }
        }
    }

    /// Current linear velocity of an entity's body (zero if none).
    #[must_use]
    pub fn velocity(&self, entity: Entity) -> Vec3 {
        self.entity_bodies
            .get(&entity)
            .and_then(|h| self.bodies.get(*h))
            .map_or(Vec3::ZERO, |b| Vec3::new(b.linvel().x, b.linvel().y, 0.0))
    }

    /// Apply an impulse to a dynamic body's centre of mass.
    pub fn apply_impulse(&mut self, entity: Entity, impulse: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_impulse(Vector::new(impulse.x, impulse.y), true);
            }
        }
    }

    /// Apply a force to a dynamic body (cleared each step).
    pub fn apply_force(&mut self, entity: Entity, force: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_force(Vector::new(force.x, force.y), true);
            }
        }
    }

    /// Set a kinematic-velocity body's velocity.
    pub fn set_velocity(&mut self, entity: Entity, velocity: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_linvel(Vector::new(velocity.x, velocity.y), true);
            }
        }
    }

    /// True if the entity's character controller was grounded after the last
    /// [`move_characters`](Self::move_characters) call.
    #[must_use]
    pub fn grounded(&self, entity: Entity) -> bool {
        self.grounded.get(&entity).copied().unwrap_or(false)
    }

    /// Cast a ray and return the closest hit, if any.
    #[must_use]
    pub fn cast_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_toi: f32,
        filter: &RayFilter,
    ) -> Option<RayHit> {
        let mut query_filter = QueryFilter::default();
        if filter.exclude_sensors {
            query_filter.flags |= QueryFilterFlags::EXCLUDE_SENSORS;
        }
        let predicate = |handle, _: &_| filter.exclude_entity != self.entity_for_collider(handle);
        query_filter = query_filter.predicate(&predicate);
        let query = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            query_filter,
        );
        let ray = r2::geometry::Ray::new(
            Vector::new(origin.x, origin.y),
            Vector::new(direction.x, direction.y),
        );
        query
            .cast_ray_and_get_normal(&ray, max_toi, filter.solid)
            .and_then(|(handle, hit)| {
                let p = ray.point_at(hit.time_of_impact);
                Some(RayHit {
                    entity: self.entity_for_collider(handle)?,
                    point: Vec3::new(p.x, p.y, 0.0),
                    normal: Vec3::new(hit.normal.x, hit.normal.y, 0.0),
                    distance: hit.time_of_impact,
                })
            })
    }

    /// Shape-cast: sweep a shape from `origin` along `direction` and report
    /// the first impact.
    #[must_use]
    pub fn cast_shape(
        &self,
        shape: Shape2D,
        origin: Vec3,
        direction: Vec3,
        max_toi: f32,
        filter: &RayFilter,
    ) -> Option<RayHit> {
        let mut query_filter = QueryFilter::default();
        if filter.exclude_sensors {
            query_filter.flags |= QueryFilterFlags::EXCLUDE_SENSORS;
        }
        let query = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            query_filter,
        );
        let shape: r2::geometry::SharedShape = match shape {
            Shape2D::Ball { radius } => r2::geometry::SharedShape::ball(radius),
            Shape2D::Cuboid { extents } => {
                r2::geometry::SharedShape::cuboid(extents.x / 2.0, extents.y / 2.0)
            }
            Shape2D::Capsule {
                half_height,
                radius,
            } => r2::geometry::SharedShape::capsule_y(half_height, radius),
        };
        let pos = Pose::from_translation(Vector::new(origin.x, origin.y));
        let vel = Vector::new(direction.x, direction.y);
        query
            .cast_shape(
                &pos,
                vel,
                &*shape,
                r2::parry::query::ShapeCastOptions {
                    max_time_of_impact: max_toi,
                    target_distance: 0.0,
                    stop_at_penetration: false,
                    compute_impact_geometry_on_penetration: true,
                },
            )
            .and_then(|(handle, toi)| {
                Some(RayHit {
                    entity: self.entity_for_collider(handle)?,
                    point: Vec3::new(toi.witness1.x, toi.witness1.y, 0.0),
                    normal: Vec3::new(toi.normal1.x, toi.normal1.y, 0.0),
                    distance: toi.time_of_impact,
                })
            })
    }
}

fn controller_from_config(config: &CharacterController) -> KinematicCharacterController {
    KinematicCharacterController {
        up: Vector::Y,
        offset: CharacterLength::Absolute(config.offset),
        slide: config.slide,
        autostep: config.autostep_height.map(|h| CharacterAutostep {
            max_height: CharacterLength::Absolute(h),
            min_width: CharacterLength::Absolute(0.1),
            include_dynamic_bodies: true,
        }),
        max_slope_climb_angle: config.max_slope_climb_angle,
        min_slope_slide_angle: config.min_slope_slide_angle,
        snap_to_ground: config.snap_to_ground.map(CharacterLength::Absolute),
        normal_nudge_factor: 0.01,
    }
}
