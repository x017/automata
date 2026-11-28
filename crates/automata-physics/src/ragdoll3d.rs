//! Ragdolls in the 3D world: when an entity's [`Ragdoll`] goes limp, a
//! capsule body per big bone (from its [`RagdollLayout`]) is built where
//! the animation has the bones, joined to its parent, and simulated; the
//! bones' places go back to the scene as a [`RagdollPose`] after every
//! step. Switched off, the bodies go, the entity moves to where the hips
//! came down, and the animation blends back from the last pose.

use std::sync::Arc;

use automata_scene::ragdoll::{RagdollBone, RagdollJointKind};
use automata_scene::{
    Animator, Entity, GlobalTransform, MeshVisual, Parent, Ragdoll, RagdollLayout, RagdollPose,
    SceneWorld, Transform,
};
use glam::{Quat, Vec3};
use r3::dynamics::{
    GenericJoint, JointAxesMask, JointAxis, RevoluteJointBuilder, RigidBodyBuilder,
    RigidBodyHandle, SphericalJointBuilder,
};
use r3::geometry::{ColliderBuilder, Group, InteractionGroups, InteractionTestMode, SharedShape};
use r3::math::Pose;
use rapier3d as r3;

use super::PhysicsWorld3D;
use crate::components::CharacterVelocity;
use crate::world::{entity_to_user_data, RayFilter};

/// The collision group every ragdoll body is in: ragdolls don't collide
/// with themselves or each other (their capsules overlap at the joints),
/// only with everything else.
const RAGDOLL_GROUP: Group = Group::GROUP_32;

/// One limp entity's bodies.
#[derive(Debug)]
pub(super) struct RagdollBodies {
    layout: Arc<RagdollLayout>,
    /// One body per layout body.
    bodies: Vec<RigidBodyHandle>,
    /// Each body's rotation relative to its parent when it went limp:
    /// what stiff joints hold.
    held: Vec<Quat>,
    /// A rough moment of inertia per body (kg·m²), for the joint torques.
    inertia: Vec<f32>,
    stiffness: f32,
    blend_time: f32,
    /// How high the entity stood above the ground when it went limp: it
    /// stands that high again where it comes down (models whose origin
    /// isn't at their feet).
    stand_height: f32,
    /// Seconds every body has been all but still.
    still: f32,
    /// Seconds the body as a whole has hardly moved (limbs may twitch).
    calm: f32,
    /// Calm long enough that the bodies are damped hard, so limbs pinned
    /// between the floor and a joint limit stop twitching.
    settled: bool,
}

/// Solver passes a ragdoll's bodies get on top of the world's.
const EXTRA_ITERATIONS: usize = 12;
/// How long a ragdoll must be calm before it's damped hard (seconds).
const SETTLE_AFTER: f32 = 0.5;
/// Mass-weighted mean speed (m/s) below which a ragdoll counts as calm.
const CALM_SPEED: f32 = 0.25;
/// Damping while falling: a little air, and joints that rub.
const FREE_DAMPING: (f32, f32) = (0.05, 0.6);
/// Damping once settled.
const SETTLED_DAMPING: (f32, f32) = (2.0, 8.0);

/// How long a ragdoll must lie still to count as resting (seconds).
const REST_AFTER: f32 = 0.3;

/// The fastest a ragdoll body may move (m/s) or spin (rad/s): a body
/// started inside something is pushed out, not fired off.
const MAX_SPEED: f32 = 30.0;
const MAX_SPIN: f32 = 120.0;

