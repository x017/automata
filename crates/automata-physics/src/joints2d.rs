//! Joints in the 2D world: the same [`Joint`] component as in 3D, on the
//! X/Y plane. Hinges always turn around Z (the axis is ignored) and a
//! ball joint is a free pivot; sliders use the axis's X/Y direction.

use std::collections::HashSet;

use automata_scene::{Entity, Name, SceneWorld};
use glam::Vec2;
use r2::dynamics::{
    FixedJointBuilder, GenericJoint, ImpulseJointHandle, IntegrationParameters, JointAxis,
    MotorModel, PrismaticJointBuilder, RevoluteJointBuilder, RigidBodyBuilder, RigidBodyHandle,
    RopeJointBuilder, SpringJointBuilder,
};
use r2::math::{Pose, Rotation};
use rapier2d as r2;

use super::PhysicsWorld2D;
use crate::joint::{Joint, JointKind, JointState};

/// One entity's joint as the world built it.
#[derive(Debug, Clone)]
pub(super) struct JointSlot {
    /// The component it was built from.
    spec: Joint,
    /// The Rapier joint, while attached.
    handle: Option<ImpulseJointHandle>,
    /// The joint's frames in each body (body 1: the connected one or the
    /// world; body 2: the entity's own), fixed when it was built.
    frame1: Pose,
    frame2: Pose,
    /// The rope / spring length it was built with.
    length: f32,
    /// The bodies it joins.
    body1: Option<RigidBodyHandle>,
    body2: Option<RigidBodyHandle>,
    /// Waiting for a body or for the entity it names; retried each sync.
    pending: bool,
    /// Snapped past its break force (stays snapped until rebuilt).
    broken: bool,
    /// Whether it ever held (a joint whose partner later vanished goes
    /// quiet instead of warning).
    ever_attached: bool,
    /// The force at the last step (newtons).
    force: f32,
}

impl PhysicsWorld2D {
    /// The fixed body joints to the world hang from (made on first use).
    fn ground(&mut self) -> RigidBodyHandle {
        if let Some(handle) = self.ground {
            if self.bodies.get(handle).is_some() {
                return handle;
            }
        }
        let handle = self.bodies.insert(RigidBodyBuilder::fixed().build());
        self.ground = Some(handle);
        handle
    }

    /// Builds, adjusts and removes joints to match the scene's [`Joint`]
    /// components. Runs at the end of [`sync_from_scene`](Self::sync_from_scene).
    pub(super) fn sync_joints(&mut self, scene: &SceneWorld) {
        // Only entities with a 2D collider are ours.
        let specs: Vec<(Entity, Joint)> = scene
            .query::<(Entity, &Joint)>()
            .iter()
            .filter(|(e, _)| scene.get::<&crate::components::Collider2D>(*e).is_ok())
            .map(|(e, j)| (e, j.clone()))
            .collect();
        let present: HashSet<Entity> = specs.iter().map(|(e, _)| *e).collect();
        let gone: Vec<Entity> = self
            .joints
            .keys()
            .filter(|e| !present.contains(e))
            .copied()
            .collect();
        for entity in gone {
            if let Some(slot) = self.joints.remove(&entity) {
                if let Some(handle) = slot.handle {
                    self.impulse_joints.remove(handle, true);
                }
            }
            self.joint_warned.remove(&entity);
        }

        for (entity, spec) in specs {
            let Some(mut slot) = self.joints.remove(&entity) else {
                let slot = self.build_joint(scene, entity, spec, false);
                self.joints.insert(entity, slot);
                continue;
            };
            // A body it joined went away (Rapier drops its joints then).
            if let Some(handle) = slot.handle {
                if self.impulse_joints.get(handle).is_none() {
                    slot.handle = None;
                    slot.pending = !slot.broken;
                }
            }
            if slot.spec.needs_rebuild(&spec) {
                if let Some(handle) = slot.handle.take() {
                    self.impulse_joints.remove(handle, true);
                }
                let slot = self.build_joint(scene, entity, spec, false);
                self.joints.insert(entity, slot);
            } else if slot.pending {
                let slot = self.build_joint(scene, entity, spec, slot.ever_attached);
                self.joints.insert(entity, slot);
            } else {
                if slot.spec != spec {
                    if let Some(joint) = slot
                        .handle
                        .and_then(|h| self.impulse_joints.get_mut(h, true))
                    {
                        joint.data = joint_data(&spec, slot.frame1, slot.frame2, slot.length);
                    }
                    slot.spec = spec;
                }
                self.joints.insert(entity, slot);
            }
        }
    }

