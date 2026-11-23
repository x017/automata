//! The scene-to-renderer bridge: renderable components and the system that
//! records a scene into the frame's [`DrawList`].
//!
//! Games attach [`MeshRenderer`] / [`SpriteRenderer`] components to
//! entities; [`record_draw_list`] walks the world once per frame and emits
//! draw commands using each entity's [`GlobalTransform`]. The first entity
//! carrying a [`Camera3D`] or [`Camera2D`] component becomes the frame's
//! camera, the first [`DirectionalLight`] becomes the frame's sun, and the
//! first [`Environment`] sets the frame's look (ambient, fog, tone mapping).
//! Every [`PointLight`] and [`SpotLight`] lights the frame too (the nearest
//! and brightest, up to the quality preset's cap).

use automata_renderer::{
    Camera2D, Camera3D, DrawList, Environment, Material, Mesh, MeshData, Sprite, Sun, Texture,
    Water,
};
use std::sync::Arc;

use automata_assets::Model;
use glam::{EulerRot, Vec2};

use crate::scene::SceneWorld;
use crate::transform::GlobalTransform;

/// Renders a 3D mesh with a material, plus any extra parts (a model with
/// several materials draws its first part as `mesh` and the rest from
/// `parts`, all with the entity's transform).
#[derive(Debug, Clone)]
pub struct MeshRenderer {
    /// The mesh to draw.
    pub mesh: Mesh,
    /// The material to draw it with.
    pub material: Material,
    /// More meshes drawn with the same transform, each with its own
    /// material.
    pub parts: Vec<(Mesh, Material)>,
    /// The joint palette posing skinned meshes (see
    /// [`automata_renderer::DrawList::skinned_mesh`]); set every frame by
    /// [`crate::animate`]. Skinned meshes need one to draw correctly.
    pub joints: Option<Arc<[glam::Mat4]>>,
    /// Levels of detail and draw distance per mesh: `[0]` for `mesh`,
    /// then one per part. Missing entries (or `None`) draw at full detail
    /// at any distance.
    pub lods: Vec<Option<Arc<automata_renderer::MeshLod>>>,
}

impl MeshRenderer {
    /// Creates a mesh renderer.
    #[must_use]
    pub fn new(mesh: Mesh, material: Material) -> Self {
        Self {
            mesh,
            material,
            parts: Vec::new(),
            joints: None,
            lods: Vec::new(),
        }
    }

    /// Adds a part drawn with the same transform.
    #[must_use]
    pub fn with_part(mut self, mesh: Mesh, material: Material) -> Self {
        self.parts.push((mesh, material));
        self
    }
}

/// The scene's directional sun: parallel light rays plus ambient fill.
///
/// The first entity carrying one becomes the frame's light in
/// [`record_draw_list`]. Enable `shadows` for shadow-mapped sunlight.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectionalLight {
    /// The direction the light travels (normalized on use).
    pub direction: glam::Vec3,
    /// Light color (linear RGB).
    pub color: [f32; 3],
    /// Ambient light fraction, 0.0..=1.0.
    pub ambient: f32,
    /// Whether this light casts shadows.
    pub shadows: bool,
    /// How far from the camera shadows reach (metres); they fade out
    /// beyond. Shorter is sharper.
    #[serde(
        default = "default_shadow_distance",
        skip_serializing_if = "is_default_shadow_distance"
    )]
    pub shadow_distance: f32,
    /// Shadow edge softness: 1 the default, 0 as sharp as possible, up to
    /// 8.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub shadow_softness: f32,
}

fn default_shadow_distance() -> f32 {
    automata_renderer::DEFAULT_SHADOW_DISTANCE
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_default_shadow_distance(v: &f32) -> bool {
    *v == automata_renderer::DEFAULT_SHADOW_DISTANCE
}

fn one() -> f32 {
    1.0
}

impl DirectionalLight {
    /// Creates a light shining along `direction`, with shadows on.
    #[must_use]
    pub fn new(direction: glam::Vec3) -> Self {
        Self {
            direction,
            color: [1.0, 1.0, 1.0],
            ambient: 0.3,
            shadows: true,
            shadow_distance: automata_renderer::DEFAULT_SHADOW_DISTANCE,
            shadow_softness: 1.0,
        }
    }
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self::new(glam::Vec3::new(-0.5, -1.0, -0.35))
    }
}