impl PhysicsWorld3D {
    /// Builds and removes ragdoll bodies to match the scene's [`Ragdoll`]s
    /// and hands them their pushes. Runs in
    /// [`sync_from_scene`](Self::sync_from_scene).
    pub(super) fn sync_ragdolls(&mut self, scene: &mut SceneWorld) {
        let wanted: Vec<(Entity, Ragdoll)> = scene
            .query::<(Entity, &Ragdoll)>()
            .iter()
            .map(|(e, r)| (e, r.sanitized()))
            .collect();
        let gone: Vec<Entity> = self
            .ragdolls
            .keys()
            .filter(|e| !wanted.iter().any(|(w, r)| w == *e && r.active))
            .copied()
            .collect();
        for entity in gone {
            self.release_ragdoll(scene, entity);
        }
        for (entity, ragdoll) in wanted {
            match self.ragdolls.get_mut(&entity) {
                Some(built) => {
                    built.stiffness = ragdoll.stiffness;
                    built.blend_time = ragdoll.blend_time;
                }
                None if ragdoll.active => self.build_ragdoll(scene, entity, &ragdoll),
                None => {}
            }
        }
        self.push_ragdolls(scene);
    }

    /// The layout for `entity`'s model (worked out once per model).
    fn ragdoll_layout(&mut self, scene: &SceneWorld, entity: Entity) -> Option<Arc<RagdollLayout>> {
        let model = scene.get::<&MeshVisual>(entity).ok()?.model.clone()?;
        let key = Arc::as_ptr(&model) as usize;
        if let Some(layout) = self.ragdoll_layouts.get(&key) {
            return layout.clone();
        }
        let layout = RagdollLayout::of(&model).map(Arc::new);
        if layout.is_none() {
            tracing::warn!("ragdoll: the model has no rig or skin to build a ragdoll from");
        }
        self.ragdoll_layouts.insert(key, layout.clone());
        layout
    }

