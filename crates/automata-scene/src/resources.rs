//! Shared, type-indexed resources.
//!
//! Resources hold application-wide state that is not tied to an entity:
//! configuration, scores, asset registries ([`crate::Assets`]), timers,
//! and similar. One value per type.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// A type-indexed store of shared resources.
///
/// One value per Rust type; inserting the same type twice replaces the old
/// value. Resources must be `Send + Sync` so the store stays thread-safe.
#[derive(Debug, Default)]
pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    /// Creates an empty resource store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts (or replaces) a resource of type `T`.
    pub fn insert<T: 'static + Send + Sync>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Returns the resource of type `T`, if present.
    #[must_use]
    pub fn get<T: 'static + Send + Sync>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    /// Returns the resource of type `T` mutably, if present.
    #[must_use]
    pub fn get_mut<T: 'static + Send + Sync>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut::<T>())
    }

    /// Removes and returns the resource of type `T`, if present.
    pub fn remove<T: 'static + Send + Sync>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast::<T>().ok())
            .map(|b| *b)
    }

    /// Whether a resource of type `T` is present.
    #[must_use]
    pub fn contains<T: 'static + Send + Sync>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Score(u32);
    struct Volume(f32);

    #[test]
    fn insert_and_get_roundtrip() {
        let mut resources = Resources::new();
        resources.insert(Score(42));
        assert_eq!(resources.get::<Score>().map(|s| s.0), Some(42));
    }

    #[test]
    fn types_are_independent() {
        let mut resources = Resources::new();
        resources.insert(Score(1));
        resources.insert(Volume(0.5));
        assert!(resources.contains::<Score>());
        assert!(resources.contains::<Volume>());
        assert_eq!(resources.get::<Volume>().map(|v| v.0), Some(0.5));
    }

    #[test]
    fn insert_replaces_existing_value() {
        let mut resources = Resources::new();
        resources.insert(Score(1));
        resources.insert(Score(2));
        assert_eq!(resources.get::<Score>().map(|s| s.0), Some(2));
    }

    #[test]
    fn get_mut_edits_in_place() {
        let mut resources = Resources::new();
        resources.insert(Score(1));
        resources.get_mut::<Score>().unwrap().0 += 9;
        assert_eq!(resources.get::<Score>().map(|s| s.0), Some(10));
    }

    #[test]
    fn remove_returns_and_clears() {
        let mut resources = Resources::new();
        resources.insert(Score(7));
        assert_eq!(resources.remove::<Score>().map(|s| s.0), Some(7));
        assert!(!resources.contains::<Score>());
    }
}
