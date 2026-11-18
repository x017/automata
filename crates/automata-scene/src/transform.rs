//! Transforms and parent-child hierarchies.
//!
//! Entities carry a local [`Transform`] (position, rotation, scale relative
//! to the parent) and receive a [`GlobalTransform`] (model-space to
//! world-space matrix) from [`propagate_transforms`], which the scene runs
//! once per frame after game updates.

use std::collections::{HashMap, HashSet};

use glam::{Mat4, Quat, Vec3};

use crate::scene::SceneWorld;

/// Position, rotation, and scale relative to the parent (or world origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Translation relative to the parent.
    pub position: Vec3,
    /// Rotation relative to the parent.
    pub rotation: Quat,
    /// Scale relative to the parent.
    pub scale: Vec3,
}

impl Transform {
    /// The identity transform: origin, no rotation, unit scale.
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Creates a transform at `position` with no rotation and unit scale.
    #[must_use]
    pub fn from_position(position: Vec3) -> Self {
        Self {
            position,
            ..Self::IDENTITY
        }
    }

    /// Creates a transform at `(x, y)` — convenience for 2D scenes.
    #[must_use]
    pub fn from_xy(x: f32, y: f32) -> Self {
        Self::from_position(Vec3::new(x, y, 0.0))
    }

    /// Sets the rotation.
    #[must_use]
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets the scale.
    #[must_use]
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// The local model matrix (scale, then rotate, then translate).
    #[must_use]
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The world-space transform, computed each frame by
/// [`propagate_transforms`]. Rendering reads this; games write
/// [`Transform`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalTransform(pub Mat4);

impl GlobalTransform {
    /// The world-space matrix.
    #[must_use]
    pub fn matrix(&self) -> Mat4 {
        self.0
    }

    /// The world-space position.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        self.0.transform_point3(Vec3::ZERO)
    }
}

/// Marks an entity as a child of another entity. The child's
/// [`GlobalTransform`] composes with the parent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent(pub hecs::Entity);

/// Computes [`GlobalTransform`] for every entity with a [`Transform`]:
/// parentless entities take their local matrix; children compose with the
/// parent's global transform.
///
/// Orphaned children (their parent has no [`Transform`], e.g. because the
/// parent was despawned) are treated as roots. Cycles are broken by
/// treating the revisited entity as a root. Stale [`GlobalTransform`]
/// components on entities without a [`Transform`] are removed.
pub fn propagate_transforms(world: &mut SceneWorld) {
    // Collect locals and parents in one immutable pass.
    let mut locals: HashMap<hecs::Entity, (Mat4, Option<hecs::Entity>)> = HashMap::new();
    for (entity, transform) in &mut world.query::<(hecs::Entity, &Transform)>() {
        let parent = world.get::<&Parent>(entity).ok().map(|p| p.0);
        locals.insert(entity, (transform.to_matrix(), parent));
    }

    // Resolve each entity's global matrix, memoizing up the parent chain.
    let mut memo: HashMap<hecs::Entity, Mat4> = HashMap::with_capacity(locals.len());
    for entity in locals.keys().copied().collect::<Vec<_>>() {
        let mut visiting = HashSet::new();
        resolve_global(entity, &locals, &mut memo, &mut visiting);
    }

    // Write back: insert_or_replace GlobalTransform on transformed entities.
    for (entity, global) in memo {
        // `insert_one` replaces an existing component of the same type.
        let _ = world.insert_one(entity, GlobalTransform(global));
    }

    // Remove GlobalTransform from entities that lost their Transform.
    let stale: Vec<hecs::Entity> = (&mut world.query::<(hecs::Entity, &GlobalTransform)>())
        .into_iter()
        .filter(|(entity, _)| world.get::<&Transform>(*entity).is_err())
        .map(|(entity, _)| entity)
        .collect();
    for entity in stale {
        let _ = world.remove_one::<GlobalTransform>(entity);
    }
}

