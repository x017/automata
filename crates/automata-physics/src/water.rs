//! Water physics: buoyancy and drag for dynamic bodies in [`Water`].
//!
//! Every fixed step, before Rapier integrates, each awake dynamic body that
//! reaches into a water surface gets:
//!
//! - **buoyancy**: an upward impulse of `water density × g × submerged
//!   volume`, applied where the body is submerged, so bodies settle at the
//!   depth their density gives (half under at half the water's density),
//!   and tilt to follow the waves;
//! - **drag**: velocity and spin lost in proportion to how submerged it is.
//!
//! The surface comes from [`WaterSurface`], the same waves the GPU draws.
//!
//! # Cost
//!
//! Nothing at all without water. With water, sleeping bodies are skipped
//! (Rapier's active set), and the rest are rejected by two comparisons
//! against the surface's bounds and highest crest before any wave maths.
//! A body in the water is sampled at 1 point if small, else at the 8
//! octants of its bounding box (what makes long bodies pitch and roll on
//! the swell), which share 4 surface lookups. A lookup is two evaluations
//! of four waves; on calm water, none. Bodies that settle on calm water
//! fall asleep and cost nothing; on waves they stay awake to ride them.
//!
//! Measured (release, one core): about 0.25 µs per floating body per step
//! on waves (a mix of crates and buoys), so a thousand of them add ~0.25 ms
//! to a step in which Rapier spends ~0.6 ms simulating them. The CPU wave
//! sampler evaluates all four waves at once in SIMD lanes with a branchless
//! `sin_cos` (see `automata_renderer::WaterSurface`).

use automata_renderer::{Water, WaterSurface};
use automata_scene::{GlobalTransform, SceneWorld, Transform};
use glam::{Quat, Vec3};
use rapier3d::math::Vector;

use super::PhysicsWorld3D;

/// Bodies whose bounding box is smaller than this (metres, largest side)
/// are sampled at their centre only.
const SMALL_BODY: f32 = 0.6;

/// Velocity lost per second, fully submerged, at `Water::drag` 1. Enough
/// that a floating crate settles after two or three bobs, like in water.
const DRAG_RATE: f32 = 4.0;

/// A collider's bounding box in the world: centre, rotated axes, half size.
struct Bounds {
    center: Vec3,
    axes: [Vec3; 3],
    half: Vec3,
}

/// What the water does to one body, per unit of its weight: the upward
/// push, its torque about the centre of mass, and the submerged fraction.
struct Lift {
    up: f32,
    torque: Vec3,
    submerged: f32,
    /// Sampled at one point (small body): no torque.
    single: bool,
}

impl Bounds {
    /// Samples `surface` over the box. A small body is one sample; a big
    /// one is its 8 octants, in 4 columns sharing a height lookup each
    /// (the surface only depends on x and z).
    fn lift(&self, surface: &WaterSurface, com: Vec3) -> Lift {
        let [ax, ay, az] = self.axes;
        let single = self.half.max_element() * 2.0 < SMALL_BODY;
        let cell = if single { self.half } else { self.half * 0.5 };
        // Half a cell's vertical extent in the world.
        let reach = (ax.y.abs() * cell.x + ay.y.abs() * cell.y + az.y.abs() * cell.z).max(1e-3);
        let (columns, layers, weight) = if single { (1, 1, 1.0) } else { (4, 2, 0.125) };
        let calm = surface.max_height() <= surface.level();
        let mut lift = Lift {
            up: 0.0,
            torque: Vec3::ZERO,
            submerged: 0.0,
            single,
        };
        for column in 0..columns {
            let (mid, layer_offsets) = if single {
                (self.center, [Vec3::ZERO; 2])
            } else {
                let sx = if column & 1 == 0 { -cell.x } else { cell.x };
                let sz = if column & 2 == 0 { -cell.z } else { cell.z };
                let dy = ay * cell.y;
                (self.center + ax * sx + az * sz, [-dy, dy])
            };
            let level = if calm {
                surface.level()
            } else {
                surface.height_unchecked(mid.x, mid.z)
            };
            for offset in &layer_offsets[..layers] {
                let point = mid + *offset;
                let depth = ((level - (point.y - reach)) / (2.0 * reach)).clamp(0.0, 1.0);
                if depth > 0.0 {
                    let push = weight * depth;
                    lift.up += push;
                    // (point - com) × (0, push, 0)
                    let arm = point - com;
                    lift.torque += Vec3::new(-arm.z * push, 0.0, arm.x * push);
                    lift.submerged += push;
                }
            }
        }
        lift
    }
}

