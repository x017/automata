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
//! - [`Animation`], [`Animator`] and [`animate`] — clip playback (with
//!   crossfades) for animated models.
//! - [`ParticleEmitter`] and [`simulate_particles`] — CPU particles.
//!
//! All ECS types needed for common game code (`Entity`, queries) are
//! re-exported; `hecs` remains available for advanced use.

mod animation;
mod events;
mod handle;
mod particles;
mod render;
mod resources;
mod scene;
mod transform;

pub use animation::{
    animate, animation_state, attach_animators, command_animation, Animation, AnimationCommand,
    AnimationState, Animator, PlayOptions,
};
pub use automata_renderer::{ParticleBlend, ParticleSprite};
pub use events::Events;
pub use handle::{Assets, Handle};
pub use particles::{
    blend_label, burst, load_particle_textures, preview_particles, simulate_particles, spawn_burst,
    splash_emitter, spray_emitter, EmitterShape, FieldValue as ParticleField, ParticleEmitter,
    ParticleState, ParticleTexture, PRESETS as PARTICLE_PRESETS,
};
pub use render::{
    materialize_visuals, record_draw_list, DirectionalLight, MeshRenderer, MeshVisual,
    MeshVisualKind, PointLight, SpotLight, SpriteRenderer, Surface,
};
pub use resources::Resources;
pub use scene::{EntityId, Name, Scene, SceneWorld};
pub use transform::{propagate_transforms, GlobalTransform, Parent, Transform};

pub use hecs::Entity;