/// Resolves `entity`'s global matrix, recursing into parents. Entities
/// already in `visiting` form a cycle and resolve to identity.
fn resolve_global(
    entity: hecs::Entity,
    locals: &HashMap<hecs::Entity, (Mat4, Option<hecs::Entity>)>,
    memo: &mut HashMap<hecs::Entity, Mat4>,
    visiting: &mut HashSet<hecs::Entity>,
) -> Mat4 {
    if let Some(global) = memo.get(&entity) {
        return *global;
    }
    let Some(&(local, parent)) = locals.get(&entity) else {
        return Mat4::IDENTITY; // not a transformed entity (dangling Parent)
    };
    if !visiting.insert(entity) {
        return Mat4::IDENTITY; // cycle
    }
    let global = match parent {
        Some(p) => resolve_global(p, locals, memo, visiting) * local,
        None => local,
    };
    visiting.remove(&entity);
    memo.insert(entity, global);
    global
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;

    fn global_position(scene: &Scene, entity: hecs::Entity) -> Vec3 {
        scene
            .world()
            .get::<&GlobalTransform>(entity)
            .expect("entity should have a GlobalTransform")
            .position()
    }

    #[test]
    fn root_entity_gets_its_local_transform() {
        let mut scene = Scene::new();
        let e = scene.spawn((Transform::from_position(Vec3::new(1.0, 2.0, 3.0)),));
        scene.update();
        assert_eq!(global_pos_helper(&scene, e), Vec3::new(1.0, 2.0, 3.0));
    }

    fn global_pos_helper(scene: &Scene, entity: hecs::Entity) -> Vec3 {
        global_position(scene, entity)
    }

    #[test]
    fn child_composes_with_parent() {
        let mut scene = Scene::new();
        let parent = scene.spawn((Transform::from_position(Vec3::new(10.0, 0.0, 0.0)),));
        let child = scene.spawn((Transform::from_position(Vec3::new(0.0, 5.0, 0.0)),));
        scene.set_parent(child, parent);
        scene.update();

        assert_eq!(global_position(&scene, child), Vec3::new(10.0, 5.0, 0.0));
    }

    #[test]
    fn grandchild_composes_full_chain() {
        let mut scene = Scene::new();
        let a = scene.spawn((Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),));
        let b = scene.spawn((Transform::from_position(Vec3::new(0.0, 1.0, 0.0)),));
        let c = scene.spawn((Transform::from_position(Vec3::new(0.0, 0.0, 1.0)),));
        scene.set_parent(b, a);
        scene.set_parent(c, b);
        scene.update();

        assert_eq!(global_position(&scene, c), Vec3::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn parent_rotation_and_scale_apply_to_child() {
        let mut scene = Scene::new();
        let parent = scene.spawn((Transform::from_position(Vec3::ZERO)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(2.0)),));
        let child = scene.spawn((Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),));
        scene.set_parent(child, parent);
        scene.update();

        let p = global_position(&scene, child);
        // Rotate (1,0,0) by 90° around Z -> (0,1,0), then scale by 2.
        assert!((p - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-4, "{p:?}");
    }

    #[test]
    fn moving_parent_moves_child_next_frame() {
        let mut scene = Scene::new();
        let parent = scene.spawn((Transform::from_position(Vec3::ZERO),));
        let child = scene.spawn((Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),));
        scene.set_parent(child, parent);
        scene.update();
        assert_eq!(global_position(&scene, child), Vec3::new(1.0, 0.0, 0.0));

        scene
            .world_mut()
            .get::<&mut Transform>(parent)
            .unwrap()
            .position = Vec3::new(5.0, 0.0, 0.0);
        scene.update();
        assert_eq!(global_position(&scene, child), Vec3::new(6.0, 0.0, 0.0));
    }

    #[test]
    fn orphan_child_is_treated_as_root() {
        let mut scene = Scene::new();
        let parent = scene.spawn((Transform::from_position(Vec3::new(10.0, 0.0, 0.0)),));
        let child = scene.spawn((Transform::from_position(Vec3::new(0.0, 5.0, 0.0)),));
        scene.set_parent(child, parent);
        scene.despawn(parent).unwrap();
        scene.update();

        // The parent is gone; the child falls back to its local transform.
        assert_eq!(global_position(&scene, child), Vec3::new(0.0, 5.0, 0.0));
    }

    #[test]
    fn cycle_does_not_hang() {
        let mut scene = Scene::new();
        let a = scene.spawn((Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),));
        let b = scene.spawn((Transform::from_position(Vec3::new(0.0, 1.0, 0.0)),));
        scene.set_parent(a, b);
        scene.set_parent(b, a);
        scene.update(); // must terminate

        // Both entities still get some global transform.
        assert!(scene.world().get::<&GlobalTransform>(a).is_ok());
        assert!(scene.world().get::<&GlobalTransform>(b).is_ok());
    }

    #[test]
    fn removing_transform_removes_global_transform() {
        let mut scene = Scene::new();
        let e = scene.spawn((Transform::from_position(Vec3::ONE),));
        scene.update();
        assert!(scene.world().get::<&GlobalTransform>(e).is_ok());

        scene.world_mut().remove_one::<Transform>(e).unwrap();
        scene.update();
        assert!(scene.world().get::<&GlobalTransform>(e).is_err());
    }
}