    /// The body a joint on `entity` holds on to, and its pose: the entity
    /// named `wanted` (a fixed point where it is if it has no body), or the
    /// world. `None` (warning once) when no entity has that name.
    fn partner(
        &mut self,
        scene: &SceneWorld,
        entity: Entity,
        wanted: Option<&str>,
        ever_attached: bool,
    ) -> Option<(RigidBodyHandle, Pose)> {
        let Some(wanted) = wanted else {
            return Some((self.ground(), Pose::IDENTITY));
        };
        let found = scene
            .query::<(Entity, &Name)>()
            .iter()
            .find(|(e, n)| *e != entity && n.0 == wanted)
            .map(|(e, _)| e);
        let Some(other) = found else {
            if !ever_attached && self.joint_warned.insert(entity) {
                let own = scene
                    .get::<&Name>(entity)
                    .map_or_else(|_| format!("{entity:?}"), |n| n.0.clone());
                tracing::warn!("joint on '{own}': there's no entity named '{wanted}' to join");
            }
            return None;
        };
        if let Some(&b) = self.entity_bodies.get(&other) {
            return Some((b, *self.bodies[b].position()));
        }
        let pose = scene
            .get::<&automata_scene::Transform>(other)
            .map_or(Pose::IDENTITY, |t| pose_of(&t));
        Some((self.ground(), pose))
    }

    /// Makes the Rapier joint for `entity`'s `spec`, or a pending slot when
    /// its body or partner isn't there yet (warning once).
    fn build_joint(
        &mut self,
        scene: &SceneWorld,
        entity: Entity,
        spec: Joint,
        ever_attached: bool,
    ) -> JointSlot {
        let mut slot = JointSlot {
            spec,
            handle: None,
            frame1: Pose::IDENTITY,
            frame2: Pose::IDENTITY,
            length: 0.0,
            body1: None,
            body2: None,
            pending: true,
            broken: false,
            ever_attached,
            force: 0.0,
        };
        let name = |e: Entity| {
            scene
                .get::<&Name>(e)
                .map_or_else(|_| format!("{e:?}"), |n| n.0.clone())
        };
        let Some(&body2) = self.entity_bodies.get(&entity) else {
            if !ever_attached && self.joint_warned.insert(entity) {
                tracing::warn!(
                    "joint on '{}': it needs a Rigidbody (or a Collider) to hold",
                    name(entity)
                );
            }
            return slot;
        };
        // The partner: a named entity's body, or the world.
        let Some((body1, partner_pose)) =
            self.partner(scene, entity, slot.spec.connected.as_deref(), ever_attached)
        else {
            return slot;
        };
        let own_pose = *self.bodies[body2].position();
        let is_ground = Some(body1) == self.ground;
        let body1_pose = if is_ground {
            Pose::IDENTITY
        } else {
            partner_pose
        };

        // Frames: X runs along a slider's axis, placed at the anchor.
        let spec = &slot.spec;
        let local_rot = if spec.kind == JointKind::Slider {
            let axis = spec.axis.truncate().try_normalize().unwrap_or(Vec2::Y);
            Rotation::from_rotation_arc(Vec2::X, axis)
        } else {
            Rotation::IDENTITY
        };
        let anchor = spec.anchor.truncate();
        let frame2 = Pose::from_parts(anchor, local_rot);
        let world_anchor = own_pose.transform_point(anchor);
        let world_rot = own_pose.rotation * local_rot;
        let ends_apart = spec.kind.has_length();
        let world_other = match spec.connected_anchor {
            // In the partner's space (the world's, joined to the world).
            Some(a) if spec.connected.is_some() => partner_pose.transform_point(a.truncate()),
            Some(a) => a.truncate(),
            None if ends_apart && spec.connected.is_some() => partner_pose.translation,
            None if ends_apart => {
                world_anchor + Vec2::Y * if spec.length > 0.0 { spec.length } else { 1.0 }
            }
            None => world_anchor,
        };
        let frame1 = Pose::from_parts(
            body1_pose.inverse_transform_point(world_other),
            body1_pose.rotation.inverse() * world_rot,
        );
        let length = if spec.length > 0.0 {
            spec.length
        } else {
            world_anchor.distance(world_other).max(0.01)
        };
        let data = joint_data(spec, frame1, frame2, length);
        slot.handle = Some(self.impulse_joints.insert(body1, body2, data, true));
        slot.frame1 = frame1;
        slot.frame2 = frame2;
        slot.length = length;
        slot.body1 = Some(body1);
        slot.body2 = Some(body2);
        slot.pending = false;
        slot.ever_attached = true;
        self.joint_warned.remove(&entity);
        slot
    }

