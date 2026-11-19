//! Engine-level physics events delivered to the game's event bus.
//!
//! These are intentionally free of Rapier handles — entities are identified
//! by their stable [`EntityId`] and every field is plain data, so events can
//! cross the Python boundary and be matched with `==` in tests.

use automata_scene::EntityId;
use glam::Vec3;

/// A collision event from the physics world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PhysicsEvent {
    /// Two entities started touching this step.
    CollisionStarted {
        /// One of the two entities.
        entity_a: EntityId,
        /// The other entity.
        entity_b: EntityId,
    },
    /// Two entities stopped touching this step.
    CollisionStopped {
        /// One of the two entities.
        entity_a: EntityId,
        /// The other entity.
        entity_b: EntityId,
    },
}

/// A contact-force event: a contact pair whose total force exceeded the
/// collider's `contact_force_event_threshold`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactForce {
    /// One of the two entities.
    pub entity_a: EntityId,
    /// The other entity.
    pub entity_b: EntityId,
    /// Sum of all contact forces between the two colliders.
    pub total_force: Vec3,
    /// Largest single contact-force magnitude.
    pub max_force_magnitude: f32,
}