impl PhysicsWorld3D {
    /// Reads the scene's water surfaces at the current simulation time.
    pub(super) fn collect_water(&mut self, scene: &SceneWorld) {
        self.water.clear();
        let time = self.water_time();
        for (water, global, local) in
            &mut scene.query::<(&Water, Option<&GlobalTransform>, Option<&Transform>)>()
        {
            // Before the first transform propagation, the local transform
            // is the world one (water is rarely parented).
            let model = global.map_or_else(
                || local.map_or(glam::Mat4::IDENTITY, Transform::to_matrix),
                GlobalTransform::matrix,
            );
            self.water.push(WaterSurface::new(water, model, time));
        }
    }

    /// Applies buoyancy and drag for one step of `dt` seconds.
    pub(super) fn apply_water(&mut self, dt: f32) {
        let gravity = -self.gravity.y;
        if self.water.is_empty() || gravity <= 0.0 {
            return;
        }
        let water = &self.water;
        for handle in self.islands.active_bodies() {
            let Some(body) = self.bodies.get_mut(handle) else {
                continue;
            };
            if !body.is_dynamic() {
                continue;
            }
            let Some(collider) = body
                .colliders()
                .first()
                .and_then(|c| self.colliders.get(*c))
            else {
                continue;
            };
            if collider.is_sensor() {
                continue;
            }
            // Density gives the volume from Rapier's cached mass (shape
            // volumes are recomputed per call for hulls and meshes).
            let density = collider.density();
            if density <= 0.0 {
                continue;
            }
            let pose = collider.position();
            let rot = Quat::from_xyzw(
                pose.rotation.x,
                pose.rotation.y,
                pose.rotation.z,
                pose.rotation.w,
            );
            let origin = Vec3::new(pose.translation.x, pose.translation.y, pose.translation.z);
            let local = collider.shape().compute_local_aabb();
            let (mid, ext) = (local.center(), local.half_extents());
            let (center, half) = (
                Vec3::new(mid.x, mid.y, mid.z),
                Vec3::new(ext.x, ext.y, ext.z),
            );
            let (ax, ay, az) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
            let world_center = origin + rot * center;
            let world_half = ax.abs() * half.x + ay.abs() * half.y + az.abs() * half.z;

            // Cheap rejection: above every crest, or off every rectangle.
            let (lo, hi) = (world_center - world_half, world_center + world_half);
            let Some(surface) = water.iter().find(|s| {
                let (min, max) = s.bounds();
                lo.y < s.max_height()
                    && hi.x >= min.x
                    && lo.x <= max.x
                    && hi.z >= min.y
                    && lo.z <= max.y
            }) else {
                continue;
            };
            // Over the rectangle (rotated ones too)? Tested once per body.
            if !surface.contains(world_center.x, world_center.z) {
                continue;
            }
            let calm = surface.max_height() <= surface.level();
            let com = body.center_of_mass();
            let shape = Bounds {
                center: world_center,
                axes: [ax, ay, az],
                half,
            };
            let lift = shape.lift(surface, Vec3::new(com.x, com.y, com.z));
            let Lift {
                up,
                torque,
                submerged,
                single,
            } = lift;
            // Lift is in units of the water's weight per unit volume.
            let scale = surface.density * gravity * (body.mass() / density) * dt;
            let (up, torque) = (up * scale, torque * scale);
            if submerged > 0.0 {
                // Waves keep what floats on them awake (they'd freeze in
                // mid-swell otherwise); on calm water, settled bodies sleep
                // and cost nothing until something wakes them.
                let wake = !calm;
                body.apply_impulse(Vector::new(0.0, up, 0.0), wake);
                if !single {
                    body.apply_torque_impulse(Vector::new(torque.x, torque.y, torque.z), wake);
                }
                // Implicit damping: stable at any drag and timestep.
                let rate = surface.drag * DRAG_RATE * submerged * dt;
                let linear = 1.0 / (1.0 + rate);
                let angular = 1.0 / (1.0 + 2.0 * rate);
                body.set_linvel(body.linvel() * linear, wake);
                body.set_angvel(body.angvel() * angular, wake);
            }
        }
    }

    /// The clock the water's waves run on: simulated seconds since the
    /// world was created. Pass it to `DrawList::set_time` so the drawn
    /// waves are the ones bodies float on.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // the GPU clock is f32
    pub fn water_time(&self) -> f32 {
        self.sim_time as f32
    }

    /// The water surface height at world `(x, z)` at the last physics step,
    /// or `None` where there's no water. Use it to swim, splash or spawn
    /// things on the surface.
    #[must_use]
    pub fn water_height(&self, x: f32, z: f32) -> Option<f32> {
        self.water.iter().find_map(|s| s.height_at(x, z))
    }

    /// The scene's water surfaces as of the last physics step.
    #[must_use]
    pub fn water_surfaces(&self) -> &[WaterSurface] {
        &self.water
    }
}