    #[allow(clippy::too_many_lines)]
    fn build_ragdoll(&mut self, scene: &mut SceneWorld, entity: Entity, ragdoll: &Ragdoll) {
        let Some(layout) = self.ragdoll_layout(scene, entity) else {
            return;
        };
        // Where the bones are now: the animated pose, in the world.
        let (globals, import) = {
            let Ok(animator) = scene.get::<&Animator>(entity) else {
                return;
            };
            let rig = animator.rig();
            (rig.globals(animator.pose()), rig.import)
        };
        let place = scene
            .get::<&GlobalTransform>(entity)
            .map_or(glam::Mat4::IDENTITY, |g| g.0);
        let (scale, _, _) = place.to_scale_rotation_translation();
        let size = (scale.x.abs() + scale.y.abs() + scale.z.abs()) / 3.0;
        let velocity = scene
            .get::<&CharacterVelocity>(entity)
            .map_or(Vec3::ZERO, |v| v.velocity);
        let filter = RayFilter {
            exclude_sensors: true,
            exclude_entity: Some(entity),
            ..RayFilter::default()
        };
        let origin = place.w_axis.truncate();
        let stand_height = self
            .cast_ray(origin + Vec3::Y * 0.1, Vec3::NEG_Y, 5.0, &filter)
            .map_or(0.0, |hit| origin.y - hit.point.y);
        let groups = InteractionGroups::new(
            RAGDOLL_GROUP,
            Group::ALL.difference(RAGDOLL_GROUP),
            InteractionTestMode::And,
        );
        let mut bodies = Vec::with_capacity(layout.bodies.len());
        let mut inertia = Vec::with_capacity(layout.bodies.len());
        let mut rotations = Vec::with_capacity(layout.bodies.len());
        for body in &layout.bodies {
            let world = place * import * globals.get(body.node).copied().unwrap_or_default();
            let (_, rotation, position) = world.to_scale_rotation_translation();
            let rotation = rotation.normalize();
            let handle = self.bodies.insert(
                RigidBodyBuilder::dynamic()
                    .translation(position)
                    .rotation(rotation.to_scaled_axis())
                    .linvel(velocity)
                    .linear_damping(FREE_DAMPING.0)
                    .angular_damping(FREE_DAMPING.1)
                    .ccd_enabled(true)
                    // Chains of limited joints pressed into the floor need
                    // more solver passes than a lone body to lie still.
                    .additional_solver_iterations(EXTRA_ITERATIONS)
                    .build(),
            );
            let (a, b, r) = (body.a * size, body.b * size, body.radius * size);
            let shape = if a.distance(b) < 1e-4 {
                SharedShape::ball(r)
            } else {
                SharedShape::capsule(a, b, r)
            };
            let mass = ragdoll.mass * body.share;
            let collider = ColliderBuilder::new(shape)
                .mass(mass)
                .friction(0.8)
                .restitution(0.0)
                .collision_groups(groups)
                .user_data(entity_to_user_data(entity));
            self.colliders
                .insert_with_parent(collider.build(), handle, &mut self.bodies);
            let length = a.distance(b) + 2.0 * r;
            inertia.push((mass * length * length / 12.0).max(1e-4));
            bodies.push(handle);
            rotations.push(rotation);
        }
        let mut held = Vec::with_capacity(layout.bodies.len());
        for (i, body) in layout.bodies.iter().enumerate() {
            let (Some(parent), Some(joint)) = (body.parent, body.joint) else {
                held.push(Quat::IDENTITY);
                continue;
            };
            let mut data: GenericJoint = match joint.kind {
                RagdollJointKind::Hinge { .. } => RevoluteJointBuilder::new(Vec3::X).build().into(),
                RagdollJointKind::Ball { .. } => SphericalJointBuilder::new().build().into(),
            };
            data.set_local_frame1(Pose::from_parts(
                joint.anchor_parent * size,
                joint.frame_parent,
            ));
            data.set_local_frame2(Pose::from_parts(
                joint.anchor_child * size,
                joint.frame_child,
            ));
            match joint.kind {
                RagdollJointKind::Hinge { min, max } => {
                    data.set_limits(JointAxis::AngX, [min, max]);
                }
                RagdollJointKind::Ball { swing, twist } => {
                    data.set_limits(JointAxis::AngX, [-twist, twist]);
                    // Swing is one cone round the bone (the angle between
                    // the two frames' X axes), not two separate angles:
                    // those fight each other once a limb lies bent far
                    // over, and the limb never stops twitching.
                    data.coupled_axes = JointAxesMask::ANG_Y | JointAxesMask::ANG_Z;
                    data.set_limits(JointAxis::AngY, [-swing, swing]);
                }
            }
            data.set_contacts_enabled(false);
            self.impulse_joints
                .insert(bodies[parent], bodies[i], data, true);
            held.push((rotations[parent].inverse() * rotations[i]).normalize());
        }
        // The entity's own collider (a character's capsule) would shove
        // the bodies apart: it sleeps while the ragdoll runs.
        self.set_own_collider(entity, false);
        if scene.get::<&RagdollPose>(entity).is_err() {
            let _ = scene.insert_one(entity, RagdollPose::default());
        }
        self.ragdolls.insert(
            entity,
            RagdollBodies {
                layout,
                bodies,
                held,
                inertia,
                stiffness: ragdoll.stiffness,
                blend_time: ragdoll.blend_time,
                stand_height,
                still: 0.0,
                calm: 0.0,
                settled: false,
            },
        );
        self.report_ragdolls(scene);
    }

    /// Turns the entity's own collider on or off.
    fn set_own_collider(&mut self, entity: Entity, on: bool) {
        if let Some(collider) = self
            .entity_colliders
            .get(&entity)
            .and_then(|h| self.colliders.get_mut(*h))
        {
            collider.set_enabled(on);
        }
    }

