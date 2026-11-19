//! Shared physics-world types: ray hits, filters, character moves, ticks.

use automata_scene::Entity;
use glam::Vec3;

use crate::events::{ContactForce, PhysicsEvent};

/// A ray hit: which entity was hit, where, and with what normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// The entity whose collider was hit.
    pub entity: Entity,
    /// World-space point of impact.
    pub point: Vec3,
    /// World-space surface normal at the impact point.
    pub normal: Vec3,
    /// Distance from the ray origin to the impact point.
    pub distance: f32,
}

/// What to include/exclude in a raycast or shape cast.
#[derive(Debug, Clone, Default)]
pub struct RayFilter {
    /// Skip colliders flagged as sensors.
    pub exclude_sensors: bool,
    /// Skip the colliders of this entity (e.g. the shooter).
    pub exclude_entity: Option<Entity>,
    /// Only report "solid" contacts (not sensors, not back-faces).
    pub solid: bool,
}

impl RayFilter {
    /// A filter that hits everything.
    pub fn everything() -> Self {
        Self::default()
    }

    /// A filter that skips sensors.
    pub fn no_sensors() -> Self {
        Self {
            exclude_sensors: true,
            ..Self::default()
        }
    }

    /// Skip the given entity (e.g. the character casting the ray).
    #[must_use]
    pub fn excluding(mut self, entity: Entity) -> Self {
        self.exclude_entity = Some(entity);
        self
    }
}

/// The outcome of moving a character controller one step.
#[derive(Debug, Default)]
pub struct CharacterMove {
    /// The displacement actually applied (collision-resolved).
    pub translation: Vec3,
    /// Whether the character is standing on the ground.
    pub grounded: bool,
    /// Whether the character is sliding down a too-steep slope.
    pub sliding: bool,
    /// Entities the character collided with during the move.
    pub collisions: Vec<Entity>,
}

/// The result of stepping the world for one fixed tick.
#[derive(Debug, Default)]
pub struct PhysicsTick {
    /// Collision start/stop events.
    pub collisions: Vec<PhysicsEvent>,
    /// Contact-force events (forces over the configured threshold).
    pub contact_forces: Vec<ContactForce>,
}

pub(crate) fn entity_to_user_data(entity: Entity) -> u128 {
    u128::from(entity.to_bits().get())
}

pub(crate) fn user_data_to_entity(bits: u128) -> Option<Entity> {
    Entity::from_bits(bits as u64)
}
