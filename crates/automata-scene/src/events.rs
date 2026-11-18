//! Frame-scoped event dispatching.
//!
//! [`Events`] is a double-buffered, type-indexed event queue. Events sent
//! during a frame are readable during that frame and the following one;
//! [`Events::update`] is called once per frame by the engine to age the
//! buffers.

use std::any::TypeId;
use std::collections::HashMap;

/// A double-buffered event queue indexed by event type.
///
/// Events are plain Rust values (`'static + Send`). Senders use
/// [`Events::send`]; readers use [`Events::read`]. The engine calls
/// [`Events::update`] at the start of each frame; an event stays readable
/// until two updates have passed since it was sent.
#[derive(Debug, Default)]
pub struct Events {
    buffers: [HashMap<TypeId, Vec<Box<dyn std::any::Any + Send>>>; 2],
    current: usize,
}

impl Events {
    /// Creates an empty event queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends an event, readable from now until two frame updates pass.
    pub fn send<E: 'static + Send>(&mut self, event: E) {
        self.buffers[self.current]
            .entry(TypeId::of::<E>())
            .or_default()
            .push(Box::new(event));
    }

    /// Iterates over all live events of type `E`, oldest first.
    pub fn read<E: 'static>(&self) -> impl Iterator<Item = &E> {
        let id = TypeId::of::<E>();
        let old = self.buffers[1 - self.current].get(&id);
        let new = self.buffers[self.current].get(&id);
        old.into_iter()
            .chain(new)
            .flatten()
            .filter_map(|e| e.downcast_ref::<E>())
    }

    /// Counts the live events of type `E`.
    #[must_use]
    pub fn count<E: 'static>(&self) -> usize {
        self.read::<E>().count()
    }

    /// Ages the buffers: the current buffer becomes the old one, and the
    /// new current buffer starts empty. Called once per frame by the
    /// engine; games should not call this.
    pub fn update(&mut self) {
        self.current = 1 - self.current;
        self.buffers[self.current].clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Jump {
        height: f32,
    }

    #[derive(Debug, PartialEq)]
    struct Score(u32);

    #[test]
    fn events_are_readable_in_the_frame_they_are_sent() {
        let mut events = Events::new();
        events.send(Jump { height: 2.0 });
        assert_eq!(events.read::<Jump>().count(), 1);
    }

    #[test]
    fn events_survive_into_the_next_frame() {
        let mut events = Events::new();
        events.send(Score(10));
        events.update();
        assert_eq!(events.read::<Score>().count(), 1);
    }

    #[test]
    fn events_expire_after_two_frames() {
        let mut events = Events::new();
        events.send(Score(1));
        events.update();
        events.update();
        assert_eq!(events.read::<Score>().count(), 0);
    }

    #[test]
    fn different_types_are_independent() {
        let mut events = Events::new();
        events.send(Score(5));
        events.send(Jump { height: 1.0 });
        events.send(Jump { height: 3.0 });

        let jumps: Vec<f32> = events.read::<Jump>().map(|j| j.height).collect();
        assert_eq!(jumps, vec![1.0, 3.0]);
        assert_eq!(events.count::<Score>(), 1);
    }

    #[test]
    fn events_arrive_in_send_order() {
        let mut events = Events::new();
        events.send(Score(1));
        events.update();
        events.send(Score(2));
        let scores: Vec<u32> = events.read::<Score>().map(|s| s.0).collect();
        assert_eq!(scores, vec![1, 2]);
    }
}