impl From<DirectionalLight> for Sun {
    fn from(light: DirectionalLight) -> Self {
        Sun {
            direction: light.direction,
            color: light.color,
            ambient: light.ambient,
            shadows: light.shadows,
            shadow_distance: light.shadow_distance,
            shadow_softness: light.shadow_softness,
        }
    }
}

/// A light placed in the scene that shines in every direction from the
/// entity's position: a torch, a lamp, a campfire. Its light fades with the
/// square of the distance and reaches nothing past `range`.
///
/// Brightness: `intensity` 1 lights a surface 1 m away, facing it, as
/// brightly as a white sun shining straight down; at 2 m it is a quarter
/// of that.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PointLight {
    /// Light color (linear RGB, 0..1).
    pub color: [f32; 3],
    /// Brightness (see the type's docs).
    pub intensity: f32,
    /// Where the light ends (metres); 0 picks it from the brightness: where
    /// it has faded to almost nothing.
    #[serde(skip_serializing_if = "is_zero")]
    pub range: f32,
    /// The size of the glowing source (metres): bigger is softer up close.
    #[serde(skip_serializing_if = "is_default_light_radius")]
    pub radius: f32,
    /// Whether objects block it and cast shadows. Only the nearest few
    /// shadowed lights get shadows (the quality preset sets how many; none
    /// on Low and Medium).
    #[serde(skip_serializing_if = "is_true")]
    pub shadows: bool,
}

impl PointLight {
    /// A white light of `intensity`, reaching as far as its brightness
    /// carries, casting shadows.
    #[must_use]
    pub fn new(intensity: f32) -> Self {
        Self {
            color: [1.0; 3],
            intensity,
            range: 0.0,
            radius: DEFAULT_LIGHT_RADIUS,
            shadows: true,
        }
    }

    /// The renderer's light at `position`.
    #[must_use]
    pub fn to_light(&self, position: glam::Vec3) -> automata_renderer::PointLight {
        automata_renderer::PointLight {
            radius: self.radius.max(0.01),
            range: self.range.max(0.0),
            shadows: self.shadows,
            ..automata_renderer::PointLight::new(position, scaled(self.color, self.intensity))
        }
    }
}

impl Default for PointLight {
    /// A warm-white light a few metres across, like a lamp.
    fn default() -> Self {
        Self {
            color: [1.0, 0.85, 0.65],
            ..Self::new(3.0)
        }
    }
}

/// A light placed in the scene that shines in a cone: a flashlight, a car
/// headlight, a stage light. It shines along the entity's forward
/// direction (its local -Z, turned by its rotation), at full strength
/// inside `inner_angle` and fading out by `outer_angle`.
///
/// Brightness works as for [`PointLight`]: `intensity` 1 at 1 m on the
/// axis matches a white sun.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpotLight {
    /// Light color (linear RGB, 0..1).
    pub color: [f32; 3],
    /// Brightness on the cone's axis.
    pub intensity: f32,
    /// Where the light ends (metres); 0 picks it from the brightness.
    #[serde(skip_serializing_if = "is_zero")]
    pub range: f32,
    /// Angle from the axis (degrees) inside which it is at full strength.
    pub inner_angle: f32,
    /// Angle from the axis (degrees) of the cone's edge, where it has
    /// faded out (at most 80).
    pub outer_angle: f32,
    /// The size of the glowing source (metres).
    #[serde(skip_serializing_if = "is_default_light_radius")]
    pub radius: f32,
    /// Whether objects block it and cast shadows (a spot's shadow costs a
    /// sixth of a point light's).
    #[serde(skip_serializing_if = "is_true")]
    pub shadows: bool,
}

