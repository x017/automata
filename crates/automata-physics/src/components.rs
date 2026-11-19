//! Physics components: rigid bodies, colliders, and character controllers.
//!
//! Components describe *intent*: [`RigidBody`] marks an entity for the
//! physics world, [`Collider`] gives it a shape, and
//! [`CharacterController`] makes the engine move a kinematic body with
//! slide/slope/autostep logic. The [`PhysicsWorld3D`](crate::PhysicsWorld3D)
//! synchronises them with Rapier at the start of every fixed step.
//!
//! Units are metres and seconds; positions come straight from the entity's
//! [`automata_scene::Transform`], so everything scales with the fixed timestep.

use glam::{Vec2, Vec3};

/// How a rigid body responds to forces and collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BodyType {
    /// Affected by gravity and collisions; moves freely.
    #[default]
    Dynamic,
    /// Immovable and infinite-mass; dynamic bodies bounce off it.
    Static,
    /// Moved by the game (via velocity); pushes dynamic bodies but is not
    /// itself affected by forces.
    KinematicVelocity,
    /// Moved by the game (via transform); pushes dynamic bodies.
    KinematicPosition,
}

/// How a [`CharacterController`] interacts with the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CharacterMode {
    /// Collides with the world and receives collision events.
    #[default]
    Normal,
    /// Collides with the world but does not report the collisions.
    NoEvents,
    /// Passes through the world (ghost mode).
    Ghost,
}

/// Component: attach a rigid body to an entity.
///
/// The body's pose is driven from (dynamic) or by (kinematic) the entity's
/// [`automata_scene::Transform`]. Pair with a [`Collider`] on the same entity to make it
/// collide; a body without a collider still exists but cannot be hit.
///
/// ```rust
/// # use automata_physics::prelude::*;
/// let body = RigidBody::dynamic()
///     .gravity_scale(0.5)
///     .linear_damping(0.2);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RigidBody {
    /// How the body moves.
    pub body_type: BodyType,
    /// Multiplies world gravity. `0.0` = floating, negative = rises.
    pub gravity_scale: f32,
    /// Velocity lost per second, `0.0` disables damping.
    pub linear_damping: f32,
    /// Angular velocity lost per second.
    pub angular_damping: f32,
    /// Set to `false` to exclude this body from the simulation.
    pub enabled: bool,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: BodyType::default(),
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            enabled: true,
        }
    }
}

impl RigidBody {
    /// A free-moving body (affected by gravity).
    #[must_use]
    pub fn dynamic() -> Self {
        Self::default()
    }

    /// An immovable body (floors, walls).
    #[must_use]
    pub fn fixed() -> Self {
        Self {
            body_type: BodyType::Static,
            ..Self::default()
        }
    }

    /// A kinematic body driven by velocity (moving platforms).
    #[must_use]
    pub fn kinematic_velocity() -> Self {
        Self {
            body_type: BodyType::KinematicVelocity,
            ..Self::default()
        }
    }

    /// A kinematic body driven by its transform.
    #[must_use]
    pub fn kinematic_position() -> Self {
        Self {
            body_type: BodyType::KinematicPosition,
            ..Self::default()
        }
    }

    /// Builder-style `gravity_scale` setter.
    #[must_use]
    pub fn gravity_scale(mut self, scale: f32) -> Self {
        self.gravity_scale = scale;
        self
    }

    /// Builder-style `linear_damping` setter.
    #[must_use]
    pub fn linear_damping(mut self, damping: f32) -> Self {
        self.linear_damping = damping;
        self
    }

    /// Builder-style `angular_damping` setter.
    #[must_use]
    pub fn angular_damping(mut self, damping: f32) -> Self {
        self.angular_damping = damping;
        self
    }

    /// Builder-style `enabled` setter.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// A 2D collision shape (used by [`Collider2D`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape2D {
    /// Circle with the given radius.
    Ball {
        /// Radius in metres.
        radius: f32,
    },
    /// Box with the given full extents (width, height).
    Cuboid {
        /// Full size along X and Y.
        extents: Vec2,
    },
    /// Capsule aligned with Y: rectangle of `half_height` with round caps.
    Capsule {
        /// Half the height of the rectangular part.
        half_height: f32,
        /// Radius of the caps.
        radius: f32,
    },
}