    /// After a step: measures every joint into its [`JointState`] and
    /// snaps the ones over their break force. Returns the snapped ones.
    pub(super) fn update_joints(&mut self, scene: &mut SceneWorld, dt: f32) -> Vec<Entity> {
        #[allow(clippy::cast_precision_loss)]
        let substep = dt
            / IntegrationParameters::default()
                .num_solver_iterations
                .max(1) as f32;
        let mut broken = Vec::new();
        let mut states = Vec::with_capacity(self.joints.len());
        for (&entity, slot) in &mut self.joints {
            let mut value = 0.0;
            if let Some(handle) = slot.handle {
                let Some(joint) = self.impulse_joints.get(handle) else {
                    slot.handle = None;
                    states.push((entity, slot.state(0.0)));
                    continue;
                };
                let impulse = linear_impulse(joint);
                // The solver substeps the frame, and the impulse kept is
                // the last substep's.
                slot.force = if dt > 0.0 { impulse / substep } else { 0.0 };
                let pose = |b: Option<RigidBodyHandle>| {
                    b.and_then(|b| self.bodies.get(b))
                        .map_or(Pose::IDENTITY, |rb| *rb.position())
                };
                let (p1, p2) = (pose(slot.body1), pose(slot.body2));
                value = measure(slot.spec.kind, p1 * slot.frame1, p2 * slot.frame2);
                let limit = slot.spec.break_force;
                if limit > 0.0 && slot.force > limit && slot.spec.enabled {
                    self.impulse_joints.remove(handle, true);
                    slot.handle = None;
                    slot.broken = true;
                    broken.push(entity);
                }
            }
            states.push((entity, slot.state(value)));
        }
        for (entity, state) in states {
            if let Ok(mut s) = scene.get::<&mut JointState>(entity) {
                *s = state;
                continue;
            }
            let _ = scene.insert_one(entity, state);
        }
        broken
    }

    /// Number of joints currently holding two bodies together.
    #[must_use]
    pub fn joint_count(&self) -> usize {
        self.joints.values().filter(|s| s.handle.is_some()).count()
    }
}

impl JointSlot {
    fn state(&self, value: f32) -> JointState {
        JointState {
            value,
            attached: self.handle.is_some() && self.spec.enabled,
            broken: self.broken,
            // A snapped joint keeps the force that broke it.
            force: if self.handle.is_some() || self.broken {
                self.force
            } else {
                0.0
            },
        }
    }
}

/// A transform's pose on the X/Y plane.
fn pose_of(t: &automata_scene::Transform) -> Pose {
    let angle = t.rotation.to_euler(glam::EulerRot::XYZ).2;
    Pose::new(t.position.truncate(), angle)
}

/// The impulse a joint pushed its bodies with along its linear axes last
/// substep: locked axes, limits (ropes, slider stops) and motors (springs).
fn linear_impulse(joint: &r2::dynamics::ImpulseJoint) -> f32 {
    let mut sum = 0.0;
    for i in 0..2 {
        let axis = joint.impulses[i] + joint.data.limits[i].impulse + joint.data.motors[i].impulse;
        sum += axis * axis;
    }
    sum.sqrt()
}

/// How far a joint has turned or slid (or how far apart its anchors are),
/// from its two frames in the world.
fn measure(kind: JointKind, world1: Pose, world2: Pose) -> f32 {
    match kind {
        JointKind::Hinge | JointKind::Ball => (world1.rotation.inverse() * world2.rotation).angle(),
        JointKind::Slider => {
            let axis = world1.rotation * Vec2::X;
            (world2.translation - world1.translation).dot(axis)
        }
        _ => world1.translation.distance(world2.translation),
    }
}

/// The Rapier joint for `spec` between frames already placed in each body.
fn joint_data(spec: &Joint, frame1: Pose, frame2: Pose, length: f32) -> GenericJoint {
    let mut data: GenericJoint = match spec.kind {
        JointKind::Fixed => FixedJointBuilder::new().build().into(),
        JointKind::Hinge | JointKind::Ball => RevoluteJointBuilder::new().build().into(),
        JointKind::Slider => PrismaticJointBuilder::new(Vec2::X).build().into(),
        JointKind::Rope => RopeJointBuilder::new(length).build().into(),
        JointKind::Spring => SpringJointBuilder::new(length, spec.stiffness, spec.damping)
            .spring_model(MotorModel::AccelerationBased)
            .build()
            .into(),
    };
    data.set_local_frame1(frame1);
    data.set_local_frame2(frame2);
    let axis = if spec.kind == JointKind::Slider {
        JointAxis::LinX
    } else {
        JointAxis::AngX
    };
    if let Some([lo, hi]) = spec.limits {
        match spec.kind {
            JointKind::Hinge | JointKind::Slider => {
                data.set_limits(axis, [lo.min(hi), lo.max(hi)]);
            }
            JointKind::Ball => {
                let swing = lo.abs().max(hi.abs());
                data.set_limits(JointAxis::AngX, [-swing, swing]);
            }
            _ => {}
        }
    }
    if let (Some(motor), true) = (spec.motor, spec.kind.has_motor()) {
        data.set_motor_model(axis, MotorModel::AccelerationBased);
        match motor.target {
            Some(target) => {
                data.set_motor(axis, target, motor.speed, motor.stiffness, motor.damping);
            }
            None => {
                data.set_motor_velocity(axis, motor.speed, motor.damping);
            }
        }
        if motor.max_force > 0.0 {
            data.set_motor_max_force(axis, motor.max_force);
        }
    }
    data.set_contacts_enabled(spec.collide_connected);
    data.set_enabled(spec.enabled);
    data
}