impl SpotLight {
    /// A white spot of `intensity` whose cone's edge is `outer_angle`
    /// degrees from its axis, fading over the outer quarter.
    #[must_use]
    pub fn new(intensity: f32, outer_angle: f32) -> Self {
        Self {
            color: [1.0; 3],
            intensity,
            range: 0.0,
            inner_angle: outer_angle * 0.75,
            outer_angle,
            radius: DEFAULT_LIGHT_RADIUS,
            shadows: true,
        }
    }

    /// The renderer's light at `position`, shining along `direction`.
    #[must_use]
    pub fn to_light(
        &self,
        position: glam::Vec3,
        direction: glam::Vec3,
    ) -> automata_renderer::PointLight {
        automata_renderer::PointLight {
            radius: self.radius.max(0.01),
            range: self.range.max(0.0),
            shadows: self.shadows,
            spot: Some(automata_renderer::SpotCone {
                direction,
                inner_angle: self.inner_angle,
                outer_angle: self.outer_angle,
            }),
            ..automata_renderer::PointLight::new(position, scaled(self.color, self.intensity))
        }
    }
}

impl Default for SpotLight {
    /// A bright white 30° flashlight.
    fn default() -> Self {
        Self::new(8.0, 30.0)
    }
}

/// A placed light's source size unless set.
const DEFAULT_LIGHT_RADIUS: f32 = 0.05;

