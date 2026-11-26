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
//! - [`Grass`] and [`Scatter`] — fields of blades and scattered models,
//!   painted onto the ground.
//! - [`Terrain`] — a heightmap ground, sculpted and painted with brushes.
//!
//! All ECS types needed for common game code (`Entity`, queries) are
//! re-exported; `hecs` remains available for advanced use.

pub mod anim_graph;
mod animation;
mod attach;
mod events;
mod foliage;
mod handle;
pub mod ik;
mod particles;
mod render;
mod resources;
mod scene;
mod springs;
pub mod terrain;
mod transform;

pub use anim_graph::{GraphLayerState, GraphParam, GraphPlayer, GraphState};
pub use animation::{
    animate, animate_on, animation_state, attach_animators, command_animation, Animation,
    AnimationCommand, AnimationState, Animator, CharacterMotion, FiredEvent, Ground, PlayOptions,
    END_EVENT,
};
pub use attach::{attach_to_bones, bone_frame, bone_world, offset_for, Attachment};
pub use automata_renderer::{ParticleBlend, ParticleSprite};
pub use events::Events;
pub use foliage::{
    raycast_surfaces, Dab, Grass, Scatter, SurfaceHit, MAX_AREA, MAX_BLADES, MAX_SCATTERED,
    PAINT_CELL,
};
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
pub use springs::SpringSim;
pub use terrain::{
    default_layers as default_terrain_layers, Sculpt, Terrain, TerrainLayer,
    MAX_TERRAIN_RESOLUTION, MAX_TERRAIN_SIZE, MIN_TERRAIN_RESOLUTION, TERRAIN_LAYERS,
};
pub use transform::{propagate_transforms, GlobalTransform, Parent, Transform};

pub use hecs::Entity;
