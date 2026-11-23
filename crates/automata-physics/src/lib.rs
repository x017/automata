//! Physics for AUTOMATA, built on [Rapier](https://rapier.rs).
//!
//! Phase 6: rigid bodies, colliders, collision events, raycasts, shape
//! casts, and kinematic character controllers — in both 2D and 3D.
//!
//! # Model
//!
//! The physics world mirrors the ECS scene:
//!
//! - Add a [`RigidBody`] (and optionally a [`Collider2D`]/[`Collider3D`]) to
//!   an entity to make it physical.
//! - The engine syncs the world from the scene, steps it at the fixed
//!   timestep, and writes the resulting [`automata_scene::Transform`]s back.
//! - Collision events arrive on the event bus as [`PhysicsEvent`]; contact
//!   forces as [`ContactForce`].
//! - Queries ([`PhysicsWorld3D::cast_ray`], [`PhysicsWorld3D::cast_shape`])
//!   and character controllers ([`CharacterController`] +
//!   [`CharacterVelocity`]) work in both dimensions.
//!
//! The world itself is engine-side: game scripts never see Rapier types.
//!
//! # Example
//!
//! ```rust
//! use automata_physics::prelude::*;
//! use automata_scene::{SceneWorld, Transform};
//!
//! let mut scene = SceneWorld::new();
//! let floor = scene.spawn((
//!     Transform::from_position(glam::Vec3::new(0.0, 0.0, 0.0)),
//!     RigidBody::fixed(),
//!     Collider3D::cuboid(10.0, 1.0, 10.0),
//! ));
//! let ball = scene.spawn((
//!     Transform::from_position(glam::Vec3::new(0.0, 5.0, 0.0)),
//!     RigidBody::dynamic(),
//!     Collider3D::ball(0.5),
//! ));
//!
//! let mut physics = PhysicsWorld3D::new();
//! physics.sync_from_scene(&mut scene);
//! let tick = physics.step(&mut scene, 1.0 / 60.0);
//! assert!(tick.collisions.is_empty()); // ball still falling
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod components;
pub mod events;
pub mod grass;
pub mod model_shape;
pub mod ripples;
pub mod world;
pub mod world2d_impl;
pub mod world3d_impl;

pub use components::{
    BodyType, CharacterController, CharacterMode, CharacterVelocity, Collider, Collider2D,
    Collider3D, RigidBody, Shape2D, Shape3D,
};
pub use events::{ContactForce, PhysicsEvent};
pub use grass::record_grass_pushers;
pub use model_shape::{auto_model_shape, box_fill, collider_problem};
pub use world::{CharacterMove, PhysicsTick, RayFilter, RayHit, WaterSplash};
pub use world2d_impl::PhysicsWorld2D;
pub use world3d_impl::{ColliderSnapshot, PhysicsWorld3D, WorldShape};

/// Everything you need to use physics: `use automata_physics::prelude::*`.
pub mod prelude {
    pub use crate::components::{
        BodyType, CharacterController, CharacterMode, CharacterVelocity, Collider, Collider2D,
        Collider3D, RigidBody, Shape2D, Shape3D,
    };
    pub use crate::events::{ContactForce, PhysicsEvent};
    pub use crate::world::{CharacterMove, PhysicsTick, RayFilter, RayHit, WaterSplash};
    pub use crate::world2d_impl::PhysicsWorld2D;
    pub use crate::world3d_impl::{ColliderSnapshot, PhysicsWorld3D, WorldShape};
}