fn scaled(color: [f32; 3], intensity: f32) -> [f32; 3] {
    color.map(|c| c.max(0.0) * intensity.max(0.0))
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_default_light_radius(v: &f32) -> bool {
    *v == DEFAULT_LIGHT_RADIUS
}

/// Renders a textured 2D quad.
///
/// The sprite is centered on the entity's position; the entity's Z rotation
/// rotates it, and its scale multiplies `size`.
#[derive(Debug, Clone)]
pub struct SpriteRenderer {
    /// The texture to sample.
    pub texture: Texture,
    /// Unscaled size in the 2D camera's units.
    pub size: Vec2,
    /// Tint color, multiplied with the texture.
    pub color: [f32; 4],
    /// Texture coordinate window `(u0, v0, u1, v1)`.
    pub uv: [f32; 4],
}

impl SpriteRenderer {
    /// Creates a sprite renderer showing the full texture, untinted, at the
    /// texture's pixel size.
    #[must_use]
    pub fn new(texture: Texture) -> Self {
        let (w, h) = texture.size();
        Self {
            texture,
            size: Vec2::new(w as f32, h as f32),
            color: [1.0, 1.0, 1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// Sets the unscaled size.
    #[must_use]
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    /// Sets the tint color.
    #[must_use]
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    /// Sets the texture coordinate window.
    #[must_use]
    pub fn uv(mut self, uv: [f32; 4]) -> Self {
        self.uv = uv;
        self
    }
}

/// Records a scene into a draw list: cameras, then meshes, then sprites.
///
/// Entities without a [`GlobalTransform`] are skipped — either their
/// [`crate::Transform`] hasn't propagated yet (the engine propagates once
/// per frame) or they were never meant to render.
pub fn record_draw_list(world: &SceneWorld, draw: &mut DrawList) {
    // Cameras: the first entity with each camera component wins.
    if let Some(camera) = (&mut world.query::<&Camera3D>()).into_iter().next() {
        draw.set_camera_3d(*camera);
    }
    if let Some(camera) = (&mut world.query::<&Camera2D>()).into_iter().next() {
        draw.set_camera_2d(*camera);
    }
    // Lighting: the first entity with a directional light becomes the sun.
    if let Some(light) = (&mut world.query::<&DirectionalLight>()).into_iter().next() {
        draw.set_sun((*light).into());
    }
    // Look: the first entity with an environment sets ambient, fog, tone
    // mapping, and the background color.
    if let Some(env) = (&mut world.query::<&Environment>()).into_iter().next() {
        draw.set_environment(*env);
        draw.set_clear_color(env.clear_color());
    }

    for (renderer, global, scatter) in
        &mut world.query::<(&MeshRenderer, &GlobalTransform, Option<&crate::Scatter>)>()
    {
        let model = global.matrix();
        let mut record = |model: glam::Mat4| {
            let parts = std::iter::once((&renderer.mesh, &renderer.material))
                .chain(renderer.parts.iter().map(|(m, mat)| (m, mat)));
            for (i, (mesh, material)) in parts.enumerate() {
                match &renderer.joints {
                    Some(joints) => draw.skinned_mesh(mesh, model, material, joints),
                    None => draw.mesh(mesh, model, material),
                }
                if let Some(Some(lod)) = renderer.lods.get(i) {
                    draw.set_lod(lod);
                }
            }
        };
        // A scattered model draws at every spot it's scattered to instead.
        match scatter {
            Some(scatter) => {
                for spot in scatter.placements().iter() {
                    record(model * *spot);
                }
            }
            None => record(model),
        }
    }
    crate::foliage::record_grass(world, draw);

    // Placed lights: at the entity's position; spots shine along its
    // forward (-Z) axis.
    for (light, global) in &mut world.query::<(&PointLight, &GlobalTransform)>() {
        draw.point_light(light.to_light(global.matrix().w_axis.truncate()));
    }
    for (light, global) in &mut world.query::<(&SpotLight, &GlobalTransform)>() {
        let m = global.matrix();
        draw.point_light(light.to_light(m.w_axis.truncate(), -m.z_axis.truncate()));
    }

    // Water surfaces: centred on their transform, waves in its local XZ.
    for (water, global) in &mut world.query::<(&Water, &GlobalTransform)>() {
        draw.water(*water, global.matrix());
    }

    crate::particles::record_particles(world, draw);

    for (renderer, global) in &mut world.query::<(&SpriteRenderer, &GlobalTransform)>() {
        let (scale, rotation, translation) = global.matrix().to_scale_rotation_translation();
        let (z_rotation, _, _) = rotation.to_euler(EulerRot::ZYX);
        draw.sprite(
            Sprite::new(renderer.texture.clone())
                .position(translation.truncate())
                .size(renderer.size * scale.truncate())
                .rotation(z_rotation)
                .color(renderer.color)
                .uv(renderer.uv),
        );
    }
}

/// A plain-data request to render a built-in primitive mesh with a flat
/// color.
///
/// Unlike [`MeshRenderer`], this component carries no GPU handles, so
/// games (and scripting bindings) can stage it before a renderer exists.
/// The engine's windowed loop materializes it into a real [`MeshRenderer`]
/// on the first frame via [`materialize_visuals`]; headless runs leave it
/// untouched. The entity's [`crate::Transform`] scale sizes the primitive.
///
/// A model visual ([`MeshVisual::model`]) instead references a model file;
/// its parts keep their own colors and textures, tinted by `color`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeshVisual {
    /// Which primitive (or custom geometry) `data` holds. Kept alongside
    /// the baked mesh so scene files can record `Cube(size: 1.0)` instead
    /// of 24 raw vertices.
    pub kind: MeshVisualKind,
    /// The primitive's geometry (empty for models).
    pub data: MeshData,
    /// Flat base color, RGBA. For models, a tint multiplied with every
    /// part's own color.
    pub color: [f32; 4],
    /// The loaded model, for [`MeshVisualKind::Model`] (shared through the
    /// asset cache). `None` when the file failed to load.
    #[serde(skip)]
    pub model: Option<Arc<Model>>,
    /// Roughness, metalness and glow. Unset values keep the default
    /// surface (primitives) or the file's own materials (models).
    #[serde(default, skip_serializing_if = "Surface::is_default")]
    pub surface: Surface,
    /// Beyond this camera distance (metres) it isn't drawn, shadow
    /// included; over the last tenth it dithers away. 0 draws it at any
    /// distance.
    #[serde(default)]
    pub draw_distance: f32,
}

/// How a [`MeshVisual`]'s surface responds to light, on top of its color.
/// Every field is optional: unset, a primitive gets the plain default
/// (roughness 0.6, non-metal, no glow) and a model keeps what its file
/// says, per part.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Surface {
    /// 0 = mirror-smooth, 1 = chalky. Replaces every part's roughness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
    /// 0 = plastic, wood, stone; 1 = bare metal. Replaces every part's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metallic: Option<f32>,
    /// Light the surface gives off, RGB 0..1 (times `emissive_strength`).
    /// Added to a model's own glow.
    #[serde(skip_serializing_if = "is_black")]
    pub emissive: [f32; 3],
    /// Multiplier on `emissive`; above 1 it glows (and blooms on
    /// Beautiful).
    #[serde(skip_serializing_if = "is_one")]
    pub emissive_strength: f32,
    /// How much the glow lights the surroundings: 1 as a real glowing
    /// object of this size and brightness would, 0 not at all, 2 twice as
    /// much. Applies to a model's own glow too.
    #[serde(skip_serializing_if = "is_one")]
    pub glow_light: f32,
    /// Whether objects block that light, casting shadows from it (on the
    /// presets that shadow point lights: High and up).
    #[serde(skip_serializing_if = "is_true")]
    pub glow_shadows: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_true(v: &bool) -> bool {
    *v
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_black(c: &[f32; 3]) -> bool {
    *c == [0.0; 3]
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_one(v: &f32) -> bool {
    *v == 1.0
}

impl Surface {
    /// Nothing overridden.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The emitted light: `emissive × emissive_strength`.
    #[must_use]
    pub fn glow(&self) -> [f32; 3] {
        self.emissive.map(|c| c * self.emissive_strength)
    }

    /// The renderer parameters for a primitive of this `color`.
    #[must_use]
    pub fn params(&self, color: [f32; 4]) -> automata_renderer::MaterialParams {
        automata_renderer::MaterialParams {
            color,
            roughness: self
                .roughness
                .unwrap_or(automata_renderer::DEFAULT_ROUGHNESS),
            metallic: self.metallic.unwrap_or(0.0),
            emissive: self.glow(),
            normal_scale: 1.0,
            glow_light: self.glow_light.max(0.0),
            glow_shadows: self.glow_shadows,
        }
    }

    /// The renderer parameters for a model part: the part's own surface
    /// with these overrides applied, its color tinted by `tint`.
    #[must_use]
    pub fn part_params(
        &self,
        part: &automata_assets::ModelPart,
        tint: [f32; 4],
    ) -> automata_renderer::MaterialParams {
        let own = &part.surface;
        let glow = self.glow();
        automata_renderer::MaterialParams {
            color: std::array::from_fn(|i| part.color[i] * tint[i]),
            roughness: self.roughness.unwrap_or(own.roughness),
            metallic: self.metallic.unwrap_or(own.metallic),
            emissive: std::array::from_fn(|i| own.emissive[i] + glow[i]),
            normal_scale: own.normal_scale,
            // The shader multiplies all the glow by the emissive map, so
            // the light follows how much of the map is lit.
            glow_light: self.glow_light.max(0.0) * own.emissive_coverage,
            glow_shadows: self.glow_shadows,
        }
    }
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            roughness: None,
            metallic: None,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            glow_light: 1.0,
            glow_shadows: true,
        }
    }
}

/// The geometry source a [`MeshVisual`] was built from.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MeshVisualKind {
    /// `MeshData::cube(size)`.
    Cube {
        /// Side length before scaling.
        size: f32,
    },
    /// `MeshData::quad(size)`.
    Quad {
        /// Side length before scaling.
        size: f32,
    },
    /// `MeshData::sphere(size)`.
    Sphere {
        /// Diameter before scaling.
        size: f32,
    },
    /// Hand-built geometry, stored verbatim.
    Custom,
    /// A model file (`.glb`, `.gltf` or `.obj`), relative to the asset root
    /// (normally the scene's folder).
    Model {
        /// The file path, `/`-separated.
        path: String,
    },
}

impl MeshVisual {
    /// A cube centered on the entity, `size` units on a side before scaling.
    #[must_use]
    pub fn cube(size: f32, color: [f32; 4]) -> Self {
        Self {
            kind: MeshVisualKind::Cube { size },
            data: MeshData::cube(size),
            color,
            model: None,
            surface: Surface::default(),
            draw_distance: 0.0,
        }
    }

    /// A quad centered on the entity, facing +Z, `size` units on a side
    /// before scaling.
    #[must_use]
    pub fn quad(size: f32, color: [f32; 4]) -> Self {
        Self {
            kind: MeshVisualKind::Quad { size },
            data: MeshData::quad(size),
            color,
            model: None,
            surface: Surface::default(),
            draw_distance: 0.0,
        }
    }

    /// A sphere centered on the entity, `size` units across before scaling.
    #[must_use]
    pub fn sphere(size: f32, color: [f32; 4]) -> Self {
        Self {
            kind: MeshVisualKind::Sphere { size },
            data: MeshData::sphere(size),
            color,
            model: None,
            surface: Surface::default(),
            draw_distance: 0.0,
        }
    }

    /// Hand-built geometry with a flat color.
    #[must_use]
    pub fn custom(data: MeshData, color: [f32; 4]) -> Self {
        Self {
            kind: MeshVisualKind::Custom,
            data,
            color,
            model: None,
            surface: Surface::default(),
            draw_distance: 0.0,
        }
    }

    /// This visual with a different [`Surface`].
    #[must_use]
    pub fn with_surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }

    /// This visual with a draw distance (see [`Self::draw_distance`]).
    #[must_use]
    pub fn with_draw_distance(mut self, draw_distance: f32) -> Self {
        self.draw_distance = draw_distance;
        self
    }

    /// A model file, tinted by `tint` (white keeps the file's colors).
    ///
    /// # Errors
    ///
    /// Fails when the file is missing or can't be read as a model.
    pub fn try_model(path: &str, tint: [f32; 4]) -> Result<Self, automata_assets::ModelError> {
        let model = automata_assets::load(path)?;
        Ok(Self::model_with(path, tint, Some(model)))
    }

    /// A model file, tinted by `tint`. A file that fails to load is logged
    /// and draws nothing (the reference is kept, so fixing the file and
    /// reloading brings it back).
    #[must_use]
    pub fn model(path: &str, tint: [f32; 4]) -> Self {
        match automata_assets::load(path) {
            Ok(model) => Self::model_with(path, tint, Some(model)),
            Err(e) => {
                tracing::warn!("{e}");
                Self::model_with(path, tint, None)
            }
        }
    }

    fn model_with(path: &str, tint: [f32; 4], model: Option<Arc<Model>>) -> Self {
        Self {
            kind: MeshVisualKind::Model {
                path: path.replace('\\', "/"),
            },
            data: MeshData::new(Vec::new(), Vec::new()),
            color: tint,
            model,
            surface: Surface::default(),
            draw_distance: 0.0,
        }
    }
}

/// Entities carrying a [`MeshVisual`] that has not been materialized yet
/// (no [`MeshRenderer`] attached). Models that failed to load stay pending
/// but are skipped cheaply.
fn pending_visuals(world: &SceneWorld) -> Vec<hecs::Entity> {
    world
        .query::<hecs::Entity>()
        .iter()
        .filter(|&entity| {
            world.get::<&MeshVisual>(entity).is_ok() && world.get::<&MeshRenderer>(entity).is_err()
        })
        .collect()
}

/// Materializes every pending [`MeshVisual`] into a GPU-backed
/// [`MeshRenderer`]. Called once per frame by the windowed loop before
/// [`record_draw_list`]; returns how many visuals were materialized.
pub fn materialize_visuals(
    world: &mut SceneWorld,
    renderer: &automata_renderer::Renderer,
) -> usize {
    let mut materialized = 0;
    for entity in pending_visuals(world) {
        // Scope the hecs Ref borrow: it has a Drop impl, so without the
        // block it would live until the end of the loop body and conflict
        // with the mutable insert below.
        let visual = {
            let Ok(visual) = world.get::<&MeshVisual>(entity) else {
                continue;
            };
            if visual.model.is_none() && matches!(visual.kind, MeshVisualKind::Model { .. }) {
                continue; // the file failed to load: nothing to draw
            }
            // The hecs `Ref` must drop before the insert below, so bind the
            // clone first (returning it directly keeps the borrow alive).
            #[allow(clippy::let_and_return)]
            let cloned = MeshVisual::clone(&visual);
            cloned
        };
        let component = if let Some(model) = &visual.model {
            let tint = visual.color;
            let mut parts = model
                .gpu(renderer)
                .iter()
                .zip(&model.parts)
                .map(|(gpu, part)| {
                    let params = visual.surface.part_params(part, tint);
                    // Rigged parts draw skinned, posed by the joints below.
                    let mesh = if model.rig.is_some() {
                        gpu.skinned.as_ref().unwrap_or(&gpu.mesh)
                    } else {
                        &gpu.mesh
                    };
                    (
                        mesh.clone(),
                        renderer.create_pbr_material(
                            &params,
                            automata_renderer::MaterialTextures {
                                base: gpu.texture.clone(),
                                ..gpu.maps.clone()
                            },
                        ),
                    )
                });
            let Some((mesh, material)) = parts.next() else {
                continue;
            };
            let mut component = MeshRenderer::new(mesh, material);
            component.parts.extend(parts);
            // One shared level list per part (drawn for every copy).
            let draw_distance = visual.draw_distance;
            component.lods = model
                .gpu(renderer)
                .iter()
                .map(|gpu| {
                    (!gpu.lods.is_empty() || draw_distance > 0.0).then(|| {
                        Arc::new(automata_renderer::MeshLod {
                            levels: if model.rig.is_some() {
                                Vec::new()
                            } else {
                                gpu.lods.clone()
                            },
                            draw_distance,
                        })
                    })
                })
                .collect();
            // An Animator that already ran keeps its pose; otherwise the
            // rest pose until one poses it.
            let posed = world
                .get::<&crate::animation::Animator>(entity)
                .ok()
                .map(|animator| animator.palette());
            component.joints =
                posed.or_else(|| model.rig.as_ref().map(|rig| rig.rest_palette().into()));
            component
        } else {
            let mesh = renderer.create_mesh(&visual.data);
            let material = renderer.create_pbr_material(
                &visual.surface.params(visual.color),
                automata_renderer::MaterialTextures::default(),
            );
            let mut component = MeshRenderer::new(mesh, material);
            if visual.draw_distance > 0.0 {
                component.lods = vec![Some(Arc::new(automata_renderer::MeshLod {
                    levels: Vec::new(),
                    draw_distance: visual.draw_distance,
                }))];
            }
            component
        };
        if world.insert_one(entity, component).is_ok() {
            materialized += 1;
        }
    }
    materialized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;
    use glam::Vec3;

    #[test]
    fn pending_visuals_lists_only_unmaterialized_visual_entities() {
        let mut scene = Scene::new();
        let visual = scene.spawn((MeshVisual::cube(1.0, [1.0, 0.0, 0.0, 1.0]),));
        let _camera = scene.spawn((Camera3D::new(Vec3::ZERO, Vec3::Z),));

        assert_eq!(pending_visuals(scene.world()), vec![visual]);

        // Despawned entities drop out of the pending list.
        scene.despawn(visual).unwrap();
        let pending_after = pending_visuals(scene.world());
        assert_eq!(pending_after, [] as [hecs::Entity; 0]);
    }

    #[test]
    fn placed_lights_shine_from_their_transform() {
        let mut scene = Scene::new();
        scene.spawn((
            PointLight::new(2.0),
            GlobalTransform(glam::Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))),
        ));
        // Tipped forward 90° about X: its -Z forward points straight down.
        scene.spawn((
            SpotLight {
                color: [1.0, 0.5, 0.0],
                shadows: false,
                ..SpotLight::new(4.0, 20.0)
            },
            GlobalTransform(glam::Mat4::from_rotation_translation(
                glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                Vec3::new(0.0, 5.0, 0.0),
            )),
        ));
        // No transform: not drawn.
        scene.spawn((PointLight::default(),));
        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);
        let lights = draw.point_lights();
        assert_eq!(lights.len(), 2);
        let point = lights.iter().find(|l| l.spot.is_none()).unwrap();
        assert_eq!(point.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(point.intensity, [2.0; 3]);
        assert!(point.shadows);
        let spot = lights.iter().find(|l| l.spot.is_some()).unwrap();
        assert_eq!(spot.intensity, [4.0, 2.0, 0.0]);
        assert!(!spot.shadows);
        let cone = spot.spot.unwrap();
        assert!((cone.direction - Vec3::NEG_Y).length() < 1e-5, "{cone:?}");
        assert!((cone.inner_angle - 15.0).abs() < 1e-5);
        assert!((cone.outer_angle - 20.0).abs() < 1e-5);
    }

    #[test]
    fn camera_components_set_the_frame_cameras() {
        let mut scene = Scene::new();
        scene.spawn((Camera3D::new(Vec3::new(0.0, 2.0, 8.0), Vec3::ZERO),));
        scene.spawn((Camera2D::new(Vec2::ZERO, 100.0),));

        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);

        // Cameras were adopted by the draw list.
        assert!(draw.camera_2d().is_some());
        assert!(draw.camera_3d().is_some());
        assert_eq!(draw.camera_3d().unwrap().position, Vec3::new(0.0, 2.0, 8.0));
    }

    #[test]
    fn environment_component_sets_the_frame_look() {
        let mut scene = Scene::new();
        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);
        assert!(draw.environment().is_none());
        scene.spawn((Environment::night(),));
        record_draw_list(scene.world(), &mut draw);
        assert_eq!(draw.environment(), Some(Environment::night()));
    }

    #[test]
    fn water_component_records_a_water_draw() {
        let mut scene = Scene::new();
        scene.spawn((Water::pond(),));
        scene.spawn((Water::ocean(), crate::Transform::default()));
        crate::transform::propagate_transforms(scene.world_mut());
        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);
        assert_eq!(draw.water_count(), 1, "only water with a transform draws");
    }

    #[test]
    fn entities_without_global_transform_are_skipped() {
        // No Transform -> no GlobalTransform -> nothing recorded. The
        // renderable components need GPU handles, so this test only checks
        // the empty-scene behavior and the camera path above; the full
        // render path is covered by the renderer crate's GPU tests.
        let scene = Scene::new();
        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);
        assert_eq!(draw.sprite_count(), 0);
        assert_eq!(draw.mesh_count(), 0);
    }

    #[test]
    fn directional_light_component_sets_the_frame_sun() {
        let mut scene = Scene::new();
        let light = DirectionalLight {
            direction: Vec3::new(0.0, -1.0, 0.0),
            color: [1.0, 0.9, 0.8],
            ambient: 0.4,
            shadow_distance: 50.0,
            shadow_softness: 2.0,
            ..DirectionalLight::default()
        };
        scene.spawn((light,));

        let mut draw = DrawList::new();
        record_draw_list(scene.world(), &mut draw);

        let sun = draw.sun().expect("light should set the frame sun");
        assert_eq!(sun.direction, Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(sun.color, [1.0, 0.9, 0.8]);
        assert!(sun.shadows);
        assert_eq!((sun.shadow_distance, sun.shadow_softness), (50.0, 2.0));

        // No light entity -> no sun override on the draw list.
        let empty = Scene::new();
        let mut draw = DrawList::new();
        record_draw_list(empty.world(), &mut draw);
        assert!(draw.sun().is_none());
    }
}