/// A 3D collision shape (used by [`Collider3D`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape3D {
    /// Sphere with the given radius.
    Ball {
        /// Radius in metres.
        radius: f32,
    },
    /// Box with the given full extents (width, height, depth).
    Cuboid {
        /// Full size along each axis.
        extents: Vec3,
    },
    /// Capsule aligned with Y: cylinder of `half_height` with round caps.
    Capsule {
        /// Half the height of the cylindrical part.
        half_height: f32,
        /// Radius of the caps and cylinder.
        radius: f32,
    },
}

/// Component: a collider shape. Generic over [`Shape2D`]/[`Shape3D`]; use the
/// [`Collider2D`] / [`Collider3D`] aliases.
///
/// Requires a [`RigidBody`] on the same entity. Shapes are centred on the
/// entity's origin; use `density` to control mass and
/// `friction`/`restitution` for contact response.
///
/// ```rust
/// # use automata_physics::prelude::*;
/// let trigger = Collider3D::ball(1.0).sensor(true).density(0.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Collider<S> {
    /// The collider's shape.
    pub shape: S,
    /// Kg/m³. Sets the collider's mass via its volume.
    pub density: f32,
    /// Coulomb friction, `0.0` (ice) to `1.0+`.
    pub friction: f32,
    /// Bounciness, `0.0` (no bounce) to `1.0` (perfect).
    pub restitution: f32,
    /// Sensors detect overlaps but produce no contact response.
    pub sensor: bool,
    /// Set to `false` to disable collisions for this collider.
    pub enabled: bool,
}

/// Collider with a 2D shape.
pub type Collider2D = Collider<Shape2D>;
/// Collider with a 3D shape.
pub type Collider3D = Collider<Shape3D>;

impl<S> Collider<S> {
    /// A collider of the given shape with default material properties.
    pub fn new(shape: S) -> Self {
        Self {
            shape,
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            sensor: false,
            enabled: true,
        }
    }

    /// Builder-style `density` setter.
    #[must_use]
    pub fn density(mut self, density: f32) -> Self {
        self.density = density;
        self
    }

    /// Builder-style `friction` setter.
    #[must_use]
    pub fn friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    /// Builder-style `restitution` setter.
    #[must_use]
    pub fn restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution;
        self
    }

    /// Builder-style `sensor` setter.
    #[must_use]
    pub fn sensor(mut self, sensor: bool) -> Self {
        self.sensor = sensor;
        self
    }

    /// Builder-style `enabled` setter.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Collider2D {
    /// A circular collider.
    #[must_use]
    pub fn ball(radius: f32) -> Self {
        Self::new(Shape2D::Ball { radius })
    }

    /// A box collider with the given full extents.
    #[must_use]
    pub fn cuboid(x: f32, y: f32) -> Self {
        Self::new(Shape2D::Cuboid {
            extents: Vec2::new(x, y),
        })
    }

    /// A capsule collider with the given half-height and radius.
    #[must_use]
    pub fn capsule(half_height: f32, radius: f32) -> Self {
        Self::new(Shape2D::Capsule {
            half_height,
            radius,
        })
    }
}

impl Collider3D {
    /// A spherical collider.
    #[must_use]
    pub fn ball(radius: f32) -> Self {
        Self::new(Shape3D::Ball { radius })
    }

    /// A box collider with the given full extents.
    #[must_use]
    pub fn cuboid(x: f32, y: f32, z: f32) -> Self {
        Self::new(Shape3D::Cuboid {
            extents: Vec3::new(x, y, z),
        })
    }

    /// A capsule collider with the given half-height and radius.
    #[must_use]
    pub fn capsule(half_height: f32, radius: f32) -> Self {
        Self::new(Shape3D::Capsule {
            half_height,
            radius,
        })
    }
}

