//! Attaching entities to bones: a sword in a hand, a hat on a head.
//!
//! An entity with an [`Attachment`] follows a bone of another entity's
//! animated model (or, with no bone, the entity itself), placed by its
//! offset in the bone's space. It keeps its own size: the bone's position
//! and rotation carry it, not the model's scale. [`attach_to_bones`]
//! places every attached entity, writing its [`Transform`] (now where it
//! is in the world) and [`GlobalTransform`]; [`crate::animate`] runs it
//! after posing, and it can run again after anything else moves the
//! models (physics).
//!
//! The entity to follow is named, as scene files have no other way to
//! point at one; the first entity with that name is followed. Attachments
//! can chain (a gem on a sword in a hand); a loop just stops following.

use std::collections::HashMap;

use glam::Mat4;
use hecs::Entity;

use crate::animation::Animator;
use crate::render::MeshVisual;
use crate::scene::{Name, SceneWorld};
use crate::transform::{GlobalTransform, Parent, Transform};

/// Follows a bone of another entity (also its ECS component).
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// The name of the entity followed.
    pub to: String,
    /// The bone followed (empty: the entity's origin).
    pub bone: String,
    /// Where the entity sits in the bone's space.
    pub offset: Transform,
    /// The entity found for `to` last time.
    target: Option<Entity>,
}

impl Attachment {
    /// Follows `bone` of the entity named `to`, at `offset` from it.
    #[must_use]
    pub fn new(to: impl Into<String>, bone: impl Into<String>, offset: Transform) -> Self {
        Self {
            to: to.into(),
            bone: bone.into(),
            offset,
            target: None,
        }
    }

    /// The entity followed, once [`attach_to_bones`] has found it.
    #[must_use]
    pub fn resolved(&self) -> Option<Entity> {
        self.target
    }
}

/// How deep attachments may chain.
const MAX_CHAIN: usize = 8;

/// Where the bone named `bone` of `entity`'s model is in the entity's
/// space, position and rotation only: from its animator's pose, else its
/// model's rest pose. `None` when there's no such bone.
#[must_use]
pub fn bone_frame(world: &SceneWorld, entity: Entity, bone: &str) -> Option<Mat4> {
    if bone.is_empty() {
        return Some(Mat4::IDENTITY);
    }
    let m = if let Ok(animator) = world.get::<&Animator>(entity) {
        animator.bone_matrix(bone)?
    } else {
        let visual = world.get::<&MeshVisual>(entity).ok()?;
        let rig = visual.model.as_ref()?.rig.as_ref()?;
        let node = rig.node(bone)?;
        rig.import * rig.globals(&rig.rest_pose())[node]
    };
    let (_, rotation, translation) = m.to_scale_rotation_translation();
    Some(Mat4::from_rotation_translation(
        rotation.normalize(),
        translation,
    ))
}

/// Where an entity that isn't attached is: its own transform when it's
/// a root (fresher than its global one, which waits for the next
/// propagation), else its global transform.
fn placed(world: &SceneWorld, entity: Entity) -> Mat4 {
    if world.get::<&Parent>(entity).is_err() {
        if let Ok(t) = world.get::<&Transform>(entity) {
            return t.to_matrix();
        }
    }
    world
        .get::<&GlobalTransform>(entity)
        .map_or(Mat4::IDENTITY, |g| g.0)
}

/// Removes a matrix's scale, keeping its position and rotation.
fn unscaled(m: Mat4) -> Mat4 {
    let (_, rotation, translation) = m.to_scale_rotation_translation();
    Mat4::from_rotation_translation(rotation.normalize(), translation)
}

