//! Water physics: buoyancy and drag for dynamic bodies in [`Water`].
//!
//! Every fixed step, before Rapier integrates, each awake dynamic body that
//! reaches into a water surface gets:
//!
//! - **buoyancy**: an upward impulse of `water density × g × submerged
//!   volume`, applied where the body is submerged, so bodies settle at the
//!   depth their density gives (half under at half the water's density),
//!   and tilt to follow the waves;
//! - **drag**: velocity and spin lost in proportion to how submerged it is;
//! - a **dent** in the ripple field (see [`crate::ripples`]) while it
//!   crosses the surface, so it sends out rings and leaves a wake.
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

use automata_renderer::{RippleMap, Water, WaterSurface};
use automata_scene::{GlobalTransform, SceneWorld, Transform};
use glam::{Quat, Vec3};
use rapier3d::dynamics::RigidBodyHandle;
use rapier3d::math::Vector;

use super::PhysicsWorld3D;
use crate::ripples::Dent;
use crate::world::WaterSplash;

/// Bodies whose bounding box is smaller than this (metres, largest side)
/// are sampled at their centre only.
const SMALL_BODY: f32 = 0.6;

/// Velocity lost per second, fully submerged, at `Water::drag` 1. Enough
/// that a floating crate settles after two or three bobs, like in water.
const DRAG_RATE: f32 = 4.0;

/// How fast (m/s) a body must hit the water to splash.
const SPLASH_SPEED: f32 = 1.5;

