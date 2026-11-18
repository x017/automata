//! Generational resource handles and typed asset registries.
//!
//! [`Handle<T>`] is a cheap, copyable reference to a value stored in an
//! [`Assets<T>`] registry. Handles are generational: a slot freed by
//! [`Assets::remove`] can be reused, but the generation counter prevents
//! stale handles from silently resolving to a new value.
//!
//! This is the foundation Phase 7's asset loading builds on; the renderer's
//! `Texture`/`Mesh`/`Material` types are the values stored in registries.

use std::marker::PhantomData;

/// A typed, generational handle into an [`Assets<T>`] registry.
///
/// `Copy` and cheap to pass around. The `PhantomData<fn() -> T>` marker
/// keeps the handle `Send + Sync` regardless of `T`.
pub struct Handle<T> {
    index: u32,
    generation: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            marker: PhantomData,
        }
    }
}

// Manual trait impls avoid imposing trait bounds on `T`.
impl<T> Copy for Handle<T> {}
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T> Eq for Handle<T> {}
impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}
impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// One registry slot: the value plus the generation it was stored with.
#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A registry of values of type `T`, addressed by [`Handle<T>`].
#[derive(Debug, Default)]
pub struct Assets<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

impl<T> Assets<T> {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// Stores a value and returns its handle.
    ///
    /// # Panics
    ///
    /// Panics if the registry holds more than `u32::MAX` entries.
    #[must_use]
    pub fn add(&mut self, value: T) -> Handle<T> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.generation += 1;
            slot.value = Some(value);
            Handle::new(index, slot.generation)
        } else {
            let index = u32::try_from(self.slots.len()).expect("registry slot count fits u32");
            self.slots.push(Slot {
                generation: 0,
                value: Some(value),
            });
            Handle::new(index, 0)
        }
    }

    /// The value a handle points to; `None` for stale or unknown handles.
    #[must_use]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.slot(handle).and_then(|s| s.value.as_ref())
    }

    /// The value a handle points to, mutably.
    #[must_use]
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.slot_mut(handle).and_then(|s| s.value.as_mut())
    }

    /// Removes the value a handle points to; the slot can be reused with a
    /// bumped generation.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let slot = self.slot_mut(handle)?;
        let value = slot.value.take()?;
        self.free.push(handle.index);
        Some(value)
    }

    /// The number of live values in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    /// Whether the registry holds no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn slot(&self, handle: Handle<T>) -> Option<&Slot<T>> {
        self.slots
            .get(handle.index as usize)
            .filter(|s| s.generation == handle.generation)
    }

    fn slot_mut(&mut self, handle: Handle<T>) -> Option<&mut Slot<T>> {
        self.slots
            .get_mut(handle.index as usize)
            .filter(|s| s.generation == handle.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_get_roundtrip() {
        let mut assets = Assets::new();
        let handle = assets.add(String::from("sword"));
        assert_eq!(assets.get(handle).map(String::as_str), Some("sword"));
    }

    #[test]
    fn handles_are_copy_and_comparable() {
        let mut assets = Assets::<u32>::new();
        let a = assets.add(1);
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn stale_handle_does_not_resolve_after_reuse() {
        let mut assets = Assets::new();
        let old = assets.add(1);
        assets.remove(old);
        let new = assets.add(2); // reuses the slot with a bumped generation

        assert_eq!(assets.get(new), Some(&2));
        assert_eq!(assets.get(old), None, "stale handle must not resolve");
    }

    #[test]
    fn remove_makes_slot_reusable() {
        let mut assets = Assets::new();
        let a = assets.add("a");
        let b = assets.add("b");
        assert_eq!(assets.len(), 2);
        assert_eq!(assets.remove(a), Some("a"));
        assert_eq!(assets.len(), 1);
        assert_eq!(assets.get(b), Some(&"b"));
        let c = assets.add("c");
        assert_eq!(assets.len(), 2);
        assert_eq!(assets.get(c), Some(&"c"));
    }

    #[test]
    fn get_mut_edits_in_place() {
        let mut assets = Assets::new();
        let handle = assets.add(1u32);
        *assets.get_mut(handle).unwrap() += 41;
        assert_eq!(assets.get(handle), Some(&42));
    }
}