/// Places every attached entity on its bone (see the module docs).
pub fn attach_to_bones(world: &mut SceneWorld) {
    let mut attached: HashMap<Entity, (String, String, Transform, Option<Entity>)> = world
        .query::<(Entity, &Attachment)>()
        .iter()
        .map(|(e, a)| (e, (a.to.clone(), a.bone.clone(), a.offset, a.target)))
        .collect();
    if attached.is_empty() {
        return;
    }
    // Find (again) the entities followed.
    let mut names: Option<HashMap<String, Entity>> = None;
    for (entity, (to, _, _, target)) in &mut attached {
        let still =
            target.filter(|t| *t != *entity && world.get::<&Name>(*t).is_ok_and(|n| n.0 == *to));
        *target = still.or_else(|| {
            let names = names.get_or_insert_with(|| {
                let mut map = HashMap::new();
                for (e, name) in &mut world.query::<(Entity, &Name)>() {
                    map.entry(name.0.clone()).or_insert(e);
                }
                map
            });
            names.get(to).copied().filter(|t| t != entity)
        });
    }
    let mut placed_at: HashMap<Entity, Mat4> = HashMap::new();
    let order: Vec<Entity> = attached.keys().copied().collect();
    for entity in order {
        resolve(world, entity, &attached, &mut placed_at, 0);
    }
    for (entity, (_, _, _, target)) in &attached {
        if let Ok(mut a) = world.get::<&mut Attachment>(*entity) {
            a.target = *target;
        }
        let Some(&m) = placed_at.get(entity) else {
            continue;
        };
        if world.get::<&Parent>(*entity).is_err() {
            if let Ok(mut t) = world.get::<&mut Transform>(*entity) {
                let (scale, rotation, position) = m.to_scale_rotation_translation();
                *t = Transform {
                    position,
                    rotation: rotation.normalize(),
                    scale,
                };
            }
        }
        let _ = world.insert_one(*entity, GlobalTransform(m));
    }
}

/// Where `entity` is in the world, following its attachment chain.
fn resolve(
    world: &SceneWorld,
    entity: Entity,
    attached: &HashMap<Entity, (String, String, Transform, Option<Entity>)>,
    placed_at: &mut HashMap<Entity, Mat4>,
    depth: usize,
) -> Mat4 {
    if let Some(&m) = placed_at.get(&entity) {
        return m;
    }
    let Some((_, bone, offset, target)) = attached.get(&entity) else {
        return placed(world, entity);
    };
    let Some(target) = target.filter(|_| depth < MAX_CHAIN) else {
        // Following nothing (or a loop): where it is.
        let m = placed(world, entity);
        placed_at.insert(entity, m);
        return m;
    };
    let parent = unscaled(resolve(world, target, attached, placed_at, depth + 1));
    let frame = bone_frame(world, target, bone).unwrap_or(Mat4::IDENTITY);
    let m = parent * frame * offset.to_matrix();
    placed_at.insert(entity, m);
    m
}

/// The offset that puts an entity attached to `bone_world` (a bone's
/// place in the world, as [`bone_world`] gives it) at `world`: what an
/// editor stores when the attached entity is moved.
#[must_use]
pub fn offset_for(bone_world: Mat4, world: Mat4) -> Transform {
    let (scale, rotation, position) =
        (bone_world.inverse() * world).to_scale_rotation_translation();
    Transform {
        position,
        rotation: rotation.normalize(),
        scale,
    }
}