    /// Takes `entity`'s ragdoll bodies away: the entity stands where the
    /// hips came down, and the animation blends back from the last pose.
    fn release_ragdoll(&mut self, scene: &mut SceneWorld, entity: Entity) {
        let Some(built) = self.ragdolls.remove(&entity) else {
            return;
        };
        let hips = built
            .bodies
            .first()
            .and_then(|h| self.bodies.get(*h))
            .map(r3::dynamics::RigidBody::translation);
        for handle in built.bodies {
            self.bodies.remove(
                handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.soft_bodies,
                true,
            );
        }
        if !scene.contains(entity) {
            return;
        }
        self.set_own_collider(entity, true);
        if let Ok(mut state) = scene.get::<&mut RagdollPose>(entity) {
            state.simulating = false;
            state.resting = false;
            state.blend_time = built.blend_time;
            state.pushes.clear();
        }
        // Stand up under the hips (on whatever is below them).
        let Some(hips) = hips else { return };
        if scene.get::<&Parent>(entity).is_ok() {
            return;
        }
        let filter = RayFilter {
            exclude_sensors: true,
            exclude_entity: Some(entity),
            ..RayFilter::default()
        };
        let Ok(mut transform) = scene.get::<&mut Transform>(entity) else {
            return;
        };
        let floor = self
            .cast_ray(hips, Vec3::NEG_Y, 50.0, &filter)
            .map_or(transform.position.y, |hit| hit.point.y + built.stand_height);
        transform.position = Vec3::new(hips.x, floor, hips.z);
        let to = transform.position;
        drop(transform);
        if let Some(body) = self
            .entity_bodies
            .get(&entity)
            .and_then(|h| self.bodies.get_mut(*h))
        {
            body.set_translation(to, true);
        }
    }