/// Component: a kinematic character controller.
///
/// Every physics step the controller computes a collision-resolved movement
/// from the [`CharacterVelocity`] you set and writes the result back into the
/// entity's [`automata_scene::Transform`]. Requires a [`RigidBody`] of type
/// [`BodyType::KinematicVelocity`] (or [`BodyType::KinematicPosition`]) and
/// a [`Collider`] on the same entity.
///
/// ```rust
/// # use automata_physics::prelude::*;
/// let controller = CharacterController::new()
///     .max_slope_climb_angle(0.8)
///     .snap_to_ground(0.4);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterController {
    /// Whether the character collides and/or reports events.
    pub mode: CharacterMode,
    /// Maximum slope angle (radians) the character can climb.
    pub max_slope_climb_angle: f32,
    /// Minimum slope angle (radians) at which the character starts sliding.
    pub min_slope_slide_angle: f32,
    /// Automatically step over obstacles up to this height (None = off).
    pub autostep_height: Option<f32>,
    /// Snap to the ground when within this distance (None = off).
    pub snap_to_ground: Option<f32>,
    /// Small gap kept between the character and its surroundings.
    pub offset: f32,
    /// Slide along surfaces when hitting them.
    pub slide: bool,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            mode: CharacterMode::default(),
            max_slope_climb_angle: 45.0_f32.to_radians(),
            min_slope_slide_angle: 30.0_f32.to_radians(),
            autostep_height: None,
            snap_to_ground: None,
            offset: 0.05,
            slide: true,
        }
    }
}

impl CharacterController {
    /// A controller with sensible humanoid defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder-style `mode` setter.
    #[must_use]
    pub fn mode(mut self, mode: CharacterMode) -> Self {
        self.mode = mode;
        self
    }

    /// Builder-style `max_slope_climb_angle` setter (radians).
    #[must_use]
    pub fn max_slope_climb_angle(mut self, angle: f32) -> Self {
        self.max_slope_climb_angle = angle;
        self
    }

    /// Builder-style `min_slope_slide_angle` setter (radians).
    #[must_use]
    pub fn min_slope_slide_angle(mut self, angle: f32) -> Self {
        self.min_slope_slide_angle = angle;
        self
    }

    /// Builder-style `autostep_height` setter.
    #[must_use]
    pub fn autostep(mut self, height: f32) -> Self {
        self.autostep_height = Some(height);
        self
    }

    /// Builder-style `snap_to_ground` setter.
    #[must_use]
    pub fn snap_to_ground(mut self, distance: f32) -> Self {
        self.snap_to_ground = Some(distance);
        self
    }

    /// Builder-style `offset` setter.
    #[must_use]
    pub fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// Builder-style `slide` setter.
    #[must_use]
    pub fn slide(mut self, slide: bool) -> Self {
        self.slide = slide;
        self
    }
}

/// Component: velocity applied to a kinematic character controller.
///
/// Set this from your game logic; the physics world consumes it, resolves
/// collisions, and writes the result to the [`automata_scene::Transform`]. The value is
/// *not* cleared automatically — set it every frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CharacterVelocity {
    /// Desired velocity in metres per second.
    pub velocity: Vec3,
}

impl CharacterVelocity {
    /// Velocity of `v` metres per second.
    #[must_use]
    pub fn new(velocity: Vec3) -> Self {
        Self { velocity }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_body_builders() {
        let body = RigidBody::dynamic().gravity_scale(0.5).linear_damping(0.1);
        assert_eq!(body.body_type, BodyType::Dynamic);
        assert!((body.gravity_scale - 0.5).abs() < f32::EPSILON);
        assert_eq!(RigidBody::fixed().body_type, BodyType::Static);
        assert_eq!(
            RigidBody::kinematic_position().body_type,
            BodyType::KinematicPosition
        );
    }

    #[test]
    fn collider_builders() {
        let collider = Collider3D::ball(0.5).density(2.0).sensor(true);
        assert_eq!(collider.shape, Shape3D::Ball { radius: 0.5 });
        assert!((collider.density - 2.0).abs() < f32::EPSILON);
        assert!(collider.sensor);

        let cuboid = Collider2D::cuboid(1.0, 2.0);
        assert_eq!(
            cuboid.shape,
            Shape2D::Cuboid {
                extents: Vec2::new(1.0, 2.0)
            }
        );
    }

    #[test]
    fn character_controller_builders() {
        let controller = CharacterController::new()
            .max_slope_climb_angle(0.8)
            .snap_to_ground(0.4)
            .autostep(0.3);
        assert!((controller.max_slope_climb_angle - 0.8).abs() < f32::EPSILON);
        assert_eq!(controller.snap_to_ground, Some(0.4));
        assert_eq!(controller.autostep_height, Some(0.3));
        assert_eq!(controller.mode, CharacterMode::Normal);
    }
}