/// Where the bone an [`Attachment`] follows is in the world, position and
/// rotation only (identity when it follows nothing found): its offset
/// times this places the attached entity.
#[must_use]
pub fn bone_world(world: &SceneWorld, attachment: &Attachment) -> Mat4 {
    let Some(target) = attachment.target else {
        return Mat4::IDENTITY;
    };
    let base = world
        .get::<&GlobalTransform>(target)
        .map_or_else(|_| placed(world, target), |g| g.0);
    unscaled(base) * bone_frame(world, target, &attachment.bone).unwrap_or(Mat4::IDENTITY)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::scene::Scene;
    use automata_assets::anim::{RigNode, Trs};
    use automata_assets::Rig;
    use glam::{Quat, Vec3};

    fn arm_rig() -> Arc<Rig> {
        Arc::new(Rig {
            nodes: vec![
                RigNode {
                    name: "Shoulder".into(),
                    parent: None,
                    rest: Trs {
                        translation: Vec3::new(0.0, 1.0, 0.0),
                        ..Trs::IDENTITY
                    },
                },
                RigNode {
                    name: "Hand".into(),
                    parent: Some(0),
                    rest: Trs {
                        translation: Vec3::new(0.5, 0.0, 0.0),
                        rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                        ..Trs::IDENTITY
                    },
                },
            ],
            joints: Vec::new(),
            import: Mat4::from_scale(Vec3::splat(2.0)),
            clips: Vec::new(),
            springs: automata_assets::springs::Springs::default(),
        })
    }

    #[test]
    fn entities_follow_bones_and_keep_their_size() {
        let mut scene = Scene::new();
        let world = scene.world_mut();
        let robot = world.spawn((
            Name::new("Robot"),
            Transform::from_position(Vec3::new(10.0, 0.0, 0.0)),
            Animator::new(arm_rig()),
        ));
        let offset =
            Transform::from_position(Vec3::new(0.0, 0.2, 0.0)).with_scale(Vec3::splat(0.5));
        let sword = world.spawn((
            Name::new("Sword"),
            Transform::IDENTITY,
            Attachment::new("Robot", "Hand", offset),
        ));
        // A gem on the sword's tip (no bone: the sword itself).
        let gem = world.spawn((
            Transform::IDENTITY,
            Attachment::new("Sword", "", Transform::from_position(Vec3::Y)),
        ));
        attach_to_bones(world);
        // The hand: (10, 0, 0) + 2 * (0.5, 1, 0), turned 90° about Z, so
        // the offset's +Y points along -X.
        let t = *world.get::<&Transform>(sword).unwrap();
        assert!(
            t.position.distance(Vec3::new(11.0 - 0.2, 2.0, 0.0)) < 1e-4,
            "{t:?}"
        );
        assert!(
            (t.scale - Vec3::splat(0.5)).length() < 1e-4,
            "the model's scale isn't kept"
        );
        assert_eq!(
            world.get::<&Attachment>(sword).unwrap().resolved(),
            Some(robot)
        );
        let g = world.get::<&GlobalTransform>(gem).unwrap().position();
        // The sword's own scale doesn't carry over either.
        assert!(g.distance(Vec3::new(10.8 - 1.0, 2.0, 0.0)) < 1e-4, "{g}");
        // The robot moves: the sword comes along.
        world.get::<&mut Transform>(robot).unwrap().position.z = 3.0;
        attach_to_bones(world);
        assert!((world.get::<&Transform>(sword).unwrap().position.z - 3.0).abs() < 1e-4);
        // Moving the sword in the world gives back its offset.
        let a = world.get::<&Attachment>(sword).unwrap().clone();
        let back = offset_for(
            bone_world(world, &a),
            world.get::<&GlobalTransform>(sword).unwrap().0,
        );
        assert!(back.position.distance(offset.position) < 1e-4);
        assert!((back.scale - offset.scale).length() < 1e-4);
    }

    #[test]
    fn missing_targets_and_loops_leave_entities_where_they_are() {
        let mut scene = Scene::new();
        let world = scene.world_mut();
        let at = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
        let lost = world.spawn((at, Attachment::new("Nobody", "Hand", Transform::IDENTITY)));
        let a = world.spawn((
            Name::new("A"),
            at,
            Attachment::new("B", "", Transform::from_position(Vec3::X)),
        ));
        let b = world.spawn((
            Name::new("B"),
            at,
            Attachment::new("A", "", Transform::from_position(Vec3::X)),
        ));
        attach_to_bones(world);
        assert_eq!(*world.get::<&Transform>(lost).unwrap(), at);
        for e in [a, b] {
            assert!(world.get::<&Transform>(e).unwrap().position.is_finite());
        }
    }
}