/// Wind (`Water::wind`) above which spray blows off the crests.
const SPRAY_WIND: f32 = 0.5;
/// Spray bursts per second at full wind.
const SPRAY_RATE: f32 = 45.0;
/// How far from the ripple field's centre spray appears (m).
const SPRAY_REACH: f32 = 25.0;

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
        self.winds.clear();
        let time = self.water_time();
        // The scene's wind blows over every water surface too: spray goes
        // with whichever is stronger, the water's own or the scene's.
        let scene_wind = (&mut scene.query::<&automata_renderer::Environment>())
            .into_iter()
            .next()
            .map(|env| (env.wind.clamp(0.0, 1.0), env.wind_direction.to_radians()));
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
            let own = (
                water.wind.clamp(0.0, 1.0),
                water.wave_direction.to_radians(),
            );
            self.winds.push(match scene_wind {
                Some(scene) if scene.0 > own.0 => scene,
                _ => own,
            });
        }
    }

    /// Awake bodies, and characters (moved by teleporting, they may not
    /// count as awake). Forgets characters that are gone.
    fn bodies_in_motion(&mut self) -> Vec<RigidBodyHandle> {
        let mut handles: Vec<RigidBodyHandle> = self.islands.active_bodies().collect();
        for handle in self.character_sinking.keys() {
            if !handles.contains(handle) {
                handles.push(*handle);
            }
        }
        let bodies = &self.bodies;
        self.character_sinking.retain(|h, _| bodies.contains(*h));
        handles
    }

    /// Applies buoyancy and drag for one step of `dt` seconds.
    pub(super) fn apply_water(&mut self, dt: f32) {
        let gravity = -self.gravity.y;
        if self.water.is_empty() || gravity <= 0.0 {
            return;
        }
        let handles = self.bodies_in_motion();
        let water = &self.water;
        // Each awake body's dent in the ripples this step (None: it's not
        // crossing the surface), and how fast it's moving down into it.
        let mut dents = Vec::new();
        for handle in handles {
            dents.push((handle, None, 0.0, 0.0));
            let Some(body) = self.bodies.get_mut(handle) else {
                continue;
            };
            // Kinematic bodies (script-driven boats, characters wading)
            // dent the ripples too, but the water doesn't push them.
            let dynamic = body.is_dynamic();
            if !dynamic && !body.is_kinematic() {
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
            if dynamic && density <= 0.0 {
                continue;
            }
            let (world_center, half, [ax, ay, az]) = world_box(collider);
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
            // Crossing the surface: it pushes the ripples about.
            let level = if calm {
                surface.level()
            } else {
                surface.height_unchecked(world_center.x, world_center.z)
            };
            if lo.y < level && hi.y > level {
                if let Some(last) = dents.last_mut() {
                    last.1 = Some(Dent::of_body(world_center, world_half, level));
                    // Kinematic bodies (characters) move to where they were
                    // told to be next; their velocity is that move.
                    last.2 = match self.character_sinking.get(&handle) {
                        Some(sinking) => *sinking,
                        None => -body.linvel().y,
                    };
                    last.3 = level;
                }
            }
            if !dynamic {
                continue;
            }
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
        self.dig_dents(dents);
    }

    /// Moves each body's ripple dent to where it is now (`None`: out of
    /// the water). Bodies that just hit the water faster than
    /// `SPLASH_SPEED` (`sinking`, m/s downward) splash too.
    fn dig_dents(&mut self, dents: Vec<(RigidBodyHandle, Option<Dent>, f32, f32)>) {
        for (handle, dent, sinking, level) in dents {
            let old = match dent {
                Some(dent) => self.dents.insert(handle, dent),
                None => self.dents.remove(&handle),
            };
            // Hitting the water hard splashes, on top of the water pushed
            // aside: harder and bigger, bigger splash.
            if let (None, Some(dent)) = (old, dent) {
                if sinking > SPLASH_SPEED {
                    let size = (dent.radius / 0.7).powi(2);
                    let strength = (sinking - SPLASH_SPEED) * 0.3 * size;
                    let at = Vec3::new(dent.center.x, 0.0, dent.center.y);
                    self.ripples.splash(at, strength.min(10.0));
                    self.splashes.push(WaterSplash {
                        position: Vec3::new(dent.center.x, level, dent.center.y),
                        speed: sinking,
                        radius: dent.radius,
                    });
                }
            }
            if old != dent {
                if let Some(old) = old {
                    self.ripples.fill(old);
                }
                if let Some(dent) = dent {
                    self.ripples.dig(dent);
                }
            }
        }
    }

    /// Advances the ripples by one step of `dt` seconds (after
    /// [`Self::apply_water`] dug this step's dents).
    pub(super) fn step_ripples(&mut self, dt: f32) {
        // Bodies that were removed take their dents with them.
        let bodies = &self.bodies;
        let ripples = &mut self.ripples;
        self.dents.retain(|handle, dent| {
            let alive = bodies.contains(*handle);
            if !alive {
                ripples.fill(*dent);
            }
            alive
        });
        if self.water.is_empty() {
            // No water, no ripples (pending splashes have nowhere to go).
            self.ripples.calm();
            return;
        }
        self.ripples.step(dt);
    }

    /// This step's water particles: a burst of droplets for each body that
    /// hit the water (sized by how hard), and spray off the crests of
    /// windy water around the ripple field. Returns the splashes.
    pub(super) fn water_effects(&mut self, scene: &mut SceneWorld, dt: f32) -> Vec<WaterSplash> {
        let splashes = std::mem::take(&mut self.splashes);
        if !self.water_particles {
            return splashes;
        }
        for s in &splashes {
            let emitter = automata_scene::splash_emitter(s.speed, s.radius, SPLASH_SPEED);
            automata_scene::spawn_burst(scene, emitter, s.position, None, None);
        }
        self.crest_spray(scene, dt);
        splashes
    }

    /// Spray off the wave crests of water with wind over `SPRAY_WIND`:
    /// a few bursts a second at random crests within `SPRAY_REACH` of the
    /// ripple field's centre (what the camera sees).
    fn crest_spray(&mut self, scene: &mut SceneWorld, dt: f32) {
        let center = self.ripples.center();
        for (surface, &(wind, direction)) in self.water.iter().zip(&self.winds) {
            if wind <= SPRAY_WIND || surface.max_height() <= surface.level() {
                continue;
            }
            let strength = (wind - SPRAY_WIND) / (1.0 - SPRAY_WIND);
            self.spray_carry += SPRAY_RATE * strength * dt;
            let crest = surface.level() + (surface.max_height() - surface.level()) * 0.4;
            let (dz, dx) = direction.sin_cos();
            let downwind = Vec3::new(dx, 0.0, dz);
            while self.spray_carry >= 1.0 {
                self.spray_carry -= 1.0;
                // A few tries at landing on a crest.
                for _ in 0..6 {
                    let angle = spray_random(&mut self.spray_rng) * std::f32::consts::TAU;
                    let r = SPRAY_REACH * spray_random(&mut self.spray_rng).sqrt();
                    let (x, z) = (center.x + angle.cos() * r, center.y + angle.sin() * r);
                    if !surface.contains(x, z) {
                        continue;
                    }
                    let h = surface.height_unchecked(x, z);
                    if h < crest {
                        continue;
                    }
                    let up = (downwind + Vec3::Y * (0.5 + 0.3 * spray_random(&mut self.spray_rng)))
                        .normalize();
                    let emitter = automata_scene::spray_emitter(wind);
                    automata_scene::spawn_burst(scene, emitter, Vec3::new(x, h, z), Some(up), None);
                    break;
                }
            }
        }
    }

    /// Turns the particles water makes (splash droplets, crest spray) on
    /// or off; on by default. Splashes still ripple either way.
    pub fn set_water_particles(&mut self, on: bool) {
        self.water_particles = on;
    }

    /// Makes a splash in the water at `position` (its X and Z): rings that
    /// spread from it, and foam. A `strength` of 1 is about a crate dropped
    /// from a couple of metres (0 to 10). Shows from the next step.
    pub fn splash(&mut self, position: Vec3, strength: f32) {
        self.ripples.splash(position, strength);
    }

    /// Keeps the ripple simulation (64 m across) around what a camera at
    /// `eye` looking at `target` sees. Call it each frame with the camera
    /// the scene is drawn from; ripples beyond the field aren't simulated.
    pub fn set_ripple_view(&mut self, eye: Vec3, target: Vec3) {
        self.ripples.follow(eye, target);
    }

    /// The ripples to draw (pass them to `DrawList::set_ripples`), or
    /// `None` while the water is undisturbed. Cheap to call every frame:
    /// a new snapshot is packed only after the ripples changed.
    pub fn ripples(&mut self) -> Option<RippleMap> {
        self.ripples.snapshot()
    }

    /// The ripple height (m, above or below the waves) at world `(x, z)`;
    /// 0 where the water is undisturbed or beyond the simulated field.
    #[must_use]
    pub fn ripple_height(&self, x: f32, z: f32) -> f32 {
        self.ripples.height_at(x, z)
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

/// A collider's box in the world: its centre, half extents along its own
/// axes, and those axes.
fn world_box(collider: &rapier3d::geometry::Collider) -> (Vec3, Vec3, [Vec3; 3]) {
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
    (
        origin + rot * center,
        half,
        [rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z],
    )
}

/// A random number in [0, 1) for the spray (xorshift64* on `state`).
fn spray_random(state: &mut u64) -> f32 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    #[allow(clippy::cast_precision_loss)] // 24 bits
    let v = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32;
    v / 16_777_216.0
}