    /// Hands queued pushes to the bodies (pushes on a model that isn't
    /// limp are dropped).
    fn push_ragdolls(&mut self, scene: &mut SceneWorld) {
        for (entity, state) in scene.query_mut::<(Entity, &mut RagdollPose)>() {
            if state.pushes.is_empty() {
                continue;
            }
            let pushes = std::mem::take(&mut state.pushes);
            let Some(built) = self.ragdolls.get_mut(&entity) else {
                continue;
            };
            // A push wakes it: no settling until it's calm again.
            built.calm = 0.0;
            for push in pushes {
                let target = push
                    .bone
                    .as_deref()
                    .map(|name| built.layout.body_named(name));
                match target {
                    Some(Some(i)) => {
                        if let Some(body) = self.bodies.get_mut(built.bodies[i]) {
                            body.apply_impulse(push.impulse, true);
                        }
                    }
                    Some(None) => {
                        tracing::warn!(
                            "ragdoll push: no simulated bone called {:?}",
                            push.bone.unwrap_or_default()
                        );
                    }
                    None => {
                        for (i, handle) in built.bodies.iter().enumerate() {
                            if let Some(body) = self.bodies.get_mut(*handle) {
                                body.apply_impulse(
                                    push.impulse * built.layout.bodies[i].share,
                                    true,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Joint torques before a step: every joint rubs a little (a body
    /// doesn't flop forever), and stiff ragdolls pull each body back to
    /// the turn it had against its parent when it went limp.
    pub(super) fn drive_ragdolls(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for built in self.ragdolls.values_mut() {
            let mut slow = true;
            let mut mean_speed = 0.0;
            for (handle, body) in built.bodies.iter().zip(&built.layout.bodies) {
                if let Some(rb) = self.bodies.get_mut(*handle) {
                    let (v, w) = (rb.linvel(), rb.angvel());
                    slow &= rb.is_sleeping() || (v.length() < 0.3 && w.length() < 1.0);
                    mean_speed += body.share * v.length();
                    if v.length() > MAX_SPEED {
                        rb.set_linvel(v.clamp_length_max(MAX_SPEED), true);
                    }
                    if w.length() > MAX_SPIN {
                        rb.set_angvel(w.clamp_length_max(MAX_SPIN), true);
                    }
                }
            }
            built.still = if slow { built.still + dt } else { 0.0 };
            built.calm = if mean_speed < CALM_SPEED {
                built.calm + dt
            } else {
                0.0
            };
            let settled = built.calm >= SETTLE_AFTER;
            if settled != built.settled {
                built.settled = settled;
                let (linear, angular) = if settled {
                    SETTLED_DAMPING
                } else {
                    FREE_DAMPING
                };
                for handle in &built.bodies {
                    if let Some(rb) = self.bodies.get_mut(*handle) {
                        rb.set_linear_damping(linear);
                        rb.set_angular_damping(angular);
                    }
                }
            }
            for (i, body) in built.layout.bodies.iter().enumerate() {
                let Some(parent) = body.parent else { continue };
                let (Some(child_rb), Some(parent_rb)) = (
                    self.bodies.get(built.bodies[i]),
                    self.bodies.get(built.bodies[parent]),
                ) else {
                    continue;
                };
                let (rc, rp) = (*child_rb.rotation(), *parent_rb.rotation());
                let spin = child_rb.angvel() - parent_rb.angvel();
                let inertia = built.inertia[i];
                // The strongest pull that stays stable at this step.
                let k = built.stiffness * inertia * (0.4 / dt).powi(2);
                let damping = 2.0 * inertia + 1.4 * (k * inertia).sqrt();
                let want = rp * built.held[i];
                let mut error = (want * rc.inverse()).normalize();
                if error.w < 0.0 {
                    error = -error;
                }
                let (axis, angle) = error.to_axis_angle();
                let pull = if angle.is_finite() && k > 0.0 {
                    axis * angle * k
                } else {
                    Vec3::ZERO
                };
                let torque = pull - spin * damping;
                let impulse = torque * dt;
                if !impulse.is_finite() {
                    continue;
                }
                // Rubbing alone never wakes a body up: they may sleep.
                let wake = true;
                if let Some(rb) = self.bodies.get_mut(built.bodies[i]) {
                    rb.apply_torque_impulse(impulse, wake);
                }
                if let Some(rb) = self.bodies.get_mut(built.bodies[parent]) {
                    rb.apply_torque_impulse(-impulse, wake);
                }
            }
        }
    }

    /// Writes every limp entity's bones into its [`RagdollPose`] (after a
    /// step).
    pub(super) fn report_ragdolls(&self, scene: &mut SceneWorld) {
        for (entity, built) in &self.ragdolls {
            let Ok(mut state) = scene.get::<&mut RagdollPose>(*entity) else {
                continue;
            };
            state.simulating = true;
            state.weight = 1.0;
            state.blend_time = built.blend_time;
            state.resting = self.ragdoll_resting(*entity);
            state.bones.clear();
            for (body, handle) in built.layout.bodies.iter().zip(&built.bodies) {
                if let Some(rb) = self.bodies.get(*handle) {
                    state.bones.push(RagdollBone {
                        node: body.node,
                        position: rb.translation(),
                        rotation: *rb.rotation(),
                    });
                }
            }
        }
    }

    /// Whether `entity` is limp right now (its ragdoll simulating).
    #[must_use]
    pub fn is_ragdoll(&self, entity: Entity) -> bool {
        self.ragdolls.contains_key(&entity)
    }

    /// Where a limp `entity`'s hips (its ragdoll's root body) are, if it's
    /// limp: cameras follow this.
    #[must_use]
    pub fn ragdoll_root(&self, entity: Entity) -> Option<Vec3> {
        let built = self.ragdolls.get(&entity)?;
        self.bodies
            .get(*built.bodies.first()?)
            .map(r3::dynamics::RigidBody::translation)
    }

    /// How many bodies `entity`'s ragdoll has while limp (0 otherwise).
    #[must_use]
    pub fn ragdoll_body_count(&self, entity: Entity) -> usize {
        self.ragdolls.get(&entity).map_or(0, |b| b.bodies.len())
    }

    /// Whether `entity` is limp and its bodies have come to rest.
    #[must_use]
    pub fn ragdoll_resting(&self, entity: Entity) -> bool {
        self.ragdolls
            .get(&entity)
            .is_some_and(|built| built.still >= REST_AFTER)
    }
}
