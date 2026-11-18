//! The scene: an ECS world plus hierarchy and lifecycle helpers.
//!
//! A [`Scene`] wraps a `hecs` world with AUTOMATA's conventions:
//! [`Transform`] hierarchy propagation, recursive despawn, and a per-frame
//! [`Scene::update`] that runs the built-in systems. The ECS itself is
//! `hecs` (see `docs/architecture.md` for the evaluation).

use hecs::{DynamicBundle, Entity, NoSuchEntity, World};

use crate::transform::{propagate_transforms, Parent};

/// The ECS world type. Re-exported so game code can write queries without
/// depending on `hecs` directly for the common case.
pub type SceneWorld = World;

/// An entity identifier. Re-exported from `hecs`.
pub type EntityId = Entity;

/// A game world: entities, components, and hierarchy management.
///
/// One active scene is owned by the application; games mutate it through
/// the per-frame context. Scenes support both 2D and 3D content — the
/// renderer bridge ([`crate::record_draw_list`]) reads the world each frame.
#[derive(Default)]
pub struct Scene {
    world: World,
}

impl Scene {
    /// Creates an empty scene.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns an entity from a component bundle (a tuple of components).
    ///
    /// ```
    /// use automata_scene::{Scene, Transform};
    /// use glam::Vec3;
    ///
    /// let mut scene = Scene::new();
    /// let entity = scene.spawn((Transform::from_position(Vec3::ONE),));
    /// ```
    pub fn spawn(&mut self, bundle: impl DynamicBundle) -> EntityId {
        self.world.spawn(bundle)
    }

    /// Despawns an entity. Children (entities with a [`Parent`] pointing at
    /// it) are orphaned: they keep their [`crate::Transform`] and fall back to
    /// being roots. Use [`Scene::despawn_recursive`] to remove a subtree.
    ///
    /// # Errors
    ///
    /// Returns [`NoSuchEntity`] if the entity is not alive in this scene.
    pub fn despawn(&mut self, entity: EntityId) -> Result<(), NoSuchEntity> {
        self.world.despawn(entity)
    }

    /// Despawns an entity and all of its descendants. Returns the number of
    /// entities removed (including `entity` itself; 0 if it was not alive).
    pub fn despawn_recursive(&mut self, entity: EntityId) -> usize {
        if !self.world.contains(entity) {
            return 0;
        }
        // Collect the subtree bottom-up before despawning.
        let mut subtree = vec![entity];
        let mut cursor = 0;
        while cursor < subtree.len() {
            let current = subtree[cursor];
            cursor += 1;
            for (child, parent) in &mut self.world.query::<(hecs::Entity, &Parent)>() {
                if parent.0 == current && !subtree.contains(&child) {
                    subtree.push(child);
                }
            }
        }
        let mut removed = 0;
        for member in subtree {
            if self.world.despawn(member).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Makes `child` a child of `parent` in the transform hierarchy.
    pub fn set_parent(&mut self, child: EntityId, parent: EntityId) {
        let _ = self.world.insert_one(child, Parent(parent));
    }

    /// Removes the parent link, making `entity` a hierarchy root.
    pub fn remove_parent(&mut self, entity: EntityId) {
        let _ = self.world.remove_one::<Parent>(entity);
    }

    /// The number of live entities.
    #[must_use]
    pub fn entity_count(&self) -> u32 {
        self.world.len()
    }

    /// Read-only access to the ECS world (for queries).
    #[must_use]
    pub fn world(&self) -> &SceneWorld {
        &self.world
    }

    /// Mutable access to the ECS world (for queries and component edits).
    #[must_use]
    pub fn world_mut(&mut self) -> &mut SceneWorld {
        &mut self.world
    }

    /// Runs the scene's built-in systems once per frame, after the game
    /// update: currently transform propagation.
    pub fn update(&mut self) {
        propagate_transforms(&mut self.world);
    }
}

/// An optional human-readable name for an entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(pub String);

impl Name {
    /// Creates a name component.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::Transform;

    #[test]
    fn spawn_and_despawn() {
        let mut scene = Scene::new();
        let e = scene.spawn((Name::new("hero"),));
        assert_eq!(scene.entity_count(), 1);
        scene.despawn(e).unwrap();
        assert_eq!(scene.entity_count(), 0);
    }

    #[test]
    fn despawn_of_dead_entity_errors() {
        let mut scene = Scene::new();
        let e = scene.spawn(());
        scene.despawn(e).unwrap();
        assert!(scene.despawn(e).is_err());
    }

    #[test]
    fn despawn_recursive_removes_subtree() {
        let mut scene = Scene::new();
        let root = scene.spawn((Transform::default(),));
        let child = scene.spawn((Transform::default(),));
        let grandchild = scene.spawn((Transform::default(),));
        let unrelated = scene.spawn((Transform::default(),));
        scene.set_parent(child, root);
        scene.set_parent(grandchild, child);

        assert_eq!(scene.despawn_recursive(root), 3);
        assert_eq!(scene.entity_count(), 1);
        assert!(scene.world().contains(unrelated));
    }

    #[test]
    fn plain_despawn_orphans_children() {
        let mut scene = Scene::new();
        let root = scene.spawn((Transform::default(),));
        let child = scene.spawn((Transform::default(),));
        scene.set_parent(child, root);

        scene.despawn(root).unwrap();
        assert!(scene.world().contains(child));
    }

    #[test]
    fn remove_parent_makes_root() {
        let mut scene = Scene::new();
        let root = scene.spawn((Transform::default(),));
        let child = scene.spawn((Transform::default(),));
        scene.set_parent(child, root);
        scene.remove_parent(child);
        assert!(scene.world().get::<&Parent>(child).is_err());
    }
}
