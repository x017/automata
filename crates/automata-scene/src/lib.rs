//! AUTOMATA scene system.
//!
//! Phase 4 scope: scene management, entities and components (via the `hecs`
//! ECS — see the evaluation note in the crate README/docs), transform
//! hierarchies, resource handles and registries, and event dispatching.
//!
//! Core pieces:
//!
//! - [`Scene`] — the ECS world plus hierarchy and lifecycle helpers.
//! - [`Transform`] / [`GlobalTransform`] / [`Parent`] — transform hierarchy.
//! - [`Events`] — frame-scoped, type-indexed event queue.
//! - [`Resources`] — type-indexed shared state.
//! - [`Handle`] / [`Assets`] — generational resource handles and registries.
//! - [`MeshRenderer`] / [`SpriteRenderer`] and [`record_draw_list`] — the
//!   scene-to-renderer bridge.
//!
//! All ECS types needed for common game code (`Entity`, queries) are
//! re-exported; `hecs` remains available for advanced use.

mod events;
mod handle;
mod render;
mod resources;
mod scene;
mod transform;

pub use events::Events;
pub use handle::{Assets, Handle};
pub use render::{record_draw_list, MeshRenderer, SpriteRenderer};
pub use resources::Resources;
pub use scene::{EntityId, Name, Scene, SceneWorld};
pub use transform::{propagate_transforms, GlobalTransform, Parent, Transform};

pub use hecs::Entity;
