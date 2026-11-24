//! The [`Renderer`]: pipelines, GPU resource factories, and frame execution.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;
use wgpu::util::DeviceExt;

use crate::camera;
use crate::composite::Compositor;
use crate::draw::{DrawList, MeshDraw, Sprite};
use crate::error::Result;
use crate::light::{FrameView, LightUniform};
use crate::material::{Material, MaterialInner, MaterialUniform};
use crate::mesh::{Mesh, MeshData, MeshInner, SkinVertex};
use crate::pipeline::{self, PipelineShared, SpriteVertex};
use crate::shadow::{ShadowCascades, Sphere};
use crate::texture::{Texture, TextureInner};
use crate::water::WaterUniform;
use crate::{GraphicsQuality, RenderContext, Sun};

/// Depth buffer format used by the mesh pass.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// Color format of offscreen render targets ([`Renderer::create_render_target`]).
/// Always RGBA8 sRGB so CPU readback is plain RGBA regardless of the window
/// surface's native format.
pub(crate) const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Where a frame is rendered: the window surface or an offscreen texture.
enum FrameTarget {
    /// Windowed: the surface texture is presented after rendering.
    Surface(wgpu::SurfaceTexture),
    /// Offscreen: a regular texture view owned by the caller, and its size.
    Offscreen {
        view: wgpu::TextureView,
        size: (u32, u32),
    },
}

/// Whether the frame draws the sky dome: a 3D frame whose environment
/// asks for one, or whose camera is in water (the sky pass then fills the
/// empty background with the water's colour).
fn draws_sky(draw: &DrawList) -> bool {
    draw.camera_3d.is_some() && (draw.environment.is_some_and(|e| e.sky) || camera_in_water(draw))
}

/// Whether the frame's 3D camera is in (or at the surface of) water.
fn camera_in_water(draw: &DrawList) -> bool {
    draw.camera_3d.is_some_and(|cam| {
        !draw.waters.is_empty()
            && crate::water::WaterView::new(&draw.waters, &cam, draw.time.unwrap_or(0.0), false)
                .camera_in_water()
    })
}

/// Per-frame GPU resources derived from a draw list.
struct PreparedFrame {
    camera_2d_bind: wgpu::BindGroup,
    camera_3d_bind: wgpu::BindGroup,
    /// Per mesh draw: the skinned ones' model and joint bind groups.
    model_bind_groups: Vec<Option<wgpu::BindGroup>>,
    /// Per mesh draw: what the camera sees of it (`None`: past its draw
    /// distance, so drawn nowhere).
    seen: Vec<Option<crate::batch::Seen>>,
    /// The camera's batches (culled to its view), and each sun shadow
    /// cascade's, over [`Renderer::instance_buffer`].
    main_batches: Vec<crate::batch::Batch>,
    cascade_batches: Vec<Vec<crate::batch::Batch>>,
    sprite_runs: Vec<SpriteRun>,
    light_bind: wgpu::BindGroup,
    /// The light group plus the screen-space occlusion (Beautiful meshes).
    beautiful_light_bind: Option<wgpu::BindGroup>,
    shadows: ShadowCascades,
    /// World bounding sphere per mesh (shadow culling).
    mesh_spheres: Vec<Sphere>,
    render_shadows: bool,
    /// The point lights shaded this frame (with their shadow slots).
    points: Vec<crate::light::FramePoint>,
    /// One bind group per water draw.
    water_binds: Vec<(wgpu::BindGroup, u32)>,
    /// The particle batches, uploaded in draw order.
    particle_runs: Vec<crate::particles::ParticleRun>,
    /// Per grass field the camera sees, its material (grass batches'
    /// `draw` indexes this).
    grass_materials: Vec<Material>,
    /// The frame's 3D camera.
    camera_3d: crate::Camera3D,
}

impl PreparedFrame {
    /// The shadows the forward path samples this frame: the sun's when it
    /// renders them, point lights' when any got a shadow slot.
    fn forward_shadows(&self) -> pipeline::ForwardShadows {
        pipeline::ForwardShadows {
            sun: self.render_shadows,
            points: self.points.iter().any(|p| p.shadow.is_some()),
        }
    }
}

/// A complete renderer: GPU context, pipelines, and resource factories.
///
/// Construct with [`Renderer::new`] for windowed use; the application engine
/// drives one frame by handing it a recorded [`DrawList`] via
/// [`Renderer::render`].
pub struct Renderer {
    ctx: RenderContext,
    shared: PipelineShared,
    sprite_pipeline: wgpu::RenderPipeline,
    /// Forward-path mesh pipelines by target format and the shadows they
    /// sample: every mix the preset can use, so a frame switching between
    /// them (a shadowed lamp coming into view) never waits on a compile.
    forward_pipelines: Vec<ForwardPipelines>,
    shadow_pipeline: pipeline::PipelinePair,
    sprite_batch: SpriteBatch,
    white_texture: Texture,
    /// 1×1 linear textures standing in for missing material maps: a flat
    /// normal and white (roughness × 1, metallic × 1).
    flat_normal_texture: Texture,
    linear_white_texture: Texture,
    /// The repeating, mipmapped sampler material maps share.
    material_sampler: wgpu::Sampler,
    /// Tiling ripple normals/heights for water (baked once).
    water_ripples: Texture,
    /// The simulated ripple field ([`crate::RippleMap`]), re-uploaded only
    /// when the draw list carries a different snapshot.
    ripple_field: wgpu::TextureView,
    ripple_field_texture: wgpu::Texture,
    /// Which snapshot `ripple_field` holds: its texels' address and version.
    ripple_field_key: Option<(usize, u64)>,
    depth_texture: Option<wgpu::Texture>,
    shadow_texture: wgpu::Texture,
    shadow_sampler: wgpu::Sampler,
    /// The whole shadow map array (sampled) and one view per cascade
    /// (rendered into).
    shadow_array_view: wgpu::TextureView,
    shadow_layer_views: Vec<wgpu::TextureView>,
    shadow_map_size: u32,
    shadows_enabled: bool,
    /// Point light shadow cubes: six layers per shadowed light (one 1×1
    /// stand-in layer when the preset shadows none).
    point_shadows: DepthArray,
    /// What each point shadow layer last rendered (a hash of its view and
    /// the casters it saw), so unchanged faces aren't drawn again.
    point_shadow_keys: Vec<Option<u64>>,
    /// The frame's instances (model matrices) for the camera's and the
    /// sun shadows' batches, rewritten each frame and grown as needed.
    instance_buffer: wgpu::Buffer,
    quality: GraphicsQuality,
    /// The unit grid water surfaces draw with (built on first use).
    water_mesh: Option<Mesh>,
    /// Water pipelines by target format, refraction (Beautiful) and the
    /// shadows they sample (forward path; refraction samples all).
    water_pipelines: Vec<(
        wgpu::TextureFormat,
        bool,
        pipeline::ForwardShadows,
        wgpu::RenderPipeline,
    )>,
    /// Sky dome pipelines per color format (built on first use).
    sky_pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// Mesh pipelines targeting the HDR scene (Beautiful), built lazily.
    hdr_mesh_pipeline: Option<pipeline::PipelinePair>,
    /// Bloom, tone mapping and FXAA (Beautiful).
    post: crate::post::Post,
    /// Ambient occlusion and contact shadows (Beautiful).
    ssao: crate::ssao::Ssao,
    /// The depth prepass that feeds them, built lazily.
    depth_prepass_pipeline: Option<pipeline::PipelinePair>,
    /// 1x1 stand-ins for the scene copies forward-path water binds.
    scene_placeholder: wgpu::BindGroup,
    /// Animation time source when a draw list sets none.
    clock: std::time::Instant,
    /// Scratch buffer for CPU->surface frame blits (reused across frames).
    blit_scratch: Vec<u8>,
    /// The sprite pipeline targeting [`OFFSCREEN_FORMAT`], built lazily when
    /// it differs from the surface format (meshes get theirs in
    /// `forward_pipelines`). Lets a windowed renderer also draw into
    /// offscreen targets on the same device (Oberon's viewport).
    offscreen_sprite_pipeline: Option<wgpu::RenderPipeline>,
    /// Depth buffer for offscreen targets whose size differs from the
    /// surface's, cached by size.
    offscreen_depth: Option<((u32, u32), wgpu::Texture)>,
    /// Overlay-over-scene compositing for CPU-drawn UIs.
    compositor: Compositor,
    /// Particle pipelines, sprite sheet and instances.
    particles: crate::particles::Particles,
    /// The index buffers grass chunks draw with (see [`GrassIndices`]).
    grass_indices: Option<GrassIndices>,
    /// Grass chunks baked into vertex buffers, by chunk address (each
    /// entry holds its chunk, so the address can't be reused meanwhile).
    grass_chunks: HashMap<usize, BakedGrass>,
    /// Frames that drew grass, to forget chunks not drawn for a while.
    grass_frame: u64,
    /// Grass materials (their colour palettes) by [`crate::GrassLook::key`].
    grass_materials: HashMap<u64, Material>,
}

impl Renderer {
    /// Creates a renderer for a window surface.
    ///
    /// # Errors
    ///
    /// Returns an error if no suitable GPU adapter or device is available, or
    /// if the window surface is incompatible.
    pub async fn new(
        surface_target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: (u32, u32),
        vsync: bool,
        quality: GraphicsQuality,
    ) -> Result<Self> {
        let ctx = RenderContext::new(surface_target, size, vsync).await?;
        Ok(Self::build(ctx, size, quality))
    }

    /// Creates a renderer without a surface, rendering into an offscreen
    /// texture instead. Used by tests and headless tooling; pair with
    /// [`Renderer::render_offscreen`].
    ///
    /// # Errors
    ///
    /// Returns an error if no suitable GPU adapter or device is available.
    pub async fn new_offscreen(size: (u32, u32)) -> Result<Self> {
        Self::new_offscreen_with_quality(size, GraphicsQuality::default()).await
    }

    /// Creates an offscreen renderer with an explicit quality preset.
    ///
    /// # Errors
    /// Returns an error if no suitable GPU adapter or device is available.
    pub async fn new_offscreen_with_quality(
        size: (u32, u32),
        quality: GraphicsQuality,
    ) -> Result<Self> {
        let ctx = RenderContext::new_offscreen(size).await?;
        Ok(Self::build(ctx, size, quality))
    }

    /// Builds the pipelines and resources around a context.
    #[allow(clippy::too_many_lines)] // one field per resource
    fn build(ctx: RenderContext, size: (u32, u32), quality: GraphicsQuality) -> Self {
        let device = ctx.device();
        let shared = PipelineShared::new(device);
        let sprite_pipeline = pipeline::create_sprite_pipeline(device, &shared, ctx.format());
        let forward_pipelines =
            create_forward_pipelines(device, &shared, ctx.format(), quality, &[]);
        let sprite_batch = SpriteBatch::new(device);
        let white_texture = Texture::from_rgba(
            device,
            ctx.queue(),
            1,
            1,
            &[255, 255, 255, 255],
            wgpu::FilterMode::Linear,
        );
        let linear_1x1 = |rgba: [u8; 4]| {
            Texture::from_rgba_mipmapped_as(
                device,
                ctx.queue(),
                1,
                1,
                &rgba,
                false,
                wgpu::TextureFormat::Rgba8Unorm,
            )
        };
        let flat_normal_texture = linear_1x1([128, 128, 255, 255]);
        let linear_white_texture = linear_1x1([255; 4]);
        let material_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material maps"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let water_ripples = Texture::from_rgba_mipmapped_as(
            device,
            ctx.queue(),
            crate::water::RIPPLE_SIZE,
            crate::water::RIPPLE_SIZE,
            &crate::water::ripple_texels(),
            false,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let ripple_field_texture = create_ripple_field(device);
        let ripple_field =
            ripple_field_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_texture = Some(create_depth_texture(device, size));
        let shadow_pipeline = pipeline::create_shadow_pipelines(device, &shared);
        let compositor = Compositor::new(device);
        let shadow = ShadowResources::new(device, quality);
        let point_shadows = DepthArray::for_point_shadows(device, quality);
        let post = crate::post::Post::new(device);
        let ssao = crate::ssao::Ssao::new(device);
        let scene_placeholder = placeholder_scene_bind_group(device, &shared);
        let particles = crate::particles::Particles::new(device, ctx.queue());
        let instances = instance_buffer(device, 1024);
        Self {
            shared,
            sprite_pipeline,
            forward_pipelines,
            shadow_pipeline,
            sprite_batch,
            white_texture,
            flat_normal_texture,
            linear_white_texture,
            material_sampler,
            water_ripples,
            ripple_field,
            ripple_field_texture,
            ripple_field_key: None,
            depth_texture,
            shadow_texture: shadow.texture,
            shadow_sampler: shadow.sampler,
            shadow_array_view: shadow.array_view,
            shadow_layer_views: shadow.layer_views,
            shadow_map_size: quality.shadow_map_size(),
            shadows_enabled: quality.shadows_enabled(),
            point_shadows,
            point_shadow_keys: Vec::new(),
            instance_buffer: instances,
            quality,
            water_mesh: None,
            water_pipelines: Vec::new(),
            sky_pipelines: Vec::new(),
            hdr_mesh_pipeline: None,
            post,
            ssao,
            depth_prepass_pipeline: None,
            scene_placeholder,
            clock: std::time::Instant::now(),
            blit_scratch: Vec::new(),
            offscreen_sprite_pipeline: None,
            offscreen_depth: None,
            compositor,
            particles,
            grass_indices: None,
            grass_chunks: HashMap::new(),
            grass_frame: 0,
            grass_materials: HashMap::new(),
            ctx,
        }
    }

    /// Turns V-Sync on or off at runtime (takes effect next frame). With it
    /// on, presenting paces the caller to the display's refresh rate.
    pub fn set_vsync(&mut self, vsync: bool) {
        self.ctx.set_vsync(vsync);
    }

    /// Whether V-Sync is on.
    #[must_use]
    pub fn vsync(&self) -> bool {
        self.ctx.vsync()
    }

    /// Switches the quality preset at runtime (recreates the shadow maps;
    /// leaving Beautiful frees its HDR targets).
    pub fn set_quality(&mut self, quality: GraphicsQuality) {
        let layers = self.shadow_texture.depth_or_array_layers();
        if quality.shadow_map_size() != self.shadow_map_size || quality.shadow_cascades() != layers
        {
            let shadow = ShadowResources::new(self.ctx.device(), quality);
            self.shadow_texture = shadow.texture;
            self.shadow_sampler = shadow.sampler;
            self.shadow_array_view = shadow.array_view;
            self.shadow_layer_views = shadow.layer_views;
            self.shadow_map_size = quality.shadow_map_size();
        }
        self.shadows_enabled = quality.shadows_enabled();
        if self.point_shadows.size != quality.point_shadow_size()
            || self.point_shadows.views.len() != DepthArray::point_layers(quality)
        {
            self.point_shadows = DepthArray::for_point_shadows(self.ctx.device(), quality);
            self.point_shadow_keys.clear();
        }
        // Pipelines for shadow mixes the new preset never samples go.
        let mixes = pipeline::ForwardShadows::for_quality(quality);
        self.forward_pipelines.retain(|(_, s, _)| mixes.contains(s));
        self.water_pipelines
            .retain(|(_, refract, s, _)| *refract || mixes.contains(s));
        if !quality.post_effects() || quality.is_beautiful() {
            self.post.release_ldr();
        }
        if !quality.is_beautiful() {
            self.post.release();
            self.ssao.release();
            self.hdr_mesh_pipeline = None;
            self.depth_prepass_pipeline = None;
        }
        self.quality = quality;
    }

    /// The active quality preset.
    #[must_use]
    pub fn quality(&self) -> GraphicsQuality {
        self.quality
    }

    /// The underlying GPU context.
    #[must_use]
    pub fn context(&self) -> &RenderContext {
        &self.ctx
    }

    /// The drawable size in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.ctx.size()
    }

    /// Lets go of the window surface (the app went to the background).
    /// Frames are skipped until [`resume`](Self::resume); meshes, textures
    /// and pipelines stay on the GPU.
    pub fn suspend(&mut self) {
        self.ctx.suspend();
    }

    /// Whether there's a window surface to present to.
    #[must_use]
    pub fn has_surface(&self) -> bool {
        self.ctx.has_surface()
    }

    /// Renders to a new window surface after [`suspend`](Self::suspend),
    /// keeping every GPU resource.
    ///
    /// # Errors
    ///
    /// When the surface can't be made.
    pub fn resume(
        &mut self,
        surface_target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: (u32, u32),
    ) -> Result<()> {
        self.ctx.resume(surface_target, size)?;
        let size = self.ctx.size();
        self.depth_texture = Some(create_depth_texture(self.ctx.device(), size));
        Ok(())
    }

    /// Resizes the render surface and depth buffer.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.ctx.resize(width, height);
        let size = self.ctx.size();
        self.depth_texture = Some(create_depth_texture(self.ctx.device(), size));
    }

    /// The built-in white texture, used as a fallback for untextured
    /// materials and solid-color sprites.
    #[must_use]
    pub fn white_texture(&self) -> Texture {
        self.white_texture.clone()
    }

    /// Uploads CPU mesh data to the GPU.
    ///
    /// # Panics
    ///
    /// Panics if the index count exceeds `u32::MAX`.
    #[must_use]
    pub fn create_mesh(&self, data: &MeshData) -> Mesh {
        self.upload_mesh(data, None)
    }

    /// Uploads a skinned mesh: `data` in bind space plus each vertex's
    /// joints and weights (`skin` matches `data.vertices` one to one).
    /// Draw it with [`DrawList::skinned_mesh`].
    ///
    /// # Panics
    ///
    /// Panics if the index count exceeds `u32::MAX`, or if `skin` and the
    /// vertices differ in length.
    #[must_use]
    pub fn create_skinned_mesh(&self, data: &MeshData, skin: &[SkinVertex]) -> Mesh {
        assert_eq!(skin.len(), data.vertices.len(), "one SkinVertex per vertex");
        self.upload_mesh(data, Some(skin))
    }

    fn upload_mesh(&self, data: &MeshData, skin: Option<&[SkinVertex]>) -> Mesh {
        let device = self.ctx.device();
        let skin_buffer = skin.map(|skin| {
            // Out-of-range joints would read past the palette.
            let last = u16::try_from(crate::MAX_JOINTS - 1).unwrap_or(u16::MAX);
            let clamped: Vec<SkinVertex> = skin
                .iter()
                .map(|s| SkinVertex {
                    joints: s.joints.map(|j| j.min(last)),
                    weights: s.weights,
                })
                .collect();
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh skin"),
                contents: bytemuck::cast_slice(&clamped),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytemuck::cast_slice(&data.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Mesh {
            inner: Arc::new(MeshInner {
                vertex_buffer,
                index_buffer,
                index_count: u32::try_from(data.indices.len()).expect("mesh index count fits u32"),
                skin_buffer,
                bounding_radius: data
                    .vertices
                    .iter()
                    .map(|v| glam::Vec3::from(v.position).length())
                    .fold(0.0_f32, f32::max),
                bounds: mesh_bounds(data),
                key: crate::mesh::content_key(data, skin),
            }),
        }
    }

    /// Creates a repeating, mipmapped texture from raw RGBA8 data, for 3D
    /// model surfaces (tiled UVs, seen at every distance). `pixelated`
    /// keeps hard texel edges up close (nearest magnification), for palette
    /// atlases and pixel art.
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4`.
    #[must_use]
    pub fn create_texture_rgba_mipmapped(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
        pixelated: bool,
    ) -> Texture {
        Texture::from_rgba_mipmapped(
            self.ctx.device(),
            self.ctx.queue(),
            width,
            height,
            rgba,
            pixelated,
        )
    }

    /// Creates a texture from raw RGBA8 data.
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4`.
    #[must_use]
    pub fn create_texture_rgba(&self, width: u32, height: u32, rgba: &[u8]) -> Texture {
        Texture::from_rgba(
            self.ctx.device(),
            self.ctx.queue(),
            width,
            height,
            rgba,
            wgpu::FilterMode::Linear,
        )
    }

    /// Creates a texture from raw RGBA8 data with nearest-neighbor sampling
    /// (for pixel art).
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4`.
    #[must_use]
    pub fn create_texture_rgba_nearest(&self, width: u32, height: u32, rgba: &[u8]) -> Texture {
        Texture::from_rgba(
            self.ctx.device(),
            self.ctx.queue(),
            width,
            height,
            rgba,
            wgpu::FilterMode::Nearest,
        )
    }
    /// Creates a texture by decoding an image file (PNG or JPEG).
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Image`] if the bytes are not a decodable image.
    pub fn create_texture_from_bytes(&self, bytes: &[u8]) -> Result<Texture> {
        Texture::from_image_bytes(self.ctx.device(), self.ctx.queue(), bytes)
    }

    /// Creates a material. `texture` defaults to the 1×1 white fallback when
    /// `None`. The surface is the default slightly rough non-metal; see
    /// [`Renderer::create_pbr_material`] for the full set of parameters.
    #[must_use]
    pub fn create_material(&self, color: [f32; 4], texture: Option<Texture>) -> Material {
        self.create_pbr_material(
            &crate::MaterialParams::with_color(color),
            crate::MaterialTextures {
                base: texture,
                ..crate::MaterialTextures::default()
            },
        )
    }

    /// Creates a physically based material: base color, roughness,
    /// metallic, emissive, and optional base / normal / metallic-roughness /
    /// emissive maps (see [`crate::MaterialTextures`]). Every quality preset
    /// reads all of it; Beautiful shades it with GGX and sky reflections.
    #[must_use]
    pub fn create_pbr_material(
        &self,
        params: &crate::MaterialParams,
        textures: crate::MaterialTextures,
    ) -> Material {
        let device = self.ctx.device();
        let maps = [
            (textures.normal.is_some(), crate::material::MAP_NORMAL),
            (
                textures.metallic_roughness.is_some(),
                crate::material::MAP_METAL_ROUGH,
            ),
            (textures.emissive.is_some(), crate::material::MAP_EMISSIVE),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |bits, (_, bit)| bits | bit);
        let uniform = MaterialUniform::new(params, maps);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let base = textures.base.unwrap_or_else(|| self.white_texture.clone());
        let normal = textures
            .normal
            .unwrap_or_else(|| self.flat_normal_texture.clone());
        let metallic_roughness = textures
            .metallic_roughness
            .unwrap_or_else(|| self.linear_white_texture.clone());
        let emissive = textures
            .emissive
            .unwrap_or_else(|| self.white_texture.clone());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout: &self.shared.material_layout,
            entries: &[
                view_entry(0, &base),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&base.inner.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
                view_entry(3, &normal),
                view_entry(4, &metallic_roughness),
                view_entry(5, &emissive),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.material_sampler),
                },
            ],
        });
        let key = crate::material::content_key(
            &uniform,
            [&base, &normal, &metallic_roughness, &emissive],
        );
        Material {
            inner: Arc::new(MaterialInner {
                bind_group,
                params: *params,
                texture: base,
                _maps: [normal, metallic_roughness, emissive],
                key,
            }),
        }
    }

    /// Creates a mipmapped, repeating texture holding *data* rather than
    /// color (normal maps, metallic-roughness maps): stored linear, so the
    /// GPU doesn't apply the sRGB curve to it.
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4`.
    #[must_use]
    pub fn create_data_texture(&self, width: u32, height: u32, rgba: &[u8]) -> Texture {
        Texture::from_rgba_mipmapped_as(
            self.ctx.device(),
            self.ctx.queue(),
            width,
            height,
            rgba,
            false,
            wgpu::TextureFormat::Rgba8Unorm,
        )
    }

    /// Executes a draw list and presents the frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the surface is lost or invalid and cannot be
    /// reconfigured.
    pub fn render(&mut self, draw: &DrawList) -> Result<()> {
        let target = match self.ctx.acquire_frame()? {
            Some(texture) => FrameTarget::Surface(texture),
            None => return Ok(()), // frame skipped (occluded / outdated)
        };
        self.execute_frame(draw, target);
        Ok(())
    }

    /// Executes a draw list into an offscreen RGBA8 target texture.
    ///
    /// The texture must be `Rgba8UnormSrgb` with
    /// `RENDER_ATTACHMENT | COPY_SRC` usage. Intended for tests and tooling
    /// that need to inspect rendered output without a window.
    ///
    /// Works on windowed renderers too: the target may be any size (it gets
    /// its own depth buffer) and is drawn with pipelines built for the
    /// offscreen format (`Rgba8UnormSrgb`), so one device can drive both the
    /// window and an embedded viewport.
    pub fn render_offscreen(&mut self, draw: &DrawList, target: &wgpu::Texture) {
        let size = (target.width().max(1), target.height().max(1));
        self.ensure_offscreen_resources(size);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.execute_frame(draw, FrameTarget::Offscreen { view, size });
    }

    /// Lazily builds the offscreen-format pipelines (only when the surface
    /// format differs) and a depth buffer matching `size` (only when it
    /// differs from the surface size).
    fn ensure_offscreen_resources(&mut self, size: (u32, u32)) {
        if self.ctx.format() != OFFSCREEN_FORMAT && self.offscreen_sprite_pipeline.is_none() {
            let sprite =
                pipeline::create_sprite_pipeline(self.ctx.device(), &self.shared, OFFSCREEN_FORMAT);
            self.offscreen_sprite_pipeline = Some(sprite);
        }
        let cached = self.offscreen_depth.as_ref().map(|(s, _)| *s);
        if size != self.ctx.size() && cached != Some(size) {
            self.offscreen_depth = Some((size, create_depth_texture(self.ctx.device(), size)));
        }
    }

    /// Creates an offscreen render target texture of `size` for use with
    /// [`Renderer::render_offscreen`] and [`Renderer::read_texture_rgba`].
    ///
    /// Editors and tooling (Oberon) render the live scene into this texture,
    /// then read it back as RGBA8 to composite into their own UI.
    #[must_use]
    pub fn create_render_target(&self, size: (u32, u32)) -> wgpu::Texture {
        let device = self.ctx.device();
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-target"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            // TEXTURE_BINDING: `present_composited` samples it directly.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    /// Reads a render target's pixels back to the CPU as tightly packed
    /// RGBA8 (row-major, top-left origin). Blocks until the GPU finishes.
    ///
    /// Intended for editors and tooling that composite engine output into
    /// their own window; not for per-frame game use.
    #[must_use]
    pub fn read_texture_rgba(&self, target: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
        let mut out = Vec::new();
        self.read_texture_rgba_into(target, size, &mut out);
        out
    }

    /// Like [`Renderer::read_texture_rgba`], but writes into a caller-provided
    /// buffer (reused across frames) instead of allocating. `out` is resized
    /// to exactly `width*height*4`.
    pub fn read_texture_rgba_into(
        &self,
        target: &wgpu::Texture,
        size: (u32, u32),
        out: &mut Vec<u8>,
    ) {
        read_texture_rgba_into(self, target, size, out);
    }

    /// Blits a tightly packed RGBA8 image to the window surface and presents.
    ///
    /// `size` is the surface's current drawable size and must match the
    /// configured surface; `rgba.len()` must equal `width*height*4`. This is
    /// how external UIs (Oberon) present a CPU-rendered frame through the
    /// engine's existing surface — no OpenGL or `unsafe` needed.
    ///
    /// The image is written unscaled and must exactly cover the surface.
    /// `rgba` is in **R,G,B,A byte order** (tiny-skia's layout); it is
    /// converted to the surface's native order (often BGRA) before upload.
    pub fn blit_rgba_to_surface(&mut self, rgba: &[u8], size: (u32, u32)) {
        let (width, height) = (size.0.max(1), size.1.max(1));
        if rgba.len() != (width * height * 4) as usize {
            warn!(
                expected = width * height * 4,
                got = rgba.len(),
                "blit_rgba_to_surface: buffer size mismatch; frame skipped"
            );
            return;
        }
        // Keep the surface in step with the caller's frame size (a resize
        // event may not have been delivered yet).
        if self.ctx.size() != (width, height) {
            self.resize(width, height);
        }
        let Some(texture) = self.ctx.acquire_frame().ok().flatten() else {
            return; // occluded / outdated — skip this frame
        };
        // The swapchain image must exactly match the upload; otherwise
        // `write_texture` fails validation. Drop the frame (unpresented) and
        // try again next frame.
        if (texture.texture.width(), texture.texture.height()) != (width, height) {
            drop(texture);
            return;
        }

        // Swap R<->B when the surface is BGR-ordered so colors stay correct.
        let needs_bgr = matches!(
            self.ctx.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let data: &[u8] = if needs_bgr {
            self.blit_scratch.clear();
            self.blit_scratch.extend_from_slice(rgba);
            for px in self.blit_scratch.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            &self.blit_scratch
        } else {
            rgba
        };

        let queue = self.ctx.queue();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.ctx.present(texture);
        // Reclaim finished submissions. `present` flushes the write above in
        // an internal submission but never runs device maintenance, so
        // without a poll every frame's ~(w*h*4)-byte staging buffer stays
        // alive forever — hundreds of MB per second at 60 FPS.
        let _ = self.ctx.device().poll(wgpu::PollType::Poll);
    }

    /// Presents a CPU-drawn frame: `overlay` (premultiplied RGBA8, the
    /// window's `size`) with `scene` (a render target from
    /// [`Self::create_render_target`], drawn with [`Self::render_offscreen`])
    /// showing through its transparent pixels, top-left corner at `origin`
    /// in window pixels.
    ///
    /// This is how an editor puts a live 3D view inside its UI without
    /// reading the view back to the CPU; with `scene: None` it is a plain
    /// upload like [`Self::blit_rgba_to_surface`].
    pub fn present_composited(
        &mut self,
        overlay: &[u8],
        size: (u32, u32),
        scene: Option<(&wgpu::Texture, (i32, i32))>,
    ) {
        let (width, height) = (size.0.max(1), size.1.max(1));
        if overlay.len() != (width * height * 4) as usize {
            warn!(
                expected = width * height * 4,
                got = overlay.len(),
                "present_composited: overlay size mismatch; frame skipped"
            );
            return;
        }
        if self.ctx.size() != (width, height) {
            self.resize(width, height);
        }
        let Some(texture) = self.ctx.acquire_frame().ok().flatten() else {
            return; // occluded / outdated — skip this frame
        };
        if (texture.texture.width(), texture.texture.height()) != (width, height) {
            drop(texture);
            return;
        }
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.compositor.composite(
            self.ctx.device(),
            self.ctx.queue(),
            &view,
            self.ctx.format(),
            (width, height),
            overlay,
            scene,
        );
        drop(view);
        self.ctx.present(texture);
        let _ = self.ctx.device().poll(wgpu::PollType::Poll);
    }

    /// [`Self::present_composited`] into a render target instead of the
    /// window (tests and tools).
    pub fn composite_offscreen(
        &mut self,
        overlay: &[u8],
        target: &wgpu::Texture,
        scene: Option<(&wgpu::Texture, (i32, i32))>,
    ) {
        let size = (target.width(), target.height());
        if overlay.len() != (size.0 * size.1 * 4) as usize {
            warn!("composite_offscreen: overlay size mismatch");
            return;
        }
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.compositor.composite(
            self.ctx.device(),
            self.ctx.queue(),
            &view,
            target.format(),
            size,
            overlay,
            scene,
        );
    }

    /// Builds a camera bind group from a view-projection matrix and the
    /// camera's world position (used for specular highlights).
    /// `grass` is the wind and pushers grass bends with (see
    /// [`grass_uniform`]).
    fn camera_bind_group(
        &self,
        view_proj: glam::Mat4,
        view_pos: glam::Vec3,
        grass: &[f32; GRASS_UNIFORM_FLOATS],
    ) -> wgpu::BindGroup {
        let mut contents = [0.0_f32; 20 + GRASS_UNIFORM_FLOATS];
        contents[..16].copy_from_slice(&view_proj.to_cols_array());
        contents[16..19].copy_from_slice(&view_pos.to_array());
        contents[19] = 1.0;
        contents[20..].copy_from_slice(grass);
        self.uniform_bind_group(
            Some("camera"),
            bytemuck::cast_slice(&contents),
            &self.shared.camera_layout,
        )
    }

    /// Builds a skinned draw's model bind group: the model matrix plus the
    /// joint palette (padded to [`crate::MAX_JOINTS`]).
    fn skinned_model_bind_group(
        &self,
        model: glam::Mat4,
        joints: &[glam::Mat4],
    ) -> wgpu::BindGroup {
        let device = self.ctx.device();
        let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model"),
            contents: bytemuck::cast_slice(&model.to_cols_array()),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut palette = Vec::with_capacity(crate::MAX_JOINTS * 16);
        for i in 0..crate::MAX_JOINTS {
            let joint = joints.get(i).copied().unwrap_or(glam::Mat4::IDENTITY);
            palette.extend_from_slice(&joint.to_cols_array());
        }
        let joint_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("joints"),
            contents: bytemuck::cast_slice(&palette),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skinned model"),
            layout: &self.shared.skinned_model_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: model_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: joint_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Builds a single-buffer uniform bind group for `layout`.
    fn uniform_bind_group(
        &self,
        label: Option<&str>,
        contents: &[u8],
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        let device = self.ctx.device();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label,
            contents,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label,
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// Renders the frame into `target`, then presents if it's the surface.
    fn execute_frame(&mut self, draw: &DrawList, target: FrameTarget) {
        let (viewport, offscreen) = match &target {
            FrameTarget::Offscreen { size, .. } => (*size, true),
            FrameTarget::Surface(_) => {
                let s = self.ctx.size();
                ((s.0.max(1), s.1.max(1)), false)
            }
        };
        let format = if offscreen {
            OFFSCREEN_FORMAT
        } else {
            self.ctx.format()
        };
        let beautiful = self.quality.is_beautiful();
        let chain = self.ldr_chain(draw, viewport);
        self.ensure_frame_pipelines(draw, viewport, format, beautiful);
        if chain.is_some() {
            self.post.ensure_ldr(self.ctx.device(), viewport, format);
        }

        let mut encoder =
            self.ctx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("frame"),
                });
        let prepared = self.prepare_frame(draw, viewport, beautiful);

        // --- Shadow passes: depth from the sun's view, one per cascade ---
        if prepared.render_shadows {
            self.record_shadow_passes(&mut encoder, draw, &prepared);
        }
        // --- and six per shadowed point light (those that changed) ---
        let stale = self.stale_point_shadow_layers(draw, &prepared);
        self.record_point_shadow_passes(&mut encoder, draw, &prepared, &stale);

        // The view borrows the surface texture only for the scope of the
        // passes; present() takes it by value afterwards.
        {
            let view = match &target {
                FrameTarget::Surface(texture) => texture
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default()),
                FrameTarget::Offscreen { view, .. } => view.clone(),
            };
            // Offscreen targets of a different size use their own depth buffer.
            let depth_texture = match &self.offscreen_depth {
                Some((size, texture)) if offscreen && *size == viewport => Some(texture),
                _ => self.depth_texture.as_ref(),
            };
            let depth_view =
                depth_texture.map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
            if beautiful {
                self.record_beautiful(
                    &mut encoder,
                    draw,
                    &prepared,
                    &view,
                    depth_view.as_ref(),
                    format,
                );
            } else {
                self.record_forward(
                    &mut encoder,
                    draw,
                    &prepared,
                    &view,
                    depth_view.as_ref(),
                    format,
                    offscreen,
                    chain,
                );
            }
        }

        self.ctx.queue().submit([encoder.finish()]);
        match target {
            FrameTarget::Surface(texture) => self.ctx.present(texture),
            FrameTarget::Offscreen { .. } => {
                // No wait: reads (`read_texture_rgba`) wait for their own
                // copy, and a GPU-composited view never needs the CPU to.
                let _ = self.ctx.device().poll(wgpu::PollType::Poll);
            }
        }
    }

    /// Builds what this frame needs and doesn't exist yet: the Beautiful
    /// path's HDR targets and pipelines, and water pipelines.
    fn ensure_frame_pipelines(
        &mut self,
        draw: &DrawList,
        viewport: (u32, u32),
        format: wgpu::TextureFormat,
        beautiful: bool,
    ) {
        if !draw.waters.is_empty() && self.water_mesh.is_none() {
            self.water_mesh = Some(self.create_mesh(&crate::water::grid_mesh()));
        }
        let device = self.ctx.device();
        if beautiful {
            self.post.ensure(device, viewport, format);
            self.ssao.ensure(device, viewport);
            if self.depth_prepass_pipeline.is_none() {
                self.depth_prepass_pipeline = Some(pipeline::create_depth_prepass_pipelines(
                    device,
                    &self.shared,
                ));
            }
            if self.hdr_mesh_pipeline.is_none() {
                self.hdr_mesh_pipeline = Some(pipeline::create_mesh_pipelines(
                    device,
                    &self.shared,
                    pipeline::HDR_FORMAT,
                    pipeline::HDR_DEPTH_FORMAT,
                    pipeline::ForwardShadows::ALL,
                ));
            }
        }
        if draws_sky(draw) {
            let (sky_format, depth) = if beautiful {
                (pipeline::HDR_FORMAT, pipeline::HDR_DEPTH_FORMAT)
            } else {
                (format, DEPTH_FORMAT)
            };
            if !self.sky_pipelines.iter().any(|(f, _)| *f == sky_format) {
                let p = pipeline::create_sky_pipeline(device, &self.shared, sky_format, depth);
                self.sky_pipelines.push((sky_format, p));
            }
        }
        if !beautiful {
            let missing = create_forward_pipelines(
                device,
                &self.shared,
                format,
                self.quality,
                &self.forward_pipelines,
            );
            self.forward_pipelines.extend(missing);
        }
        if !draw.particles.is_empty() {
            let particle_format = if beautiful {
                pipeline::HDR_FORMAT
            } else {
                format
            };
            self.particles.ensure(
                device,
                &self.shared.camera_layout,
                &self.shared.light_layout,
                particle_format,
            );
        }
        if !draw.waters.is_empty() {
            let (water_format, depth, mixes) = if beautiful {
                (
                    pipeline::HDR_FORMAT,
                    pipeline::HDR_DEPTH_FORMAT,
                    vec![pipeline::ForwardShadows::ALL],
                )
            } else {
                (
                    format,
                    DEPTH_FORMAT,
                    pipeline::ForwardShadows::for_quality(self.quality),
                )
            };
            for shadows in mixes {
                let exists = self
                    .water_pipelines
                    .iter()
                    .any(|(f, r, s, _)| *f == water_format && *r == beautiful && *s == shadows);
                if !exists {
                    let p = pipeline::create_water_pipeline(
                        device,
                        &self.shared,
                        water_format,
                        depth,
                        beautiful,
                        shadows,
                    );
                    self.water_pipelines
                        .push((water_format, beautiful, shadows, p));
                }
            }
        }
    }

    /// Draws the sky dome behind what's already in the pass, if the frame
    /// has one.
    fn record_sky(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draw: &DrawList,
        prepared: &PreparedFrame,
        format: wgpu::TextureFormat,
    ) {
        if !draws_sky(draw) {
            return;
        }
        let Some((_, pipeline)) = self.sky_pipelines.iter().find(|(f, _)| *f == format) else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
        pass.set_bind_group(3, &prepared.light_bind, &[]);
        pass.draw(0..3, 0..1);
    }

    fn water_pipeline(
        &self,
        format: wgpu::TextureFormat,
        refract: bool,
        shadows: pipeline::ForwardShadows,
    ) -> Option<&wgpu::RenderPipeline> {
        self.water_pipelines
            .iter()
            .find(|(f, r, s, _)| *f == format && *r == refract && *s == shadows)
            .map(|(_, _, _, p)| p)
    }

    /// The forward-path mesh pipelines for `format` sampling `shadows`.
    fn forward_pipeline(
        &self,
        format: wgpu::TextureFormat,
        shadows: pipeline::ForwardShadows,
    ) -> Option<&pipeline::PipelinePair> {
        self.forward_pipelines
            .iter()
            .find(|(f, s, _)| *f == format && *s == shadows)
            .map(|(_, _, p)| p)
    }

    /// The Medium-to-Ultra post chain this frame needs: 3D frames with an
    /// Environment whose post effects or style change something on this
    /// preset.
    fn ldr_chain(&self, draw: &DrawList, viewport: (u32, u32)) -> Option<crate::post::LdrChain> {
        let env = draw.environment.filter(|_| draw.camera_3d.is_some())?;
        let style = crate::post::Stylize::new(&env, draw.camera_3d.as_ref(), viewport);
        crate::post::LdrChain::new(self.quality, &env.post, draw.time.unwrap_or(0.0), style)
    }

    /// The forward path (Low to Ultra), straight into `view`, or with a
    /// post `chain` through an intermediate image graded into it.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn record_forward(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
        view: &wgpu::TextureView,
        depth_view: Option<&wgpu::TextureView>,
        format: wgpu::TextureFormat,
        offscreen: bool,
        chain: Option<crate::post::LdrChain>,
    ) {
        // Offscreen targets use pipelines built for their color format, and
        // only the shadow lookups this frame needs.
        let shadows = prepared.forward_shadows();
        let mesh_pipeline = self.forward_pipeline(format, shadows);
        let sprite_pipeline = match &self.offscreen_sprite_pipeline {
            Some(sprite) if offscreen => sprite,
            _ => &self.sprite_pipeline,
        };
        // With post effects the scene goes into the chain's image first
        // (the final pass needs a depth buffer for the HUD sprites).
        let post_view = match (&self.post.ldr, chain, depth_view) {
            (Some(t), Some(_), Some(_)) if t.format == format => {
                Some(t.scene.create_view(&wgpu::TextureViewDescriptor::default()))
            }
            _ => None,
        };
        let chain = chain.filter(|_| post_view.is_some());
        let scene_view = post_view.as_ref().unwrap_or(view);
        // Particles get a pass of their own after the scene's, reading its
        // depth (kept for them).
        let particle_depth = depth_view.filter(|_| !prepared.particle_runs.is_empty());
        let outlines = chain.filter(|c| c.style.outlines());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: scene_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(draw.clear_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth_view.map(|dv| wgpu::RenderPassDepthStencilAttachment {
                view: dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: if particle_depth.is_some() || outlines.is_some() {
                        wgpu::StoreOp::Store
                    } else {
                        wgpu::StoreOp::Discard
                    },
                }),
                stencil_ops: None,
            }),
            ..wgpu::RenderPassDescriptor::default()
        });
        if let Some(mesh_pipeline) = mesh_pipeline {
            Self::record_meshes(
                &mut pass,
                draw,
                prepared,
                mesh_pipeline,
                &prepared.light_bind,
                &self.instance_buffer,
            );
        }
        if depth_view.is_some() {
            self.record_sky(&mut pass, draw, prepared, format);
        }
        let water = self.water_pipeline(format, false, shadows);
        if let Some(water) = water.filter(|_| outlines.is_none()) {
            self.record_waters(&mut pass, prepared, water, &self.scene_placeholder);
        }
        let (Some(chain), Some(depth_view)) = (chain, depth_view) else {
            if let Some(depth) = particle_depth {
                drop(pass);
                let mut pass = self.particle_pass(encoder, view, depth);
                self.record_particles(&mut pass, prepared, format, depth);
                self.record_sprites(&mut pass, draw, prepared, sprite_pipeline);
            } else {
                self.record_sprites(&mut pass, draw, prepared, sprite_pipeline);
            }
            return;
        };
        drop(pass);
        let device = self.ctx.device();
        // Outlines before the water (which covers what's under it, lines
        // too) and the particles, neither of which writes depth.
        if let Some(chain) = outlines {
            self.post.record_outline(
                device,
                encoder,
                scene_view,
                format,
                depth_view,
                &chain.style,
            );
            let mut pass = self.particle_pass(encoder, scene_view, depth_view);
            if let Some(water) = water {
                self.record_waters(&mut pass, prepared, water, &self.scene_placeholder);
            }
            if particle_depth.is_some() {
                self.record_particles(&mut pass, prepared, format, depth_view);
            }
        } else if let Some(depth) = particle_depth {
            let mut pass = self.particle_pass(encoder, scene_view, depth);
            self.record_particles(&mut pass, prepared, format, depth);
        }
        self.post.record_ldr_bloom(device, encoder, &chain);
        // Final: the graded image into the target, then sprites on top.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("graded"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..wgpu::RenderPassDescriptor::default()
        });
        self.post.draw_grade(device, &mut pass, format, &chain);
        self.record_sprites(&mut pass, draw, prepared, sprite_pipeline);
    }

    /// A pass drawing over `view` with `depth` attached read-only, so the
    /// particles drawn in it can read the depth too.
    #[allow(clippy::unused_self)] // a method like the other passes
    fn particle_pass<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            ..wgpu::RenderPassDescriptor::default()
        })
    }

    /// Records the frame's particles into a pass drawing into `format`.
    fn record_particles(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedFrame,
        format: wgpu::TextureFormat,
        depth: &wgpu::TextureView,
    ) {
        self.particles.record(
            self.ctx.device(),
            pass,
            &prepared.particle_runs,
            format,
            &prepared.camera_3d,
            &prepared.camera_3d_bind,
            &prepared.light_bind,
            depth,
        );
    }

    /// The Beautiful path: the scene in HDR, water refracting through a copy
    /// of it, bloom and tone mapping, then FXAA into the target with HUD
    /// sprites drawn last (untouched by post effects).
    #[allow(clippy::too_many_lines)] // one pass sequence, read top to bottom
    fn record_beautiful(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
        view: &wgpu::TextureView,
        depth_view: Option<&wgpu::TextureView>,
        format: wgpu::TextureFormat,
    ) {
        let (Some(targets), Some(mesh_pipeline), Some(light_bind)) = (
            &self.post.targets,
            &self.hdr_mesh_pipeline,
            &prepared.beautiful_light_bind,
        ) else {
            return;
        };
        let device = self.ctx.device();
        let hdr_view = targets
            .hdr
            .create_view(&wgpu::TextureViewDescriptor::default());
        let hdr_depth = targets
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let water = self
            .water_pipeline(pipeline::HDR_FORMAT, true, pipeline::ForwardShadows::ALL)
            .filter(|_| !draw.waters.is_empty());
        let particles = !prepared.particle_runs.is_empty();
        let env = draw.environment.unwrap_or_default();
        let style = draw
            .environment
            .map(|e| crate::post::Stylize::new(&e, draw.camera_3d.as_ref(), targets.size))
            .unwrap_or_default();
        // God rays, depth of field and outlines read the scene's depth
        // afterwards.
        let lens_depth = env.god_rays > 0.0 || env.depth_of_field > 0.0 || style.outlines();
        let keep_depth = |keep: bool| {
            if keep {
                wgpu::StoreOp::Store
            } else {
                wgpu::StoreOp::Discard
            }
        };
        self.record_occlusion(encoder, draw, prepared);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hdr scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Alpha 0 marks the background (no tone mapping).
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            a: 0.0,
                            ..draw.clear_color
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &hdr_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: keep_depth(water.is_some() || particles || lens_depth),
                    }),
                    stencil_ops: None,
                }),
                ..wgpu::RenderPassDescriptor::default()
            });
            Self::record_meshes(
                &mut pass,
                draw,
                prepared,
                mesh_pipeline,
                light_bind,
                &self.instance_buffer,
            );
            self.record_sky(&mut pass, draw, prepared, pipeline::HDR_FORMAT);
        }

        // Outlines before the water, which covers (and bends) them.
        if style.outlines() {
            self.post.record_outline(
                device,
                encoder,
                &hdr_view,
                pipeline::HDR_FORMAT,
                &hdr_depth,
                &style,
            );
        }
        if let Some(water) = water {
            // Water reads the scene behind it from copies (a texture can't
            // be sampled while it's being drawn into).
            let extent = targets.hdr.size();
            encoder.copy_texture_to_texture(
                targets.hdr.as_image_copy(),
                targets.scene_copy.as_image_copy(),
                extent,
            );
            encoder.copy_texture_to_texture(
                targets.depth.as_image_copy(),
                targets.depth_copy.as_image_copy(),
                extent,
            );
            let scene_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene copy"),
                layout: &self.shared.scene_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &targets
                                .scene_copy
                                .create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &targets
                                .depth_copy
                                .create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.post_sampler()),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &hdr_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: keep_depth(particles || lens_depth),
                    }),
                    stencil_ops: None,
                }),
                ..wgpu::RenderPassDescriptor::default()
            });
            self.record_waters(&mut pass, prepared, water, &scene_bind);
        }
        if particles {
            let mut pass = self.particle_pass(encoder, &hdr_view, &hdr_depth);
            self.record_particles(&mut pass, prepared, pipeline::HDR_FORMAT, &hdr_depth);
        }

        // No lens flare or god rays from under the water.
        let sun = draw
            .camera_3d
            .filter(|_| draws_sky(draw) && !camera_in_water(draw))
            .and_then(|cam| {
                let sun = draw.sun.unwrap_or_default();
                crate::post::SunFlare::new(
                    &cam,
                    targets.size,
                    sun.normalized_direction(),
                    env.sun_size,
                    env.lens_flare,
                    sun.color,
                )
            });
        let lens = beautiful_lens(draw, sun.as_ref());
        self.post.record_bloom_and_resolve(
            device,
            encoder,
            env.exposure.max(0.0),
            beautiful_tonemap(draw.environment),
            sun,
            crate::post::Grade::new(&env.post, draw.time.unwrap_or(0.0)),
            env.post.bloom,
            lens,
        );

        // Final: FXAA into the target, then sprites on top.
        let sprite_pipeline = match &self.offscreen_sprite_pipeline {
            Some(sprite) if format == OFFSCREEN_FORMAT && format != self.ctx.format() => sprite,
            _ => &self.sprite_pipeline,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("final"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth_view.map(|dv| wgpu::RenderPassDepthStencilAttachment {
                view: dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..wgpu::RenderPassDescriptor::default()
        });
        self.post.draw_fxaa(device, &mut pass, format, &style);
        self.record_sprites(&mut pass, draw, prepared, sprite_pipeline);
    }

    /// Beautiful's screen-space occlusion: the opaque meshes' depth from
    /// the camera, then ambient occlusion and contact shadows from it.
    /// Skipped (the result cleared to "unoccluded") when the scene needs
    /// neither: AO strength 0 and no sun shadows.
    fn record_occlusion(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
    ) {
        let (Some(pipelines), Some(depth)) = (&self.depth_prepass_pipeline, self.ssao.depth_view())
        else {
            return;
        };
        let env = draw.environment.unwrap_or_default();
        let frame = crate::ssao::SsaoFrame {
            camera: draw.camera_3d.unwrap_or_default(),
            sun_direction: draw.sun.unwrap_or_default().normalized_direction(),
            ambient_occlusion: env.ambient_occlusion > 0.0,
            contact_shadows: prepared.render_shadows,
        };
        if frame.active() && !draw.meshes.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth prepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..wgpu::RenderPassDescriptor::default()
            });
            pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
            // See-through meshes don't hide what's behind them; fading ones
            // go in whole (the occlusion only darkens what's drawn).
            let solid = prepared.main_batches.iter().filter(|b| !b.transparent);
            Self::record_depth_batches(
                &mut pass,
                draw,
                prepared,
                solid,
                pipelines,
                &self.instance_buffer,
            );
        }
        let frame = crate::ssao::SsaoFrame {
            ambient_occlusion: frame.ambient_occlusion && !draw.meshes.is_empty(),
            contact_shadows: frame.contact_shadows && !draw.meshes.is_empty(),
            ..frame
        };
        self.ssao.record(self.ctx.device(), encoder, &frame);
    }

    /// A linear clamping sampler for the scene copies.
    fn post_sampler(&self) -> wgpu::Sampler {
        self.ctx.device().create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene copy"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        })
    }

    /// Records every mesh (depth-tested, lit).
    fn record_meshes(
        pass: &mut wgpu::RenderPass<'_>,
        draw: &DrawList,
        prepared: &PreparedFrame,
        pipelines: &pipeline::PipelinePair,
        light_bind: &wgpu::BindGroup,
        instances: &wgpu::Buffer,
    ) {
        pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
        pass.set_bind_group(3, light_bind, &[]);
        let mut current = None;
        for batch in &prepared.main_batches {
            if current != Some(batch.kind) {
                pass.set_pipeline(pipelines.get(batch.kind));
                current = Some(batch.kind);
            }
            let material = if batch.kind == crate::batch::BatchKind::Grass {
                &prepared.grass_materials[batch.draw]
            } else {
                &draw.meshes[batch.draw].material
            };
            pass.set_bind_group(2, &material.inner.bind_group, &[]);
            Self::record_batch(pass, draw, prepared, batch, instances);
        }
    }

    /// Records one batch's geometry and draw call: a skinned mesh with its
    /// bind group and skin, a baked grass chunk, or instanced copies from
    /// `instances`.
    fn record_batch(
        pass: &mut wgpu::RenderPass<'_>,
        draw: &DrawList,
        prepared: &PreparedFrame,
        batch: &crate::batch::Batch,
        instances: &wgpu::Buffer,
    ) {
        let mesh = &batch.mesh.inner;
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        if batch.kind == crate::batch::BatchKind::Grass {
            // A baked chunk: its first blades, no instances.
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.count.min(mesh.index_count), 0, 0..1);
            return;
        }
        if batch.kind == crate::batch::BatchKind::Skinned {
            let (Some(model_bind), Some(skin)) =
                (&prepared.model_bind_groups[batch.draw], &mesh.skin_buffer)
            else {
                return;
            };
            debug_assert!(draw.meshes[batch.draw].joints.is_some());
            pass.set_bind_group(1, model_bind, &[]);
            pass.set_vertex_buffer(1, skin.slice(..));
        } else {
            let start = u64::from(batch.first) * crate::batch::INSTANCE_SIZE;
            let end = start + u64::from(batch.count) * crate::batch::INSTANCE_SIZE;
            pass.set_vertex_buffer(1, instances.slice(start..end));
        }
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let copies = if batch.kind == crate::batch::BatchKind::Skinned {
            1
        } else {
            batch.count
        };
        pass.draw_indexed(0..mesh.index_count, 0, 0..copies);
    }

    /// Records depth-only batches (shadows, the depth prepass) with
    /// `pipelines`, their copies in `instances`.
    fn record_depth_batches<'b>(
        pass: &mut wgpu::RenderPass<'_>,
        draw: &DrawList,
        prepared: &PreparedFrame,
        batches: impl IntoIterator<Item = &'b crate::batch::Batch>,
        pipelines: &pipeline::PipelinePair,
        instances: &wgpu::Buffer,
    ) {
        let mut current = None;
        for batch in batches {
            // Depth passes draw fading copies solid.
            let kind = match batch.kind {
                crate::batch::BatchKind::Faded => crate::batch::BatchKind::Plain,
                kind => kind,
            };
            if current != Some(kind) {
                pass.set_pipeline(pipelines.get(kind));
                current = Some(kind);
            }
            Self::record_batch(pass, draw, prepared, batch, instances);
        }
    }

    /// Records every water surface with `pipeline`.
    fn record_waters(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedFrame,
        pipeline: &wgpu::RenderPipeline,
        scene: &wgpu::BindGroup,
    ) {
        let Some(mesh) = &self.water_mesh else {
            return;
        };
        if prepared.water_binds.is_empty() {
            return;
        }
        let mesh = &mesh.inner;
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
        pass.set_bind_group(2, scene, &[]);
        pass.set_bind_group(3, &prepared.light_bind, &[]);
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for (bind, indices) in &prepared.water_binds {
            pass.set_bind_group(1, bind, &[]);
            pass.draw_indexed(0..(*indices).min(mesh.index_count), 0, 0..1);
        }
    }

    /// Records the frame's sprites (on top, depth ignored).
    fn record_sprites(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draw: &DrawList,
        prepared: &PreparedFrame,
        pipeline: &wgpu::RenderPipeline,
    ) {
        if draw.sprites.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &prepared.camera_2d_bind, &[]);
        pass.set_vertex_buffer(0, self.sprite_batch.vertex_buffer.slice(..));
        pass.set_index_buffer(
            self.sprite_batch.index_buffer.slice(..),
            wgpu::IndexFormat::Uint16,
        );
        for run in &prepared.sprite_runs {
            let texture_bind = self.sprite_batch.texture_bind_group(
                self.ctx.device(),
                &self.shared.texture_layout,
                &run.texture,
            );
            pass.set_bind_group(1, &texture_bind, &[]);
            let start = run.first_index;
            let end = start + run.index_count;
            pass.draw_indexed(start..end, 0, 0..1);
        }
    }

    /// Each water surface's uniform/ripple bind group and how many indices
    /// to draw (endless ones include the horizon skirt).
    /// Uploads a ripple snapshot the GPU doesn't hold yet.
    fn upload_ripples(&mut self, ripples: Option<&crate::RippleMap>) {
        let Some(r) = ripples else { return };
        let key = (r.texels.as_ptr() as usize, r.version);
        let size = crate::water::RIPPLE_FIELD_SIZE;
        if self.ripple_field_key == Some(key)
            || r.size != size
            || r.texels.len() != (size * size * 4) as usize
        {
            return;
        }
        self.ripple_field_key = Some(key);
        self.ctx.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ripple_field_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&r.texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * size),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
    }

    fn water_bind_groups(
        &self,
        draw: &DrawList,
        beautiful: bool,
        time: f32,
    ) -> Vec<(wgpu::BindGroup, u32)> {
        draw.waters
            .iter()
            .map(|w| {
                let uniform =
                    WaterUniform::new(w, beautiful, time).with_ripples(draw.ripples.as_ref());
                let device = self.ctx.device();
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("water"),
                    contents: bytemuck::bytes_of(&uniform),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let ripples = &self.water_ripples.inner;
                let indices = if w.water.endless {
                    u32::MAX
                } else {
                    crate::water::grid_index_count()
                };
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("water"),
                    layout: &self.shared.water_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&ripples.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&ripples.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&self.ripple_field),
                        },
                    ],
                });
                (bind, indices)
            })
            .collect()
    }

    /// Builds the frame's GPU resources from the draw list: camera, model,
    /// light and water bind groups, the sprite batch, and the sun's shadow
    /// cascades.
    #[allow(clippy::too_many_lines)] // one step per resource, in order
    fn prepare_frame(
        &mut self,
        draw: &DrawList,
        viewport: (u32, u32),
        beautiful: bool,
    ) -> PreparedFrame {
        let camera_2d_bind = self.camera_bind_group(
            draw.camera_2d.map_or_else(
                || camera::screen_space_camera(viewport),
                |c| c.view_proj(viewport),
            ),
            glam::Vec3::ZERO,
            &[0.0; GRASS_UNIFORM_FLOATS],
        );
        let camera_3d = draw.camera_3d.unwrap_or_default();
        let env = draw.environment.unwrap_or_default();
        let time = draw
            .time
            .unwrap_or_else(|| self.clock.elapsed().as_secs_f32());
        let camera_3d_bind = self.camera_bind_group(
            camera_3d.view_proj(viewport),
            camera_3d.position,
            &grass_uniform(&env, time, &draw.grass_pushers, camera_3d.position),
        );

        let sprite_runs = batch_sprites(&draw.sprites);
        self.sprite_batch.write(self.ctx.queue(), &draw.sprites);

        // Skinned meshes' model and joint bind groups (shared by the shadow
        // and main passes); the rest draw instanced.
        let model_bind_groups = draw
            .meshes
            .iter()
            .map(|m| {
                m.joints
                    .as_ref()
                    .map(|joints| self.skinned_model_bind_group(m.model, joints))
            })
            .collect();

        // Sunlight: the draw list's sun, or a default shadowless light.
        let sun = draw.sun.unwrap_or_default();
        let (mesh_spheres, scene) = scene_bounds(draw);
        let direction = sun.normalized_direction();
        let aspect = viewport.0 as f32 / viewport.1.max(1) as f32;
        let shadows = ShadowCascades::fit(
            &camera_3d,
            aspect,
            scene,
            direction,
            self.shadow_map_size,
            self.quality.shadow_cascades() as usize,
            sun.shadow_distance,
        );
        let render_shadows = sun.shadows && self.shadows_enabled && !draw.meshes.is_empty();
        let (seen, main_batches, cascade_batches, grass_materials) = self.batch_frame(
            draw,
            &mesh_spheres,
            &camera_3d,
            viewport,
            render_shadows.then_some(&shadows),
        );
        let c = draw.clear_color;
        #[allow(clippy::cast_possible_truncation)] // colors fit f32
        let background = [c.r as f32, c.g as f32, c.b as f32];
        let frame = FrameView {
            camera: camera_3d,
            viewport,
            background,
            time,
            beautiful,
            point_shadow_taps: self.quality.point_shadow_taps(),
            water: crate::water::WaterView::new(
                &draw.waters,
                &camera_3d,
                time,
                self.quality != crate::GraphicsQuality::Low,
            ),
        };
        let points = frame_point_lights(
            draw,
            &camera_3d,
            self.quality.max_point_lights(),
            self.quality.point_shadows(),
        );
        let (light_bind, beautiful_light_bind) =
            self.light_bind_group(&sun, &env, &shadows, render_shadows, &frame, &points);
        self.upload_ripples(draw.ripples.as_ref());
        let water_binds = self.water_bind_groups(draw, beautiful, time);
        let forward = (camera_3d.target - camera_3d.position).normalize_or(glam::Vec3::NEG_Z);
        let particle_runs = self.particles.upload(
            self.ctx.device(),
            self.ctx.queue(),
            &draw.particles,
            camera_3d.position,
            forward,
        );

        PreparedFrame {
            camera_2d_bind,
            camera_3d_bind,
            model_bind_groups,
            seen,
            main_batches,
            cascade_batches,
            sprite_runs,
            light_bind,
            beautiful_light_bind,
            shadows,
            mesh_spheres,
            render_shadows,
            points,
            water_binds,
            particle_runs,
            grass_materials,
            camera_3d,
        }
    }

    /// Builds the frame's light bind group (uniform + shadow map array +
    /// comparison sampler). `render_shadows` gates shadow sampling in the
    /// shader — when false the shadow maps are bound but never read. On
    /// Beautiful, also a second group sharing it that adds the screen-space
    /// occlusion for meshes.
    fn light_bind_group(
        &self,
        sun: &Sun,
        env: &crate::Environment,
        shadows: &ShadowCascades,
        render_shadows: bool,
        frame: &FrameView,
        points: &[crate::light::FramePoint],
    ) -> (wgpu::BindGroup, Option<wgpu::BindGroup>) {
        let device = self.ctx.device();
        let light_uniform = LightUniform::new(
            &Sun {
                shadows: render_shadows,
                ..*sun
            },
            env,
            shadows,
            self.shadow_map_size,
            frame,
            points,
            self.point_shadows.size,
        );
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("light"),
            contents: bytemuck::bytes_of(&light_uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&self.shadow_array_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&self.point_shadows.array_view),
            },
        ];
        let light = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light"),
            layout: &self.shared.light_layout,
            entries: &entries,
        });
        let occlusion = self.ssao.output_view().filter(|_| frame.beautiful);
        let beautiful = occlusion.as_ref().map(|view| {
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(view),
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("beautiful light"),
                layout: &self.shared.beautiful_light_layout,
                entries: &entries,
            })
        });
        (light, beautiful)
    }

    /// Records the depth-only shadow passes: per cascade, every mesh that
    /// can cast into it, drawn from the sun's viewpoint into its layer.
    fn record_shadow_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
    ) {
        let shadows = &prepared.shadows;
        for (cascade, layer_view) in self
            .shadow_layer_views
            .iter()
            .enumerate()
            .take(shadows.count)
        {
            let light_vp_bind = self.uniform_bind_group(
                Some("shadow light"),
                bytemuck::cast_slice(&shadows.view_proj[cascade].to_cols_array()),
                &self.shared.shadow_light_layout,
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..wgpu::RenderPassDescriptor::default()
            });
            let Some(batches) = prepared.cascade_batches.get(cascade) else {
                continue;
            };
            pass.set_bind_group(0, &light_vp_bind, &[]);
            Self::record_depth_batches(
                &mut pass,
                draw,
                prepared,
                batches,
                &self.shadow_pipeline,
                &self.instance_buffer,
            );
        }
    }

    /// Culls and batches the frame's meshes: what the camera sees of each
    /// (level of detail, fade, or past its draw distance), the camera's
    /// batches, and each sun cascade's (`shadows`, when they render). Writes
    /// all their instances to [`Self::instance_buffer`].
    fn batch_frame(
        &mut self,
        draw: &DrawList,
        spheres: &[Sphere],
        camera: &crate::Camera3D,
        viewport: (u32, u32),
        shadows: Option<&ShadowCascades>,
    ) -> FrameBatches {
        use crate::batch::{batch, choose, Frustum, PassKind};
        let lod_scale = self.quality.lod_scale();
        let seen: Vec<_> = draw
            .meshes
            .iter()
            .zip(spheres)
            .map(|(m, s)| choose(m, *s, camera.position, lod_scale))
            .collect();
        let frustum = Frustum::new(&camera.view_proj(viewport));
        let mut instances = Vec::new();
        let visible = seen
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i, (*s)?)))
            .filter(|(i, _)| frustum.sees(spheres[*i]));
        let mut main = batch(&draw.meshes, visible, PassKind::Shaded, &mut instances);
        // Grass is solid: after the other solid batches, before see-through
        // ones. It casts no shadows.
        let (grass, grass_materials) = self.batch_grass(draw, &frustum, camera.position);
        let at = main
            .iter()
            .position(|b| b.transparent)
            .unwrap_or(main.len());
        main.splice(at..at, grass);
        let cascades = shadows.map_or_else(Vec::new, |shadows| {
            (0..shadows.count)
                .map(|cascade| {
                    let casters = seen
                        .iter()
                        .enumerate()
                        .filter_map(|(i, s)| Some((i, (*s)?)))
                        .filter(|(i, _)| shadows.touches(cascade, spheres[*i]));
                    batch(&draw.meshes, casters, PassKind::Depth, &mut instances)
                })
                .collect()
        });
        self.upload_instances(&instances);
        (seen, main, cascades, grass_materials)
    }

    /// The grass chunks the camera sees, as one batch each, nearest first:
    /// its baked blades (see [`BakedGrass`]), as many as its distance
    /// keeps, with five triangles a blade up close and one further off.
    /// Returns the
    /// batches and, per field with any in view, its material (the batches'
    /// `draw`).
    fn batch_grass(
        &mut self,
        draw: &DrawList,
        frustum: &crate::batch::Frustum,
        eye: glam::Vec3,
    ) -> (Vec<crate::batch::Batch>, Vec<Material>) {
        if draw.grass.is_empty() {
            self.grass_chunks.clear();
            return (Vec::new(), Vec::new());
        }
        self.grass_frame += 1;
        let frame = self.grass_frame;
        // Lower presets draw fewer blades.
        let density = self.quality.lod_scale().min(1.0);
        // (distance, batch): drawn nearest first, so the blades in front
        // hide the ones behind before they're shaded.
        let mut batches: Vec<(f32, crate::batch::Batch)> = Vec::new();
        let mut materials = Vec::new();
        for field in &draw.grass {
            let material = materials.len();
            let mut seen = false;
            for chunk in &field.chunks {
                let sphere = Sphere {
                    center: glam::Vec3::from(chunk.center),
                    radius: chunk.radius,
                };
                if !frustum.sees(sphere) {
                    continue;
                }
                let near = (sphere.center.distance(eye) - sphere.radius).max(0.0);
                let n = crate::grass::drawn_blades(chunk.blades.len(), near, field.draw_distance);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let n = ((n as f32 * density).ceil() as usize).min(n);
                if n == 0 {
                    continue;
                }
                let baked = self.baked_grass(chunk, frame);
                let (mesh, per_blade) = if near < crate::grass::FULL_BLADE_DISTANCE {
                    (baked.near, crate::grass::BLADE_INDICES.0)
                } else {
                    (baked.far, crate::grass::BLADE_INDICES.1)
                };
                batches.push((
                    near,
                    crate::batch::Batch {
                        kind: crate::batch::BatchKind::Grass,
                        draw: material,
                        mesh,
                        first: 0,
                        count: u32::try_from(n)
                            .unwrap_or(u32::MAX)
                            .saturating_mul(per_blade),
                        transparent: false,
                    },
                ));
                seen = true;
            }
            if seen {
                materials.push(self.grass_material(&field.look));
            }
        }
        // Forget chunks not drawn for a while: out of view for long, or
        // their field regrew (painted, moved) and they're gone.
        self.grass_chunks
            .retain(|_, baked| frame - baked.last_drawn < GRASS_KEEP_FRAMES);
        batches.sort_by(|a, b| a.0.total_cmp(&b.0));
        (batches.into_iter().map(|(_, b)| b).collect(), materials)
    }

    /// `chunk` baked into a vertex buffer (on first use), as the meshes
    /// that draw it up close and far off.
    fn baked_grass(&mut self, chunk: &Arc<crate::GrassChunk>, frame: u64) -> BakedGrass {
        let key = Arc::as_ptr(chunk) as usize;
        if let Some(baked) = self.grass_chunks.get_mut(&key) {
            baked.last_drawn = frame;
            return baked.clone();
        }
        let blades = u32::try_from(chunk.blades.len()).unwrap_or(u32::MAX);
        let indices = self.grass_indices_for(blades);
        let vertex_buffer =
            self.ctx
                .device()
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("grass chunk"),
                    contents: bytemuck::cast_slice(&crate::grass::bake(chunk)),
                    usage: wgpu::BufferUsages::VERTEX,
                });
        let mesh = |index_buffer: &wgpu::Buffer, per_blade: u32| Mesh {
            inner: Arc::new(crate::mesh::MeshInner {
                vertex_buffer: vertex_buffer.clone(),
                index_buffer: index_buffer.clone(),
                index_count: blades.saturating_mul(per_blade),
                skin_buffer: None,
                bounding_radius: chunk.radius,
                bounds: (glam::Vec3::ZERO, glam::Vec3::ZERO),
                key: 0,
            }),
        };
        let baked = BakedGrass {
            chunk: Arc::clone(chunk),
            near: mesh(&indices.near, crate::grass::BLADE_INDICES.0),
            far: mesh(&indices.far, crate::grass::BLADE_INDICES.1),
            last_drawn: frame,
        };
        self.grass_chunks.insert(key, baked.clone());
        baked
    }

    /// Index buffers covering at least `blades` blades (grown as needed;
    /// chunks baked earlier keep the smaller ones they had).
    fn grass_indices_for(&mut self, blades: u32) -> GrassIndices {
        if let Some(indices) = &self.grass_indices {
            if indices.blades >= blades {
                return indices.clone();
            }
        }
        let blades = blades.max(1024).next_power_of_two();
        let device = self.ctx.device();
        let buffer = |far: bool| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(if far {
                    "far grass indices"
                } else {
                    "grass indices"
                }),
                contents: bytemuck::cast_slice(&crate::grass::blade_indices(blades, far)),
                usage: wgpu::BufferUsages::INDEX,
            })
        };
        let indices = GrassIndices {
            near: buffer(false),
            far: buffer(true),
            blades,
        };
        self.grass_indices = Some(indices.clone());
        indices
    }

    /// The material for grass that looks like `look` (its colour palette),
    /// made once.
    fn grass_material(&mut self, look: &crate::GrassLook) -> Material {
        let key = look.key();
        if let Some(material) = self.grass_materials.get(&key) {
            return material.clone();
        }
        let (w, h) = crate::grass::PALETTE_SIZE;
        let texture = self.create_texture_rgba(w, h, &crate::grass::palette(look));
        let material = self.create_pbr_material(
            &crate::MaterialParams {
                roughness: 0.85,
                ..crate::MaterialParams::with_color([1.0; 4])
            },
            crate::MaterialTextures {
                base: Some(texture),
                ..crate::MaterialTextures::default()
            },
        );
        // Looks rarely change; a scene that keeps restyling its grass
        // shouldn't grow this forever.
        if self.grass_materials.len() > 64 {
            self.grass_materials.clear();
        }
        self.grass_materials.insert(key, material.clone());
        material
    }

    /// Writes `instances` to the instance buffer, growing it first if
    /// they don't fit.
    fn upload_instances(&mut self, instances: &[crate::batch::Instance]) {
        if instances.is_empty() {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(instances);
        if bytes.len() as u64 > self.instance_buffer.size() {
            let count = (instances.len() as u64).next_power_of_two();
            self.instance_buffer = instance_buffer(self.ctx.device(), count);
        }
        self.ctx
            .queue()
            .write_buffer(&self.instance_buffer, 0, bytes);
    }

    /// Which point shadow layers need drawing this frame: those whose view
    /// or casters (meshes, their transforms and poses) differ from what the
    /// layer holds. Lamps and glowing props mostly sit still among still
    /// scenery, so most frames redraw none; something moving near a light
    /// redraws only the faces that see it.
    fn stale_point_shadow_layers(
        &mut self,
        draw: &DrawList,
        prepared: &PreparedFrame,
    ) -> Vec<bool> {
        let layers = self.point_shadows.views.len();
        self.point_shadow_keys.resize(layers, None);
        let mut stale = vec![false; layers];
        for point in &prepared.points {
            let Some((first, view)) = &point.shadow else {
                continue;
            };
            for (face, vp) in view.faces.iter().take(view.count).enumerate() {
                let layer = first + face;
                if layer >= layers {
                    continue;
                }
                let key = point_face_key(draw, prepared, point, face, vp);
                if self.point_shadow_keys[layer] != Some(key) {
                    self.point_shadow_keys[layer] = Some(key);
                    stale[layer] = true;
                }
            }
        }
        stale
    }

    /// Records the point lights' shadow cubes: per shadowed light and
    /// `stale` face, every mesh in range inside the face's view, except the
    /// one the light glows from (its own surface would block all its
    /// light).
    fn record_point_shadow_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
        stale: &[bool],
    ) {
        // Every stale face's batches first, so one buffer holds all their
        // copies (most frames redraw none, and make none).
        let mut instances = Vec::new();
        let mut faces = Vec::new();
        for point in &prepared.points {
            let Some((first, view)) = &point.shadow else {
                continue;
            };
            for face in 0..view.count.min(view.faces.len()) {
                if stale.get(first + face).copied().unwrap_or(false) {
                    let casters = face_casters(prepared, point, face)
                        .filter_map(|i| Some((i, prepared.seen[i]?)));
                    let batches = crate::batch::batch(
                        &draw.meshes,
                        casters,
                        crate::batch::PassKind::Depth,
                        &mut instances,
                    );
                    faces.push(batches);
                }
            }
        }
        if faces.is_empty() {
            return;
        }
        let instance_buffer =
            self.ctx
                .device()
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("point shadow instances"),
                    // A copy of nothing still needs a buffer to bind.
                    contents: if instances.is_empty() {
                        &[0; std::mem::size_of::<crate::batch::Instance>()]
                    } else {
                        bytemuck::cast_slice(&instances)
                    },
                    usage: wgpu::BufferUsages::VERTEX,
                });
        let mut faces = faces.into_iter();
        for point in &prepared.points {
            let Some((first, view)) = &point.shadow else {
                continue;
            };
            for (face, vp) in view.faces.iter().take(view.count).enumerate() {
                if !stale.get(first + face).copied().unwrap_or(false) {
                    continue;
                }
                let Some(layer_view) = self.point_shadows.views.get(first + face) else {
                    continue;
                };
                let light_vp_bind = self.uniform_bind_group(
                    Some("point shadow face"),
                    bytemuck::cast_slice(&vp.to_cols_array()),
                    &self.shared.shadow_light_layout,
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("point shadow"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: layer_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..wgpu::RenderPassDescriptor::default()
                });
                let batches = faces.next().unwrap_or_default();
                pass.set_bind_group(0, &light_vp_bind, &[]);
                Self::record_depth_batches(
                    &mut pass,
                    draw,
                    prepared,
                    &batches,
                    &self.shadow_pipeline,
                    &instance_buffer,
                );
            }
        }
    }
}

/// A vertex buffer for `count` [`crate::batch::Instance`]s.
/// Frames a baked grass chunk stays on the GPU undrawn.
const GRASS_KEEP_FRAMES: u64 = 300;

/// A grass chunk on the GPU: a vertex buffer of its blades (see
/// [`crate::grass::bake`]) drawn by two meshes, five triangles a blade
/// ([`Self::near`]) or one ([`Self::far`]). A batch draws the first part of
/// either: the first blades, as many as the distance keeps.
#[derive(Clone)]
struct BakedGrass {
    /// Kept so its address (the cache key) isn't reused while cached.
    #[allow(dead_code)]
    chunk: Arc<crate::GrassChunk>,
    near: Mesh,
    far: Mesh,
    last_drawn: u64,
}

/// The index buffers every baked grass chunk draws with: the blade's
/// triangles over and over, for `blades` blades.
#[derive(Clone)]
struct GrassIndices {
    near: wgpu::Buffer,
    far: wgpu::Buffer,
    blades: u32,
}

/// What [`Renderer::batch_frame`] sorted a frame into: per mesh what the
/// camera saw of it, the camera's batches, each sun cascade's batches, and
/// the materials of the grass batches.
type FrameBatches = (
    Vec<Option<crate::batch::Seen>>,
    Vec<crate::batch::Batch>,
    Vec<Vec<crate::batch::Batch>>,
    Vec<Material>,
);

/// Floats of grass data at the end of the camera uniform: the wind, then
/// [`crate::MAX_GRASS_PUSHERS`] pushers (`wind` and `pushers` in
/// common.wgsl).
const GRASS_UNIFORM_FLOATS: usize = 4 + 4 * crate::MAX_GRASS_PUSHERS;

/// The grass part of the camera uniform: the wind's direction, strength
/// and the time, then the pushers nearest `eye` (the rest zero).
fn grass_uniform(
    env: &crate::Environment,
    time: f32,
    pushers: &[crate::GrassPusher],
    eye: glam::Vec3,
) -> [f32; GRASS_UNIFORM_FLOATS] {
    let mut out = [0.0; GRASS_UNIFORM_FLOATS];
    let angle = env.wind_direction.to_radians();
    out[..4].copy_from_slice(&[angle.cos(), angle.sin(), env.wind.clamp(0.0, 1.0), time]);
    let mut nearest: Vec<&crate::GrassPusher> = pushers.iter().collect();
    nearest.sort_by(|a, b| {
        a.position
            .distance_squared(eye)
            .total_cmp(&b.position.distance_squared(eye))
    });
    for (slot, p) in out[4..]
        .chunks_exact_mut(4)
        .zip(nearest.into_iter().take(crate::MAX_GRASS_PUSHERS))
    {
        slot.copy_from_slice(&[p.position.x, p.position.y, p.position.z, p.radius]);
    }
    out
}

fn instance_buffer(device: &wgpu::Device, count: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: count.max(1) * crate::batch::INSTANCE_SIZE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Creates the depth buffer for the given size (no-op for empty sizes).
/// The GPU side of [`crate::RippleMap`]: half-float RGBA, zero until the
/// first snapshot arrives.
fn create_ripple_field(device: &wgpu::Device) -> wgpu::Texture {
    let size = crate::water::RIPPLE_FIELD_SIZE;
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ripple field"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn create_depth_texture(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        // Sampled by soft particles, attached read-only in the same pass.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// A depth texture array rendered into one layer at a time and sampled
/// whole: the point lights' shadow cubes.
struct DepthArray {
    size: u32,
    array_view: wgpu::TextureView,
    views: Vec<wgpu::TextureView>,
}

impl DepthArray {
    /// Layers the preset's point shadows need (six per light; one stand-in
    /// layer for none, since the light bind group always binds the array).
    fn point_layers(quality: GraphicsQuality) -> usize {
        (quality.point_shadows() * 6).max(1)
    }

    fn for_point_shadows(device: &wgpu::Device, quality: GraphicsQuality) -> Self {
        let size = quality.point_shadow_size().max(1);
        let layers = u32::try_from(Self::point_layers(quality)).unwrap_or(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point shadow maps"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: pipeline::SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("point shadow array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..wgpu::TextureViewDescriptor::default()
        });
        let views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("point shadow face"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                })
            })
            .collect();
        Self {
            size,
            array_view,
            views,
        }
    }
}

/// The shadow map array (one layer per cascade), its views and comparison
/// sampler.
struct ShadowResources {
    texture: wgpu::Texture,
    sampler: wgpu::Sampler,
    array_view: wgpu::TextureView,
    layer_views: Vec<wgpu::TextureView>,
}

impl ShadowResources {
    fn new(device: &wgpu::Device, quality: GraphicsQuality) -> Self {
        let size = quality.shadow_map_size().max(1);
        let layers = quality.shadow_cascades().max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: pipeline::SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        // Every tap compares a bilinear 2x2 footprint (hardware PCF), so
        // few taps give smooth edges.
        let filter = wgpu::FilterMode::Linear;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow comparison"),
            mag_filter: filter,
            min_filter: filter,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..wgpu::SamplerDescriptor::default()
        });
        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow map array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..wgpu::TextureViewDescriptor::default()
        });
        let layer_views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow cascade"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                })
            })
            .collect();
        Self {
            texture,
            sampler,
            array_view,
            layer_views,
        }
    }
}

/// The meshes (by draw index) that point light `point`'s shadow face
/// `face` draws: those its view sees, leaving out the light's own glowing
/// body and fixtures wrapped around it.
fn face_casters<'a>(
    prepared: &'a PreparedFrame,
    point: &'a crate::light::FramePoint,
    face: usize,
) -> impl Iterator<Item = usize> + 'a {
    prepared
        .mesh_spheres
        .iter()
        .enumerate()
        .filter(move |(i, sphere)| {
            prepared.seen[*i].is_some()
                && point.source != Some(*i)
                && !point.light.inside_fixture(sphere.center, sphere.radius)
                && crate::light::PointShadow::light_face_sees(
                    &point.light,
                    face,
                    sphere.center,
                    sphere.radius,
                )
        })
        .map(|(i, _)| i)
}

/// A hash of what a point shadow face renders: its view and every caster
/// it draws (mesh, transform, pose). Equal keys draw identical depth.
fn point_face_key(
    draw: &DrawList,
    prepared: &PreparedFrame,
    point: &crate::light::FramePoint,
    face: usize,
    view_proj: &glam::Mat4,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let bits = |m: &glam::Mat4, h: &mut std::collections::hash_map::DefaultHasher| {
        for v in m.to_cols_array() {
            v.to_bits().hash(h);
        }
    };
    bits(view_proj, &mut h);
    for i in face_casters(prepared, point, face) {
        let m = &draw.meshes[i];
        i.hash(&mut h);
        let level = prepared.seen[i].map_or(0, |s| s.level);
        let mesh = crate::batch::mesh_at(m, level);
        std::sync::Arc::as_ptr(&mesh.inner).hash(&mut h);
        mesh.inner.index_count.hash(&mut h);
        bits(&m.model, &mut h);
        if let Some(joints) = &m.joints {
            for j in joints.iter() {
                bits(j, &mut h);
            }
        }
    }
    h.finish()
}

/// Forward-path mesh pipelines: their target format, the shadows they
/// sample, and the plain and skinned pipelines.
type ForwardPipelines = (
    wgpu::TextureFormat,
    pipeline::ForwardShadows,
    pipeline::PipelinePair,
);

/// The forward-path mesh pipelines for `format` that `quality`'s frames can
/// use and `existing` lacks.
fn create_forward_pipelines(
    device: &wgpu::Device,
    shared: &PipelineShared,
    format: wgpu::TextureFormat,
    quality: GraphicsQuality,
    existing: &[ForwardPipelines],
) -> Vec<ForwardPipelines> {
    pipeline::ForwardShadows::for_quality(quality)
        .into_iter()
        .filter(|s| !existing.iter().any(|(f, e, _)| *f == format && e == s))
        .map(|s| {
            let pair = pipeline::create_mesh_pipelines(device, shared, format, DEPTH_FORMAT, s);
            (format, s, pair)
        })
        .collect()
}

/// The scene bindings forward-path water uses: 1x1 stand-ins (blended
/// water never samples the scene behind it).
/// The light a glowing mesh draw gives off: a [`crate::PointLight`] at the
/// center of its world-space box, as bright as its emission over its
/// surface area. `None` for draws that don't glow (or don't light).
fn glow_light(m: &MeshDraw) -> Option<crate::PointLight> {
    let p = &m.material.inner.params;
    let shadows = p.glow_shadows;
    let strength = p.glow_light.max(0.0) * p.color[3].clamp(0.0, 1.0);
    let radiance = p.emissive.map(|c| c.max(0.0) * strength);
    if radiance.iter().all(|c| *c <= 1e-4) {
        return None;
    }
    let (lo, hi) = m.mesh.inner.bounds;
    let center = m.model.transform_point3((lo + hi) * 0.5);
    // The box's half size in world space: each world axis gathers the
    // model axes' contributions.
    let half = (hi - lo) * 0.5;
    let cols = [
        m.model.x_axis.truncate(),
        m.model.y_axis.truncate(),
        m.model.z_axis.truncate(),
    ];
    let world_half = cols[0].abs() * half.x + cols[1].abs() * half.y + cols[2].abs() * half.z;
    Some(crate::PointLight {
        shadows,
        ..crate::PointLight::from_glow(center, world_half, radiance)
    })
}

/// The point lights a frame shades: the draw list's (spotlights too) plus
/// one per glowing mesh, at most `max` of them, the first of those that
/// cast shadows getting shadow map layers (six each, one per spotlight, out
/// of six per point light in `shadows`). Those that cast
/// shadows getting a shadow slot. The ones that light the camera's
/// surroundings most come first: brightest over distance squared from the
/// camera, counting lights the camera is inside as at full strength.
fn frame_point_lights(
    draw: &DrawList,
    camera: &crate::Camera3D,
    max: usize,
    shadows: usize,
) -> Vec<crate::light::FramePoint> {
    let mut lights: Vec<crate::light::FramePoint> = draw
        .point_lights
        .iter()
        .copied()
        .filter(|l| l.peak() > 0.0)
        .map(|light| crate::light::FramePoint {
            light,
            source: None,
            shadow: None,
        })
        .chain(draw.meshes.iter().enumerate().filter_map(|(i, m)| {
            glow_light(m).map(|light| crate::light::FramePoint {
                light,
                source: Some(i),
                shadow: None,
            })
        }))
        .collect();
    let score = |l: &crate::PointLight| {
        let d = (l.position.distance(camera.position) - l.effective_range()).max(0.0);
        l.peak() / (1.0 + d * d)
    };
    lights.sort_by(|a, b| score(&b.light).total_cmp(&score(&a.light)));
    lights.truncate(max.min(crate::MAX_POINT_LIGHTS));
    // Shadow map layers: six per point light the preset shadows; a point
    // light takes six, a spotlight one.
    let budget = shadows.min(crate::MAX_POINT_SHADOWS) * 6;
    let mut used = 0;
    for p in &mut lights {
        let need = p.light.shadow_layers();
        if p.light.shadows && used + need <= budget {
            p.shadow = Some((used, crate::light::PointShadow::for_light(&p.light)));
            used += need;
        }
    }
    lights
}

/// The bounding box `(min, max)` of a mesh's vertices (zero when empty).
fn mesh_bounds(data: &MeshData) -> (glam::Vec3, glam::Vec3) {
    if data.vertices.is_empty() {
        return (glam::Vec3::ZERO, glam::Vec3::ZERO);
    }
    data.vertices.iter().fold(
        (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)),
        |(lo, hi), v| {
            let p = glam::Vec3::from(v.position);
            (lo.min(p), hi.max(p))
        },
    )
}

/// A bind group entry for a texture's view.
fn view_entry(binding: u32, t: &Texture) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(&t.inner.view),
    }
}

fn placeholder_scene_bind_group(device: &wgpu::Device, shared: &PipelineShared) -> wgpu::BindGroup {
    let texture = |label, format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let color = texture("scene placeholder", pipeline::HDR_FORMAT);
    let depth = texture("depth placeholder", pipeline::HDR_DEPTH_FORMAT);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene placeholder"),
        layout: &shared.scene_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &color.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &depth.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

/// Each mesh's world bounding sphere, and one sphere around them all
/// (the shadow casters).
fn scene_bounds(draw: &DrawList) -> (Vec<Sphere>, Sphere) {
    let spheres: Vec<Sphere> = draw
        .meshes
        .iter()
        .map(|mesh_draw| {
            // The model's origin, and its largest axis scale (no full
            // decomposition: this runs for every mesh every frame).
            let m = &mesh_draw.model;
            let scale = m
                .x_axis
                .truncate()
                .length_squared()
                .max(m.y_axis.truncate().length_squared())
                .max(m.z_axis.truncate().length_squared())
                .sqrt();
            Sphere {
                center: m.w_axis.truncate(),
                radius: posed_radius(mesh_draw) * scale,
            }
        })
        .collect();
    if spheres.is_empty() {
        let scene = Sphere {
            center: glam::Vec3::ZERO,
            radius: 1.0,
        };
        return (spheres, scene);
    }
    let mut min = glam::Vec3::splat(f32::MAX);
    let mut max = glam::Vec3::splat(f32::MIN);
    for s in &spheres {
        min = min.min(s.center - glam::Vec3::splat(s.radius));
        max = max.max(s.center + glam::Vec3::splat(s.radius));
    }
    let scene = Sphere {
        center: (min + max) * 0.5,
        radius: ((max - min) * 0.5).length(),
    };
    (spheres, scene)
}

/// The post chain's tone curve index. Beautiful renders in HDR, so an
/// environment without a curve gets Khronos PBR Neutral (index 4, shader
/// only): authored colors pass through unchanged and only highlights roll
/// off, instead of clipping the bright range bloom works with.
/// Beautiful's god rays and depth of field for `draw`, through its 3D
/// camera (none without one), with the god rays from `sun`.
fn beautiful_lens(draw: &DrawList, sun: Option<&crate::post::SunFlare>) -> crate::post::Lens {
    let (Some(env), Some(camera)) = (draw.environment, draw.camera_3d) else {
        return crate::post::Lens::default();
    };
    let height = -draw.sun.unwrap_or_default().normalized_direction().y;
    crate::post::Lens::new(&env, &camera, sun, height)
}

fn beautiful_tonemap(env: Option<crate::Environment>) -> f32 {
    const NEUTRAL: f32 = 4.0;
    match env.map(|e| e.tonemapping) {
        None | Some(crate::Tonemapping::None) => NEUTRAL,
        Some(t) => t.index(),
    }
}

/// A sphere around the model origin containing the draw's vertices. A
/// skinned vertex is a weighted blend of `joint × bind vertex`, so it lies
/// within the farthest joint's reach: its translation plus the bind radius
/// times its largest scale.
fn posed_radius(mesh_draw: &MeshDraw) -> f32 {
    let bind = mesh_draw.mesh.inner.bounding_radius;
    let Some(joints) = &mesh_draw.joints else {
        return bind;
    };
    joints
        .iter()
        .take(crate::MAX_JOINTS)
        .map(|j| {
            let reach = j
                .x_axis
                .truncate()
                .length()
                .max(j.y_axis.truncate().length())
                .max(j.z_axis.truncate().length());
            j.w_axis.truncate().length() + bind * reach
        })
        .fold(0.0_f32, f32::max)
}

/// One contiguous run of sprites sharing a texture.
struct SpriteRun {
    texture: Texture,
    first_index: u32,
    index_count: u32,
}

/// Groups consecutive sprites by texture identity to minimize bind group
/// changes. Sprites with the same texture storage batch together.
fn batch_sprites(sprites: &[Sprite]) -> Vec<SpriteRun> {
    let mut runs: Vec<SpriteRun> = Vec::new();
    for (i, sprite) in sprites.iter().enumerate() {
        let id = sprite.texture.batch_id();
        let first_index = u32::try_from(i * 6).expect("sprite batch fits u32");
        match runs.last_mut() {
            Some(run) if run.texture.batch_id() == id => {
                run.index_count += 6;
            }
            _ => runs.push(SpriteRun {
                texture: sprite.texture.clone(),
                first_index,
                index_count: 6,
            }),
        }
    }
    runs
}

/// Reusable sprite batch buffers.
struct SpriteBatch {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    // Bind groups cached by texture storage identity so unchanged sprite sets
    // don't recreate bind groups every frame.
    bind_group_cache: std::cell::RefCell<HashMap<usize, (Texture, wgpu::BindGroup)>>,
}

impl SpriteBatch {
    /// Maximum cached sprite-texture bind groups before the cache is cleared
    /// and rebuilt. Prevents unbounded growth across a long editing session.
    const BIND_GROUP_CACHE_LIMIT: usize = 512;

    fn new(device: &wgpu::Device) -> Self {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite vertices"),
            size: (crate::SPRITE_VERTEX_CAPACITY * std::mem::size_of::<SpriteVertex>())
                as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indices = pipeline::sprite_indices(crate::SPRITE_VERTEX_CAPACITY / 4);
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sprite indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertex_buffer,
            index_buffer,
            bind_group_cache: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// Expands sprites into the shared vertex buffer.
    fn write(&mut self, queue: &wgpu::Queue, sprites: &[Sprite]) {
        let mut vertices = Vec::with_capacity(sprites.len() * 4);
        for sprite in sprites {
            vertices.extend_from_slice(&pipeline::expand_sprite(
                sprite.position,
                sprite.size,
                sprite.rotation,
                sprite.color,
                sprite.uv,
            ));
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    /// Returns (creating and caching if needed) the texture bind group for a
    /// sprite run. The cache is bounded: past a threshold it is cleared and
    /// rebuilt, so a long session cycling many textures can't grow it without
    /// bound (cached entries are cheap to recreate).
    fn texture_bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture: &Texture,
    ) -> wgpu::BindGroup {
        let id = texture.batch_id();
        let mut cache = self.bind_group_cache.borrow_mut();
        if cache.len() >= Self::BIND_GROUP_CACHE_LIMIT {
            cache.clear();
        }
        cache
            .entry(id)
            .or_insert_with(|| {
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sprite texture"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture.inner.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&texture.inner.sampler),
                        },
                    ],
                });
                (texture.clone(), bind_group)
            })
            .1
            .clone()
    }
}

// `TextureInner` fields are read through the bind groups that reference them.
#[allow(dead_code)]
fn _assert_texture_inner_fields(_: &TextureInner) {}

/// Reads a render target texture back to the CPU as tightly packed RGBA8
/// (row-major, top-left origin), appending into `out`. Blocks until the GPU
/// readback completes. `out` is truncated to exactly `width*height*4`.
fn read_texture_rgba_into(
    renderer: &Renderer,
    target: &wgpu::Texture,
    size: (u32, u32),
    out: &mut Vec<u8>,
) {
    let device = renderer.context().device();
    let queue = renderer.context().queue();
    // Texture->buffer copies need rows padded to 256 bytes; arbitrary widths
    // (e.g. an editor viewport) would otherwise fail validation.
    let unpadded_row = 4 * size.0;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes_per_row = unpadded_row.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(bytes_per_row) * u64::from(size.1),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size.1),
            },
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    wgpu::util::DownloadBuffer::read_buffer(device, queue, &buffer.slice(..), move |r| {
        let _ = tx.send(r.map(|b| b.to_vec()));
    });
    // Drive the device until the readback callback fires. `Poll` alone can
    // stall on some drivers; alternate Poll with a blocking Wait.
    let data = loop {
        let _ = device.poll(wgpu::PollType::Poll);
        if let Ok(received) = rx.try_recv() {
            break received.expect("readback failed");
        }
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_millis(10)),
        });
        if let Ok(received) = rx.try_recv() {
            break received.expect("readback failed");
        }
    };
    out.clear();
    if bytes_per_row == unpadded_row {
        out.extend_from_slice(&data);
    } else {
        out.reserve((unpadded_row * size.1) as usize);
        for row in data.chunks_exact(bytes_per_row as usize) {
            out.extend_from_slice(&row[..unpadded_row as usize]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera3D;
    use crate::draw::Sprite;
    use crate::mesh::MeshData;
    use crate::Color;

    /// Creates an offscreen renderer, or `None` when no GPU/software adapter
    /// exists (CI/sandbox without lavapipe).
    #[test]
    fn offscreen_skinned_mesh_follows_its_joints() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping skinned render test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        // A cube bound fully to joint 1; joint 0 is unused.
        let data = MeshData::cube(1.0);
        let skin = vec![
            SkinVertex {
                joints: [1, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            data.vertices.len()
        ];
        let cube = renderer.create_skinned_mesh(&data, &skin);
        assert!(cube.is_skinned());
        let material = renderer.create_material([1.0, 1.0, 1.0, 1.0], None);
        let camera = Camera3D::new(glam::Vec3::new(0.0, 0.0, 5.0), glam::Vec3::ZERO);

        let lit = |renderer: &mut Renderer, joint: glam::Mat4| {
            let joints: Arc<[glam::Mat4]> = vec![glam::Mat4::IDENTITY, joint].into();
            let mut draw = DrawList::new();
            // Black, so the default (sRGB-brightened) clear can't pass as lit.
            draw.set_clear_color(Color::BLACK);
            draw.set_camera_3d(camera);
            draw.skinned_mesh(&cube, glam::Mat4::IDENTITY, &material, &joints);
            renderer.render_offscreen(&draw, &target);
            let pixels = read_pixels(renderer, &target, size);
            let c = center_pixel(&pixels, size);
            c[0] > 20 || c[1] > 20 || c[2] > 20
        };
        assert!(
            lit(&mut renderer, glam::Mat4::IDENTITY),
            "at rest the cube covers the centre"
        );
        let moved = glam::Mat4::from_translation(glam::Vec3::new(3.0, 0.0, 0.0));
        assert!(!lit(&mut renderer, moved), "the joint moved the cube away");
    }

    #[test]
    fn offscreen_composite_shows_the_scene_through_the_overlay() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping composite test");
            return;
        };
        // An 8x8 red scene.
        let scene = renderer.create_render_target((8, 8));
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::RED);
        renderer.render_offscreen(&draw, &scene);
        // A 16x16 overlay: transparent, with an opaque blue first column
        // and a half-transparent premultiplied white pixel at (5, 5).
        let mut overlay = vec![0_u8; 16 * 16 * 4];
        for y in 0..16 {
            overlay[y * 64..y * 64 + 4].copy_from_slice(&[0, 0, 255, 255]);
        }
        let half = (5 * 16 + 5) * 4;
        overlay[half..half + 4].copy_from_slice(&[128, 128, 128, 128]);
        let target = renderer.create_render_target((16, 16));
        renderer.composite_offscreen(&overlay, &target, Some((&scene, (4, 4))));
        let pixels = read_pixels(&renderer, &target, (16, 16));
        let at = |x: usize, y: usize| {
            let i = (y * 16 + x) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
        assert!(near(at(0, 8), [0, 0, 255]), "overlay: {:?}", at(0, 8));
        assert!(near(at(8, 8), [255, 0, 0]), "scene: {:?}", at(8, 8));
        assert!(near(at(14, 1), [0, 0, 0]), "uncovered: {:?}", at(14, 1));
        // Half white over red: (128 + 255 / 2, 128, 128) in sRGB bytes.
        assert!(near(at(5, 5), [255, 128, 128]), "blend: {:?}", at(5, 5));
    }

    async fn offscreen() -> Option<Renderer> {
        let renderer = Renderer::new_offscreen((64, 64)).await.ok()?;
        eprintln!(
            "offscreen GPU test running on adapter-backed device (size {:?})",
            renderer.size()
        );
        Some(renderer)
    }

    /// Creates the RGBA8 render target for offscreen tests.
    fn target(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Reads the full target texture back to CPU memory.
    fn read_pixels(renderer: &Renderer, target: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
        renderer.read_texture_rgba(target, size)
    }

    /// The center pixel of a rendered target.
    fn center_pixel(pixels: &[u8], size: (u32, u32)) -> [u8; 4] {
        let (w, h) = (size.0 as usize, size.1 as usize);
        let offset = ((h / 2) * w + (w / 2)) * 4;
        [
            pixels[offset],
            pixels[offset + 1],
            pixels[offset + 2],
            pixels[offset + 3],
        ]
    }

    #[test]
    fn offscreen_clear_color_renders() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping offscreen render test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);

        let mut draw = DrawList::new();
        draw.set_clear_color(Color::RED);
        renderer.render_offscreen(&draw, &target);

        let pixels = read_pixels(&renderer, &target, size);
        let [r, _g, _b, a] = center_pixel(&pixels, size);
        assert!(r > 200, "expected red center, got {pixels:?}");
        assert_eq!(a, 255);
    }

    /// A window that has no handle (yet): enough to call `resume` with.
    struct NoWindow;

    impl wgpu::rwh::HasWindowHandle for NoWindow {
        fn window_handle(
            &self,
        ) -> std::result::Result<wgpu::rwh::WindowHandle<'_>, wgpu::rwh::HandleError> {
            Err(wgpu::rwh::HandleError::Unavailable)
        }
    }

    impl wgpu::rwh::HasDisplayHandle for NoWindow {
        fn display_handle(
            &self,
        ) -> std::result::Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
            Err(wgpu::rwh::HandleError::Unavailable)
        }
    }

    #[test]
    fn offscreen_renderers_have_no_surface_to_suspend_or_resume() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping suspend test");
            return;
        };
        assert!(!renderer.has_surface());
        renderer.suspend(); // nothing to let go of
        let size = renderer.size();
        assert!(matches!(
            renderer.resume(NoWindow, (32, 32)),
            Err(crate::Error::IncompatibleSurface)
        ));
        assert_eq!(renderer.size(), size, "a refused resume changes nothing");
        // It still draws offscreen afterwards.
        let target = target(renderer.context().device(), size);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::RED);
        renderer.render_offscreen(&draw, &target);
        let [r, ..] = center_pixel(&read_pixels(&renderer, &target, size), size);
        assert!(r > 200);
    }

    #[test]
    fn offscreen_mesh_draws_to_depth() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping mesh render test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);

        let cube = renderer.create_mesh(&MeshData::cube(2.0));
        let material = renderer.create_material([1.0, 1.0, 1.0, 1.0], None);

        let mut draw = DrawList::new();
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 5.0),
            glam::Vec3::ZERO,
        ));
        draw.mesh(&cube, glam::Mat4::IDENTITY, &material);
        renderer.render_offscreen(&draw, &target);

        let pixels = read_pixels(&renderer, &target, size);
        let center = center_pixel(&pixels, size);
        // The white Lambert-shaded cube should not be black at the center.
        assert!(
            center[0] > 20 || center[1] > 20 || center[2] > 20,
            "expected lit cube at center, got {center:?}"
        );
    }

    #[test]
    fn offscreen_levels_of_detail_swap_meshes_and_fade_out_at_the_draw_distance() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping LOD test");
            return;
        };
        renderer.set_quality(GraphicsQuality::High); // LOD scale 1
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let full = renderer.create_mesh(&MeshData::cube(4.0));
        let small = renderer.create_mesh(&MeshData::cube(1.0));
        let material = renderer.create_material([1.0, 1.0, 1.0, 1.0], None);
        let swaps = Arc::new(crate::MeshLod {
            levels: vec![(small, 8.0)],
            draw_distance: 0.0,
        });
        let fades = Arc::new(crate::MeshLod {
            levels: Vec::new(),
            draw_distance: 20.0,
        });
        // Lit pixels with the camera `distance` away.
        let mut lit = |distance: f32, lod: Option<&Arc<crate::MeshLod>>| {
            let mut draw = DrawList::new();
            draw.set_clear_color(Color::BLACK);
            draw.set_camera_3d(Camera3D::new(
                glam::Vec3::new(0.0, 0.0, distance),
                glam::Vec3::ZERO,
            ));
            // Several copies, so they draw as one instanced batch.
            for x in [-40.0, 0.0, 40.0] {
                let at = glam::Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0));
                draw.mesh(&full, at, &material);
                if let Some(lod) = lod {
                    draw.set_lod(lod);
                }
            }
            renderer.render_offscreen(&draw, &target);
            read_pixels(&renderer, &target, size)
                .chunks_exact(4)
                .filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20)
                .count()
        };
        // Near: the mesh itself.
        assert_eq!(lit(6.0, Some(&swaps)), lit(6.0, None));
        // Past 8 m: the small level, a sixteenth of the area.
        let (plain, simple) = (lit(12.0, None), lit(12.0, Some(&swaps)));
        assert!(plain > 200, "the cube shows at 12 m ({plain} px)");
        assert!(
            simple > 0 && simple * 6 < plain,
            "the small level draws ({simple} vs {plain} px)"
        );
        // Up to the last tenth before 20 m it all shows; then it dithers.
        assert_eq!(lit(17.0, Some(&fades)), lit(17.0, None));
        let (plain, fading) = (lit(19.0, None), lit(19.0, Some(&fades)));
        assert!(
            fading > 0 && fading * 4 < plain * 3,
            "part of it shows while fading ({fading} vs {plain} px)"
        );
        // Past the draw distance: gone.
        assert!(lit(21.0, None) > 0);
        assert_eq!(lit(21.0, Some(&fades)), 0);
    }

    /// A square of grass `size` metres across, `n` blades, drawn to
    /// `draw_distance`, as one chunk in random order.
    fn grass_field(n: usize, size: f32, draw_distance: f32) -> crate::GrassField {
        let mut seed = 0x2545_f491_u32;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32) / (u32::MAX as f32)
        };
        let blades = (0..n)
            .map(|i| {
                let root = glam::Vec3::new((random() - 0.5) * size, 0.0, (random() - 0.5) * size);
                crate::GrassBlade::new(
                    root,
                    glam::Vec3::Y,
                    random() * std::f32::consts::TAU,
                    0.08,
                    0.7,
                    random(),
                    i as f32 / n as f32,
                    draw_distance,
                )
            })
            .collect();
        crate::GrassField {
            chunks: vec![Arc::new(crate::GrassChunk::new(blades, 1.0))],
            look: crate::GrassLook::default(),
            draw_distance,
        }
    }

    #[test]
    fn offscreen_grass_sways_bends_away_from_pushers_and_thins_with_distance() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping grass test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let green = |px: &[u8]| {
            px.chunks_exact(4)
                .filter(|p| p[1] > 12 && p[1] > p[0] && p[1] > p[2])
                .count()
        };
        let frame = |renderer: &mut Renderer,
                     field: &crate::GrassField,
                     eye: glam::Vec3,
                     wind: f32,
                     pusher: Option<glam::Vec3>| {
            let mut draw = DrawList::new();
            draw.set_clear_color(Color::BLACK);
            draw.set_time(3.0);
            draw.set_camera_3d(Camera3D::new(eye, glam::Vec3::new(0.0, 0.3, 0.0)));
            draw.set_sun(crate::Sun::new(glam::Vec3::new(-0.3, -1.0, -0.4)));
            draw.set_environment(crate::Environment {
                wind,
                wind_direction: 0.0,
                ..crate::Environment::default()
            });
            draw.grass(field.clone());
            if let Some(at) = pusher {
                draw.push_grass(crate::GrassPusher {
                    position: at,
                    radius: 0.8,
                });
            }
            renderer.render_offscreen(&draw, &target);
            read_pixels(renderer, &target, size)
        };
        let differing = |a: &[u8], b: &[u8]| {
            a.chunks_exact(4)
                .zip(b.chunks_exact(4))
                .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 24))
                .count()
        };
        let near = glam::Vec3::new(0.0, 1.6, 3.5);
        let field = grass_field(3000, 4.0, 0.0);
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::Medium,
            GraphicsQuality::High,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let still = frame(&mut renderer, &field, near, 0.0, None);
            assert!(
                green(&still) > 600,
                "{quality:?}: a green field ({} px)",
                green(&still)
            );
            let windy = frame(&mut renderer, &field, near, 1.0, None);
            assert!(
                differing(&still, &windy) > 150,
                "{quality:?}: the wind bends it ({} px moved)",
                differing(&still, &windy)
            );
            let pushed = frame(&mut renderer, &field, near, 0.0, Some(glam::Vec3::ZERO));
            assert!(
                differing(&still, &pushed) > 80,
                "{quality:?}: a pusher parts it ({} px moved)",
                differing(&still, &pushed)
            );
        }
        renderer.set_quality(GraphicsQuality::High);
        // Far away a field with a draw distance is gone, and one without
        // stays.
        let far = glam::Vec3::new(0.0, 3.0, 24.0);
        let unlimited = green(&frame(&mut renderer, &field, far, 0.0, None));
        assert!(unlimited > 10, "seen from afar ({unlimited} px)");
        let limited = grass_field(3000, 4.0, 18.0);
        assert_eq!(green(&frame(&mut renderer, &limited, far, 0.0, None)), 0);
        // Half way to it, about half the blades draw (see `drawn_blades`),
        // widened so the field still covers the ground.
        let mid = glam::Vec3::new(0.0, 2.0, 11.0);
        let (all, thinned) = (
            green(&frame(&mut renderer, &field, mid, 0.0, None)),
            green(&frame(&mut renderer, &limited, mid, 0.0, None)),
        );
        assert!(
            thinned * 10 > all * 7,
            "thinned but still full ({thinned} of {all} px)"
        );
    }

    #[test]
    fn offscreen_material_tiling_repeats_its_textures() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping material tiling test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        // Two texels, black then white, sampled crisply.
        let texture =
            renderer.create_texture_rgba_mipmapped(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255], true);
        let quad = renderer.create_mesh(&MeshData::quad(2.0));
        // Black-to-white edges along the middle row, ignoring the red
        // background around the quad.
        let mut edges = |uv_scale: [f32; 2]| {
            let material = renderer.create_pbr_material(
                &crate::MaterialParams {
                    uv_scale,
                    ..crate::MaterialParams::default()
                },
                crate::MaterialTextures {
                    base: Some(texture.clone()),
                    ..crate::MaterialTextures::default()
                },
            );
            let mut draw = DrawList::new();
            draw.set_clear_color(Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            });
            draw.set_camera_3d(Camera3D::new(
                glam::Vec3::new(0.0, 0.0, 3.0),
                glam::Vec3::ZERO,
            ));
            draw.mesh(&quad, glam::Mat4::IDENTITY, &material);
            renderer.render_offscreen(&draw, &target);
            let pixels = read_pixels(&renderer, &target, size);
            let (width, row) = (size.0 as usize, size.1 as usize / 2);
            let mut last = None;
            let mut edges = 0;
            for col in 0..width {
                let px = &pixels[(row * width + col) * 4..][..4];
                let (red, green) = (px[0], px[1]);
                let bright = if red > 150 && green < 40 {
                    continue; // background
                } else if green > 60 {
                    true
                } else if red < 40 && green < 40 {
                    false
                } else {
                    continue; // a blended edge pixel
                };
                if last.is_some_and(|l| l != bright) {
                    edges += 1;
                }
                last = Some(bright);
            }
            edges
        };
        assert_eq!(edges([1.0, 1.0]), 1, "black half, white half");
        assert_eq!(edges([2.0, 1.0]), 3, "tiled twice across: four stripes");
    }

    #[test]
    fn offscreen_sprite_draws_textured_quad() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping sprite render test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);

        // A 4x4 all-red texture sprite covering the center of the screen.
        let texture = renderer.create_texture_rgba(4, 4, &[255u8, 0, 0, 255].repeat(16));
        let mut draw = DrawList::new();
        draw.sprite(Sprite::new(texture).position(glam::Vec2::new(32.0, 32.0)));
        renderer.render_offscreen(&draw, &target);

        let pixels = read_pixels(&renderer, &target, size);
        let center = center_pixel(&pixels, size);
        assert!(
            center[0] > 200 && center[1] < 60,
            "expected red sprite at center, got {center:?}"
        );
    }

    #[test]
    fn offscreen_hud_text_draws_glyphs_from_the_atlas() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping HUD text render test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let mut atlas = crate::text::TextAtlas::new();
        atlas.prepare(&renderer, [("H", 48.0, crate::text::FontId::BUILTIN)]);
        let style = crate::text::TextStyle {
            size: 48.0,
            color: [1.0, 0.0, 0.0, 1.0],
            align: crate::text::TextAlign::Center,
            ..crate::text::TextStyle::default()
        };
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        for sprite in atlas.sprites("H", 32.0, 32.0, &style) {
            draw.sprite(sprite);
        }
        renderer.render_offscreen(&draw, &target);
        let pixels = read_pixels(&renderer, &target, size);
        let red = |x: usize, y: usize| pixels[(y * 64 + x) * 4];
        // An "H" centred on the target: red down both stems, black in the
        // gaps above and below the bar, and black in the corners.
        let row: Vec<u8> = (0..64).map(|x| red(x, 24)).collect();
        let lit: Vec<usize> = (0..64).filter(|&x| row[x] > 128).collect();
        assert!(lit.len() >= 4, "two stems on row 24: {row:?}");
        let (first, last) = (lit[0], lit[lit.len() - 1]);
        assert!(
            first < 30 && last > 34,
            "stems either side of centre: {row:?}"
        );
        assert!(row[32] < 64, "the gap between the stems: {row:?}");
        assert!(red(2, 2) < 16 && red(61, 61) < 16, "corners stay black");
        let bar: Vec<u8> = (0..64).map(|y| red(32, y)).collect();
        assert!(bar.iter().any(|&v| v > 128), "the crossbar: {bar:?}");
    }

    #[test]
    fn offscreen_hud_text_draws_a_font_file_and_falls_back_for_missing_glyphs() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping font render test");
            return;
        };
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let mut atlas = crate::text::TextAtlas::new();
        let font = atlas.font(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/oberon/pause_menu/fonts/PressStart2P-Regular.ttf"),
        );
        assert_eq!(atlas.take_font_errors(), Vec::<String>::new());
        // "ẞ" isn't in the pixel font, so it's drawn with the built-in one.
        let text = "I\u{1e9e}";
        atlas.prepare(&renderer, [(text, 24.0, font)]);
        let style = crate::text::TextStyle {
            size: 24.0,
            color: [1.0, 0.0, 0.0, 1.0],
            font,
            ..crate::text::TextStyle::default()
        };
        let sprites = atlas.sprites(text, 8.0, 32.0, &style);
        assert_eq!(sprites.len(), 2, "both glyphs drawn");
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        for sprite in sprites {
            draw.sprite(sprite);
        }
        renderer.render_offscreen(&draw, &target);
        let pixels = read_pixels(&renderer, &target, size);
        let lit = |x0: usize, x1: usize| {
            (x0..x1)
                .flat_map(|x| (20..44).map(move |y| (x, y)))
                .filter(|&(x, y)| pixels[(y * 64 + x) * 4] > 128)
                .count()
        };
        // Press Start 2P's glyphs are 24 px wide at this size: "I" in the
        // first cell, the fallback glyph after it.
        assert!(lit(8, 32) > 20, "the pixel-font I");
        assert!(lit(32, 56) > 20, "the fallback glyph");
        // Pixel-font edges are hard: almost every lit pixel is fully lit.
        let soft = (8..32)
            .flat_map(|x| (20..44).map(move |y| (x, y)))
            .filter(|&(x, y)| (32..224).contains(&pixels[(y * 64 + x) * 4]))
            .count();
        assert!(soft * 4 < lit(8, 32), "{soft} soft pixels");
    }

    /// Renders the shadow test scene with the given sun, returning each
    /// pixel's brightness (R + G + B).
    fn shadow_scene_brightness(renderer: &mut Renderer, sun: crate::Sun) -> Vec<u32> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);

        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let ground_mat = renderer.create_material([0.8, 0.8, 0.8, 1.0], None);
        let block_mat = renderer.create_material([1.0, 1.0, 1.0, 1.0], None);

        let mut draw = DrawList::new();
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 6.0, 4.0),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(sun);
        // Wide flat ground, with a tall blocker casting a shadow onto it.
        draw.mesh(
            &cube,
            glam::Mat4::from_scale(glam::Vec3::new(20.0, 0.2, 20.0)),
            &ground_mat,
        );
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::splat(2.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, 1.6, 0.0),
            ),
            &block_mat,
        );
        renderer.render_offscreen(&draw, &target);

        read_pixels(renderer, &target, size)
            .chunks_exact(4)
            .map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]))
            .collect()
    }

    #[test]
    fn offscreen_shadow_darkens_occluded_ground() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping shadow render test");
            return;
        };

        // A low sun behind the blocker throws a long shadow across the
        // ground towards the camera, where it isn't hidden by the blocker.
        let sun = crate::Sun::new(glam::Vec3::new(-1.0, -0.5, 0.5));
        let lit = shadow_scene_brightness(&mut renderer, sun);
        let shadowed = shadow_scene_brightness(
            &mut renderer,
            crate::Sun {
                shadows: true,
                ..sun
            },
        );

        // Shadows only ever remove sunlight: a good share of the image gets
        // clearly darker, and nothing gets brighter.
        let darker = lit
            .iter()
            .zip(&shadowed)
            .filter(|(l, s)| **s + 45 < **l)
            .count();
        let brighter = lit
            .iter()
            .zip(&shadowed)
            .filter(|(l, s)| **s > **l + 45)
            .count();
        let pixels = lit.len();
        assert!(
            darker * 25 >= pixels,
            "the shadow should cover at least 4% of the image: {darker} of {pixels} pixels darker"
        );
        assert!(
            brighter * 100 <= pixels,
            "shadows made {brighter} of {pixels} pixels brighter"
        );
    }

    #[test]
    fn offscreen_beautiful_shadow_darkens_occluded_ground() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping Beautiful shadow test");
            return;
        };
        renderer.set_quality(GraphicsQuality::Beautiful);
        assert_eq!(renderer.quality(), GraphicsQuality::Beautiful);
        // Cascades, soft filtering, HDR, bloom and FXAA all on: shadows
        // still only remove light.
        let sun = crate::Sun::new(glam::Vec3::new(-1.0, -0.5, 0.5));
        let lit = shadow_scene_brightness(&mut renderer, sun);
        let shadowed = shadow_scene_brightness(
            &mut renderer,
            crate::Sun {
                shadows: true,
                ..sun
            },
        );
        let darker = lit
            .iter()
            .zip(&shadowed)
            .filter(|(l, s)| **s + 45 < **l)
            .count();
        assert!(
            darker * 25 >= lit.len(),
            "the shadow should cover at least 4% of the image: {darker} darker"
        );
        assert!(lit.iter().any(|&b| b > 150), "the scene is lit");
    }

    /// Brightness (r + g + b) of a box standing on the ground lit by
    /// ambient light only, with ambient occlusion strength `ao`.
    fn occlusion_scene_brightness(renderer: &mut Renderer, ao: f32) -> Vec<u32> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let white = renderer.create_material([1.0, 1.0, 1.0, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(wgpu::Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 4.0, 5.0),
            glam::Vec3::new(0.0, 0.5, 0.0),
        ));
        // No sunlight, only the (white) ambient: what AO darkens.
        draw.set_sun(crate::Sun {
            color: [0.0, 0.0, 0.0],
            ambient: 0.8,
            ..crate::Sun::default()
        });
        draw.set_environment(crate::Environment {
            ambient_occlusion: ao,
            ..crate::Environment::default()
        });
        draw.mesh(
            &cube,
            glam::Mat4::from_scale(glam::Vec3::new(20.0, 0.2, 20.0)),
            &white,
        );
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::splat(2.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, 1.1, 0.0),
            ),
            &white,
        );
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
            .chunks_exact(4)
            .map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]))
            .collect()
    }

    #[test]
    fn offscreen_ambient_occlusion_darkens_creases_on_beautiful_only() {
        // Big enough that the half-resolution occlusion resolves the crease.
        let Some(mut renderer) = pollster::block_on(Renderer::new_offscreen((160, 120))).ok()
        else {
            eprintln!("no GPU adapter; skipping ambient occlusion test");
            return;
        };
        renderer.set_quality(GraphicsQuality::Beautiful);
        let open = occlusion_scene_brightness(&mut renderer, 0.0);
        let occluded = occlusion_scene_brightness(&mut renderer, 1.0);
        let (w, _) = renderer.size();
        assert!(
            open.iter().zip(&occluded).all(|(o, c)| *c <= *o + 6),
            "occlusion only removes light"
        );
        let darker = open
            .iter()
            .zip(&occluded)
            .filter(|(o, c)| **c + 60 < **o)
            .count();
        assert!(
            darker * 50 >= open.len(),
            "the crease around the box gets clearly darker: {darker} pixels"
        );
        // Open ground far from the box stays as it was.
        let corner = (w as usize) * 118 + 2;
        assert!(
            occluded[corner] + 12 >= open[corner],
            "{} vs {}",
            occluded[corner],
            open[corner]
        );

        // High has no ambient occlusion: the setting changes nothing.
        renderer.set_quality(GraphicsQuality::High);
        assert_eq!(
            occlusion_scene_brightness(&mut renderer, 0.0),
            occlusion_scene_brightness(&mut renderer, 2.0)
        );
    }

    #[test]
    fn offscreen_cascaded_shadows_end_at_the_shadow_distance() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping shadow distance test");
            return;
        };
        let sun = crate::Sun::new(glam::Vec3::new(-1.0, -0.5, 0.5));
        let darker = |lit: &[u32], shadowed: &[u32]| {
            lit.iter()
                .zip(shadowed)
                .filter(|(l, s)| **s + 45 < **l)
                .count()
        };
        for quality in [
            GraphicsQuality::Medium,
            GraphicsQuality::High,
            GraphicsQuality::Ultra,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let lit = shadow_scene_brightness(&mut renderer, sun);
            let near = shadow_scene_brightness(
                &mut renderer,
                crate::Sun {
                    shadows: true,
                    ..sun
                },
            );
            // The shadow lies ~7 m from the camera: past a 3 m distance.
            let short = shadow_scene_brightness(
                &mut renderer,
                crate::Sun {
                    shadows: true,
                    shadow_distance: 3.0,
                    ..sun
                },
            );
            let pixels = lit.len();
            assert!(
                darker(&lit, &near) * 25 >= pixels,
                "{quality:?} draws the shadow"
            );
            // (Medium's single map follows the view too once that's
            // smaller than the whole scene.)
            assert!(
                darker(&lit, &short) * 200 <= pixels,
                "{quality:?}: no shadow past the shadow distance"
            );
        }
    }

    #[test]
    fn offscreen_quality_round_trips_through_beautiful() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping quality switch test");
            return;
        };
        for quality in GraphicsQuality::ALL {
            renderer.set_quality(quality);
            assert_eq!(renderer.quality(), quality);
            let lit = shadow_scene_brightness(
                &mut renderer,
                crate::Sun {
                    shadows: true,
                    ..crate::Sun::default()
                },
            );
            assert!(lit.iter().any(|&b| b > 60), "{quality:?} draws the scene");
        }
        renderer.set_quality(GraphicsQuality::Beautiful);
        assert!(
            renderer.post.targets.is_some(),
            "Beautiful drew through HDR targets"
        );
        renderer.set_quality(GraphicsQuality::High);
        assert!(
            renderer.post.targets.is_none(),
            "leaving Beautiful frees HDR targets"
        );
    }

    /// Renders a water plane seen from above over a dark floor, returning
    /// the center pixel.
    fn water_center(renderer: &mut Renderer) -> [u8; 4] {
        let size = renderer.size();
        center_pixel(&water_frame(renderer, None), size)
    }

    /// A 20 m pool over a dark floor, seen from above, with `ripples`.
    fn water_frame(renderer: &mut Renderer, ripples: Option<crate::RippleMap>) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let floor = renderer.create_material([0.05, 0.05, 0.05, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_time(1.0);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 6.0, 3.0),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun::new(glam::Vec3::new(-0.3, -1.0, -0.2)));
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(20.0, 0.2, 20.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, -1.0, 0.0),
            ),
            &floor,
        );
        let water = crate::Water {
            size: [20.0, 20.0],
            ..crate::Water::default()
        };
        draw.water(water, glam::Mat4::IDENTITY);
        if let Some(ripples) = ripples {
            draw.set_ripples(ripples);
        }
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    /// A grey floor 3 m down in a 20 m pool, seen from a camera 1 m under
    /// the surface, at `time` s with caustics of strength `caustics`.
    fn dive_frame(renderer: &mut Renderer, caustics: f32, time: f32) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let floor = renderer.create_material([0.6, 0.6, 0.6, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_time(time);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, -1.0, 3.0),
            glam::Vec3::new(0.0, -3.0, 0.0),
        ));
        draw.set_sun(crate::Sun::new(glam::Vec3::new(-0.3, -1.0, -0.2)));
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(20.0, 0.2, 20.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, -3.1, 0.0),
            ),
            &floor,
        );
        let water = crate::Water {
            size: [20.0, 20.0],
            caustics,
            ..crate::Water::default()
        };
        draw.water(water, glam::Mat4::IDENTITY);
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    /// A ripple field with a ring of `height` (m) and foam `foam` around
    /// the world origin, as the physics simulation would hand it over.
    #[allow(clippy::many_single_char_names)]
    fn ring_ripples(height: f32, foam: f32, version: u64) -> crate::RippleMap {
        let n = crate::RIPPLE_FIELD_SIZE as usize;
        let cell = 0.25;
        let mut h = vec![0.0; n * n];
        let mut f = vec![0.0; n * n];
        for z in 0..n {
            for x in 0..n {
                #[allow(clippy::cast_precision_loss)] // small
                let p = glam::Vec2::new(x as f32 + 0.5, z as f32 + 0.5) * cell
                    - glam::Vec2::splat(32.0);
                let r = p.length();
                h[z * n + x] = height * (r * 4.0).sin() * (-(r - 2.0).powi(2)).exp();
                f[z * n + x] = if r < 1.5 { foam } else { 0.0 };
            }
        }
        crate::RippleMap::new(
            glam::Vec2::ZERO,
            cell,
            crate::RIPPLE_FIELD_SIZE,
            &h,
            &f,
            version,
        )
    }

    /// A frame of empty sky through a camera looking along `look`, the
    /// sun shining toward -look (so it sits dead center when `look`
    /// points at it).
    fn sky_frame(renderer: &mut Renderer, look: glam::Vec3, sun_toward: glam::Vec3) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_time(1.0);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 1.0, 0.0),
            glam::Vec3::new(0.0, 1.0, 0.0) + look,
        ));
        draw.set_sun(crate::Sun::new(-sun_toward));
        draw.set_environment(crate::Environment {
            clouds: 0.0,
            ..crate::Environment::daylight()
        });
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    #[test]
    fn offscreen_sky_draws_a_sun_and_a_blue_dome_on_every_preset() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping sky test");
            return;
        };
        let size = renderer.size();
        let toward_sun = glam::Vec3::new(0.0, 0.5, -1.0).normalize();
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::High,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            // Looking at the sun: a white-hot center.
            let at_sun = sky_frame(&mut renderer, toward_sun, toward_sun);
            let [r, g, b, _] = center_pixel(&at_sun, size);
            assert!(
                r > 240 && g > 235 && b > 220,
                "{quality:?}: sun {:?}",
                [r, g, b]
            );
            // Looking up, away from it: blue sky, not the black clear color.
            let up = sky_frame(
                &mut renderer,
                glam::Vec3::new(0.0, 1.0, 0.3).normalize(),
                toward_sun,
            );
            let [r, g, b, _] = center_pixel(&up, size);
            assert!(b > 120 && b > r + 30, "{quality:?}: sky {:?}", [r, g, b]);
        }
    }

    /// The center pixel of a cube filling the view, lit along `sun`, with
    /// the given material parameters (and optional normal map).
    fn material_center(
        renderer: &mut Renderer,
        sun: glam::Vec3,
        params: &crate::MaterialParams,
        normal: Option<Texture>,
    ) -> [u8; 4] {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let material = renderer.create_pbr_material(
            params,
            crate::MaterialTextures {
                normal,
                ..crate::MaterialTextures::default()
            },
        );
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 1.6),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun::new(sun));
        draw.mesh(&cube, glam::Mat4::IDENTITY, &material);
        renderer.render_offscreen(&draw, &target);
        center_pixel(&read_pixels(renderer, &target, size), size)
    }

    #[test]
    fn offscreen_materials_glow_darken_and_bend_on_every_preset() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping material test");
            return;
        };
        let luma = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        // Head-on light (from behind the camera) and light from the side,
        // where the face shows no highlight.
        let front = glam::Vec3::new(0.0, 0.0, -1.0);
        let side = glam::Vec3::new(-0.8, 0.0, -0.6);
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::High,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let dark = crate::MaterialParams::with_color([0.02, 0.02, 0.02, 1.0]);
            let plain = material_center(&mut renderer, front, &dark, None);
            // Emissive red shows on a black surface.
            let glowing = material_center(
                &mut renderer,
                front,
                &crate::MaterialParams {
                    emissive: [1.0, 0.0, 0.0],
                    ..dark
                },
                None,
            );
            assert!(
                glowing[0] > plain[0] + 100 && glowing[0] > glowing[2] + 100,
                "{quality:?}: emissive {glowing:?} vs {plain:?}"
            );
            // Metals have no diffuse: away from the highlight a light metal
            // only shows reflected sky, so it's darker than plastic.
            let light = crate::MaterialParams::with_color([0.7, 0.7, 0.7, 1.0]);
            let plastic = material_center(&mut renderer, side, &light, None);
            let metal = material_center(
                &mut renderer,
                side,
                &crate::MaterialParams {
                    metallic: 1.0,
                    roughness: 0.4,
                    ..light
                },
                None,
            );
            assert!(
                luma(metal) + 60 < luma(plastic),
                "{quality:?}: metal {metal:?} vs plastic {plastic:?}"
            );
            // A normal map tilting every normal 60° away from a head-on light
            // darkens the face.
            let grey = crate::MaterialParams::with_color([0.3, 0.3, 0.3, 1.0]);
            let flat = material_center(&mut renderer, front, &grey, None);
            let (sin, cos) = 60_f32.to_radians().sin_cos();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0..=255
            let encode = |v: f32| (v * 0.5 + 0.5).mul_add(255.0, 0.5) as u8;
            let tilted = [encode(sin), 128, encode(cos), 255];
            let texels: Vec<u8> = std::iter::repeat_n(tilted, 16).flatten().collect();
            let map = renderer.create_data_texture(4, 4, &texels);
            let bent = material_center(&mut renderer, front, &grey, Some(map));
            assert!(
                luma(bent) + 30 < luma(flat),
                "{quality:?}: normal-mapped {bent:?} vs flat {flat:?}"
            );
        }
    }

    /// A sunlit orange wall filling the view with a small white-hot cube in
    /// the middle, under an Environment with `post`.
    fn graded_frame(renderer: &mut Renderer, post: crate::PostEffects) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let wall = renderer.create_material([0.7, 0.32, 0.15, 1.0], None);
        let lamp = renderer.create_pbr_material(
            &crate::MaterialParams {
                emissive: [5.0, 5.0, 5.0],
                ..crate::MaterialParams::with_color([1.0, 1.0, 1.0, 1.0])
            },
            crate::MaterialTextures::default(),
        );
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_time(1.0);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 4.0),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun::new(glam::Vec3::new(0.2, -0.3, -1.0)));
        draw.set_environment(crate::Environment {
            post,
            ..crate::Environment::default()
        });
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(30.0, 30.0, 0.2),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, 0.0, -1.0),
            ),
            &wall,
        );
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::splat(0.25),
                glam::Quat::IDENTITY,
                glam::Vec3::ZERO,
            ),
            &lamp,
        );
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    fn pixel_at(pixels: &[u8], size: (u32, u32), x: u32, y: u32) -> [u32; 3] {
        let i = ((y * size.0 + x) * 4) as usize;
        [
            u32::from(pixels[i]),
            u32::from(pixels[i + 1]),
            u32::from(pixels[i + 2]),
        ]
    }

    #[test]
    fn offscreen_post_effects_grade_the_image_from_medium_up() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping grading test");
            return;
        };
        let size = renderer.size();
        let plain = crate::PostEffects {
            bloom: 0.0,
            ..crate::PostEffects::default()
        };
        let grey = crate::PostEffects {
            saturation: 0.0,
            ..plain
        };
        let dark_corners = crate::PostEffects {
            vignette: 1.0,
            ..plain
        };
        let luma = |p: [u32; 3]| p[0] + p[1] + p[2];
        for quality in GraphicsQuality::ALL {
            renderer.set_quality(quality);
            let neutral = graded_frame(&mut renderer, plain);
            let wall = pixel_at(&neutral, size, 8, 32);
            assert!(
                wall[0] > wall[2] + 30,
                "{quality:?}: an orange wall {wall:?}"
            );
            let desaturated = graded_frame(&mut renderer, grey);
            let vignetted = graded_frame(&mut renderer, dark_corners);
            if quality == GraphicsQuality::Low {
                // Low skips post effects altogether.
                assert_eq!(neutral, desaturated, "Low: no grading");
                assert_eq!(neutral, vignetted, "Low: no vignette");
                continue;
            }
            let [r, g, b] = pixel_at(&desaturated, size, 8, 32);
            assert!(
                r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3,
                "{quality:?}: saturation 0 is grey, got {:?}",
                [r, g, b]
            );
            // The vignette darkens the corners and leaves the middle.
            let corner = (
                luma(pixel_at(&neutral, size, 1, 1)),
                luma(pixel_at(&vignetted, size, 1, 1)),
            );
            assert!(
                corner.1 * 2 < corner.0,
                "{quality:?}: corner {} -> {}",
                corner.0,
                corner.1
            );
            let near = (
                luma(pixel_at(&neutral, size, 26, 32)),
                luma(pixel_at(&vignetted, size, 26, 32)),
            );
            assert!(near.0.abs_diff(near.1) <= 3, "{quality:?}: middle {near:?}");
        }
    }

    #[test]
    fn offscreen_bloom_glows_around_bright_things_on_high_and_up() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping bloom test");
            return;
        };
        let size = renderer.size();
        let off = crate::PostEffects {
            bloom: 0.0,
            ..crate::PostEffects::default()
        };
        let strong = crate::PostEffects { bloom: 3.0, ..off };
        let luma = |p: [u32; 3]| p[0] + p[1] + p[2];
        for quality in [
            GraphicsQuality::Medium,
            GraphicsQuality::High,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let dull = graded_frame(&mut renderer, off);
            let glowing = graded_frame(&mut renderer, strong);
            // Just beside the white cube, on the wall.
            let (a, b) = (
                luma(pixel_at(&dull, size, 32, 24)),
                luma(pixel_at(&glowing, size, 32, 24)),
            );
            if quality == GraphicsQuality::Medium {
                assert_eq!(dull, glowing, "Medium: no bloom");
            } else {
                assert!(b > a + 12, "{quality:?}: bloom should glow, {a} -> {b}");
            }
        }
    }

    /// A green wall 10 m ahead of a camera 1 m up, in red fog: `falloff`
    /// 0 is plain distance fog, above 0 height fog with its top at y = 0.
    fn foggy_wall(renderer: &mut Renderer, falloff: f32) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let wall = renderer.create_material([0.1, 0.8, 0.1, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 1.0, 0.0),
            glam::Vec3::new(0.0, 1.0, -10.0),
        ));
        draw.set_sun(crate::Sun::new(glam::Vec3::new(0.0, -0.5, -1.0)));
        draw.set_environment(crate::Environment {
            fog: true,
            fog_color: [1.0, 0.0, 0.0],
            fog_density: 0.15,
            fog_start: 0.0,
            fog_height: 0.0,
            fog_falloff: falloff,
            ..crate::Environment::default()
        });
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(40.0, 40.0, 0.2),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, 0.0, -10.0),
            ),
            &wall,
        );
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    #[test]
    fn offscreen_height_fog_is_thick_low_down_and_thin_up_high() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping height fog test");
            return;
        };
        let size = renderer.size();
        // How red the fog makes it: red over red + green.
        #[allow(clippy::cast_precision_loss)] // bytes
        let redness = |p: [u32; 3]| p[0] as f32 / (p[0] + p[1]).max(1) as f32;
        for quality in GraphicsQuality::ALL {
            renderer.set_quality(quality);
            let flat = foggy_wall(&mut renderer, 0.0);
            let (top, bottom) = (
                redness(pixel_at(&flat, size, 32, 4)),
                redness(pixel_at(&flat, size, 32, 60)),
            );
            assert!(
                (top - bottom).abs() < 0.1 && top > 0.5,
                "{quality:?}: distance fog is even, {top} vs {bottom}"
            );
            let flat_top = top;
            let layered = foggy_wall(&mut renderer, 1.5);
            let (top, bottom) = (
                redness(pixel_at(&layered, size, 32, 4)),
                redness(pixel_at(&layered, size, 32, 60)),
            );
            assert!(
                bottom > top + 0.2 && top < flat_top - 0.2,
                "{quality:?}: thick below the layer's top, thin above: {top} vs {bottom}"
            );
        }
    }

    /// Looking into the sky at the sun, with a dark wall whose top edge is
    /// just below it (seen from its unlit side), or over it when `covered`.
    fn sun_over_wall(renderer: &mut Renderer, god_rays: f32, covered: bool) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let wall = renderer.create_material([0.2, 0.2, 0.2, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::ZERO,
            glam::Vec3::new(0.0, 2.0, -10.0),
        ));
        draw.set_sun(crate::Sun::new(glam::Vec3::new(0.0, -0.2, 1.0)));
        draw.set_environment(crate::Environment {
            god_rays,
            lens_flare: 0.0,
            ..crate::Environment::daylight()
        });
        // Top edge at y = 0.7, 5 m away: a little under the sun (y = 1).
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(30.0, 20.0, 0.2),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, if covered { 0.0 } else { -9.3 }, -5.0),
            ),
            &wall,
        );
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    #[test]
    fn offscreen_god_rays_stream_past_geometry_toward_the_sun_on_beautiful() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping god rays test");
            return;
        };
        let size = renderer.size();
        let luma = |p: [u32; 3]| p[0] + p[1] + p[2];
        for quality in [GraphicsQuality::High, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let plain = sun_over_wall(&mut renderer, 0.0, false);
            let rays = sun_over_wall(&mut renderer, 2.0, false);
            // On the wall just under its edge, below the sun.
            let near = (
                luma(pixel_at(&plain, size, 32, 40)),
                luma(pixel_at(&rays, size, 32, 40)),
            );
            if quality == GraphicsQuality::High {
                assert!(near.0.abs_diff(near.1) <= 6, "High: no god rays {near:?}");
                continue;
            }
            assert!(
                near.1 > near.0 + 20,
                "light streams over the wall's edge: {near:?}"
            );
            // With the wall hiding the sun there's nothing to stream.
            let hidden = (
                sun_over_wall(&mut renderer, 0.0, true),
                sun_over_wall(&mut renderer, 2.0, true),
            );
            let hidden = (
                luma(pixel_at(&hidden.0, size, 32, 40)),
                luma(pixel_at(&hidden.1, size, 32, 40)),
            );
            assert!(
                hidden.0.abs_diff(hidden.1) <= 3,
                "geometry blocks the rays: {hidden:?}"
            );
        }
    }

    /// A white glowing block 50 m away on black, `dof` strong, focused at
    /// `focus` metres.
    fn far_block(renderer: &mut Renderer, dof: f32, focus: f32) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let white = renderer.create_pbr_material(
            &crate::MaterialParams {
                emissive: [1.0, 1.0, 1.0],
                ..crate::MaterialParams::with_color([1.0, 1.0, 1.0, 1.0])
            },
            crate::MaterialTextures::default(),
        );
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::ZERO,
            glam::Vec3::new(0.0, 0.0, -2.0),
        ));
        draw.set_environment(crate::Environment {
            depth_of_field: dof,
            focus_distance: focus,
            focus_range: 0.5,
            post: crate::PostEffects {
                bloom: 0.0,
                ..crate::PostEffects::default()
            },
            ..crate::Environment::default()
        });
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(20.0, 20.0, 1.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, 0.0, -50.0),
            ),
            &white,
        );
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    #[test]
    fn offscreen_depth_of_field_blurs_what_is_out_of_focus_on_beautiful() {
        let Some(mut renderer) = pollster::block_on(Renderer::new_offscreen((160, 120))).ok()
        else {
            eprintln!("no GPU adapter; skipping depth of field test");
            return;
        };
        let size = renderer.size();
        let luma = |p: [u32; 3]| p[0] + p[1] + p[2];
        for quality in [GraphicsQuality::High, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let sharp = far_block(&mut renderer, 0.0, 0.0);
            // The block's right edge along the middle row.
            let edge = (80..size.0)
                .find(|&x| luma(pixel_at(&sharp, size, x, 60)) < 30)
                .expect("the block ends");
            let outside = |pixels: &[u8]| luma(pixel_at(pixels, size, edge + 1, 60));
            let inside = |pixels: &[u8]| luma(pixel_at(pixels, size, edge - 2, 60));
            // Focused 2 m away (the camera's target): the block blurs.
            let near_focus = far_block(&mut renderer, 4.0, 0.0);
            // Focused on it: sharp.
            let on_it = far_block(&mut renderer, 4.0, 50.0);
            if quality == GraphicsQuality::High {
                assert!(
                    outside(&near_focus).abs_diff(outside(&sharp)) <= 6,
                    "High: no depth of field"
                );
                continue;
            }
            assert!(
                outside(&near_focus) > outside(&sharp) + 60,
                "out of focus, the edge spreads: {} -> {}",
                outside(&sharp),
                outside(&near_focus)
            );
            assert!(inside(&near_focus) < inside(&sharp));
            assert!(
                outside(&on_it) <= outside(&sharp) + 6,
                "in focus it stays sharp: {}",
                outside(&on_it)
            );
        }
    }

    /// A sphere lit from the left on a dark background, under `env`, seen
    /// from 3 m away.
    fn styled_sphere(renderer: &mut Renderer, env: crate::Environment) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let sphere = renderer.create_mesh(&MeshData::sphere(2.0));
        let paint = renderer.create_material([0.8, 0.8, 0.8, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 3.0),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun::new(glam::Vec3::new(1.0, 0.0, -0.4)));
        draw.set_environment(crate::Environment {
            lens_flare: 0.0,
            post: crate::PostEffects {
                bloom: 0.0,
                ..crate::PostEffects::default()
            },
            ..env
        });
        draw.mesh(&sphere, glam::Mat4::IDENTITY, &paint);
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    #[test]
    fn offscreen_toon_bands_flatten_the_shading_into_steps() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping toon test");
            return;
        };
        let size = renderer.size();
        // The distinct brightnesses along the middle row, across the
        // sphere from its lit side to its dark side.
        let tones = |pixels: &[u8]| {
            let mut l: Vec<u32> = (12..52)
                .map(|x| {
                    let p = pixel_at(pixels, size, x, 32);
                    p[0] + p[1] + p[2]
                })
                .collect();
            l.sort_unstable();
            l.dedup_by(|a, b| a.abs_diff(*b) <= 3);
            l.len()
        };
        for quality in [GraphicsQuality::High, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let smooth = styled_sphere(&mut renderer, crate::Environment::default());
            let toon = styled_sphere(
                &mut renderer,
                crate::Environment {
                    toon_bands: 2,
                    toon_softness: 0.05,
                    ..crate::Environment::default()
                },
            );
            let (a, b) = (tones(&smooth), tones(&toon));
            assert!(
                b * 2 <= a,
                "{quality:?}: toon shading has a few flat tones, {a} -> {b}"
            );
            // The shade tint colours the dark side: blue shade.
            let tinted = styled_sphere(
                &mut renderer,
                crate::Environment {
                    toon_bands: 2,
                    shadow_tint: [0.4, 0.6, 1.6],
                    ..crate::Environment::default()
                },
            );
            let dark = pixel_at(&tinted, size, 46, 32);
            assert!(dark[2] > dark[0] + 4, "{quality:?}: blue shade, {dark:?}");
        }
    }

    #[test]
    fn offscreen_outlines_trace_edges_but_not_flat_faces() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping outline test");
            return;
        };
        let size = renderer.size();
        let luma = |p: [u32; 3]| p[0] + p[1] + p[2];
        let outlined = crate::Environment {
            outline: 1.0,
            outline_color: [0.0; 3],
            ..crate::Environment::default()
        };
        for quality in GraphicsQuality::ALL {
            renderer.set_quality(quality);
            let plain = styled_sphere(&mut renderer, crate::Environment::default());
            let lined = styled_sphere(&mut renderer, outlined);
            if quality == GraphicsQuality::Low {
                assert_eq!(plain, lined, "Low: no outlines");
                continue;
            }
            // The sphere's lit left edge along the middle row.
            let edge = (0..32)
                .find(|&x| luma(pixel_at(&plain, size, x, 32)) > 60)
                .expect("the sphere is lit");
            let darkest = (edge.saturating_sub(2)..edge + 3)
                .map(|x| luma(pixel_at(&lined, size, x, 32)))
                .min()
                .unwrap_or(0);
            let before = luma(pixel_at(&plain, size, edge + 1, 32));
            assert!(
                darkest + 60 < before,
                "{quality:?}: a dark line on the silhouette, {before} -> {darkest}"
            );
            // The sphere's middle (smoothly curved) has none.
            let (a, b) = (
                luma(pixel_at(&plain, size, 26, 32)),
                luma(pixel_at(&lined, size, 26, 32)),
            );
            assert!(
                a.abs_diff(b) <= 8,
                "{quality:?}: no line inside, {a} -> {b}"
            );
        }
    }

    #[test]
    fn offscreen_pixelation_draws_blocks_in_a_small_palette() {
        let Some(mut renderer) = pollster::block_on(Renderer::new_offscreen((96, 72))).ok() else {
            eprintln!("no GPU adapter; skipping pixelation test");
            return;
        };
        let size = renderer.size();
        let pixel_art = crate::Environment {
            pixelate: 16.0,
            ..crate::Environment::default()
        };
        // 16 on 720 is 1.6, so 2 px blocks at this size.
        let blocks_are_flat = |pixels: &[u8]| {
            (0..size.1 / 2).all(|by| {
                (0..size.0 / 2).all(|bx| {
                    let c = pixel_at(pixels, size, bx * 2, by * 2);
                    (0..2).all(|dy| {
                        (0..2).all(|dx| pixel_at(pixels, size, bx * 2 + dx, by * 2 + dy) == c)
                    })
                })
            })
        };
        for quality in [GraphicsQuality::High, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let plain = styled_sphere(&mut renderer, crate::Environment::default());
            assert!(!blocks_are_flat(&plain), "{quality:?}: smooth without it");
            let blocky = styled_sphere(&mut renderer, pixel_art);
            assert!(blocks_are_flat(&blocky), "{quality:?}: 2 px blocks");
            // Four levels a channel: at most 64 colours, and few here.
            let limited = styled_sphere(
                &mut renderer,
                crate::Environment {
                    palette: 4,
                    ..crate::Environment::default()
                },
            );
            let mut colours: Vec<[u32; 3]> = (0..size.1)
                .flat_map(|y| (0..size.0).map(move |x| (x, y)))
                .map(|(x, y)| pixel_at(&limited, size, x, y))
                .collect();
            colours.sort_unstable();
            colours.dedup();
            assert!(
                colours.len() <= 64,
                "{quality:?}: a 4-level palette, {} colours",
                colours.len()
            );
        }
    }

    #[test]
    fn offscreen_lit_particles_darken_in_shade() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping particle shade test");
            return;
        };
        let size = renderer.size();
        let luma = |p: [u32; 3]| p[0] + p[1] + p[2];
        for quality in [GraphicsQuality::Medium, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let target = target(renderer.context().device(), size);
            let cube = renderer.create_mesh(&MeshData::cube(1.0));
            let grey = renderer.create_material([0.05, 0.05, 0.05, 1.0], None);
            let mut draw = DrawList::new();
            draw.set_clear_color(Color::BLACK);
            draw.set_camera_3d(Camera3D::new(
                glam::Vec3::new(0.0, 0.0, 5.0),
                glam::Vec3::ZERO,
            ));
            draw.set_sun(crate::Sun {
                shadows: true,
                ..crate::Sun::new(glam::Vec3::new(0.0, -1.0, -0.2))
            });
            // A roof over the left particle, and the wall behind.
            for (scale, at) in [
                (
                    glam::Vec3::new(2.5, 0.2, 3.0),
                    glam::Vec3::new(-1.5, 1.6, 0.0),
                ),
                (
                    glam::Vec3::new(20.0, 20.0, 0.2),
                    glam::Vec3::new(0.0, 0.0, -2.0),
                ),
            ] {
                draw.mesh(
                    &cube,
                    glam::Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, at),
                    &grey,
                );
            }
            for x in [-1.5, 1.5] {
                draw.particles(crate::ParticleBatch {
                    lit: true,
                    ..one_particle([x, 0.0, 0.0], [1.0; 4], crate::ParticleBlend::Alpha, 0.0)
                });
            }
            renderer.render_offscreen(&draw, &target);
            let pixels = read_pixels(&renderer, &target, size);
            let (shaded, sunny) = (
                luma(pixel_at(&pixels, size, 19, 32)),
                luma(pixel_at(&pixels, size, 45, 32)),
            );
            // (sRGB bytes: ambient alone encodes to about half of full sun.)
            assert!(
                shaded + 150 < sunny,
                "{quality:?}: in shade {shaded}, in the sun {sunny}"
            );
        }
    }

    /// The center of a dark wall (lit by neither sun nor ambient) with a
    /// small red-glowing cube floating off to the side in front of it.
    fn wall_beside_glow(renderer: &mut Renderer, glow_light: f32) -> [u8; 4] {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let wall = renderer.create_pbr_material(
            &crate::MaterialParams::with_color([0.5, 0.5, 0.5, 1.0]),
            crate::MaterialTextures::default(),
        );
        let lamp = renderer.create_pbr_material(
            &crate::MaterialParams {
                emissive: [6.0, 0.0, 0.0],
                glow_light,
                ..crate::MaterialParams::with_color([0.0, 0.0, 0.0, 1.0])
            },
            crate::MaterialTextures::default(),
        );
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 1.6),
            glam::Vec3::ZERO,
        ));
        // The sun shines at the camera: the wall's face is in its own
        // shade, and there's no ambient fill.
        draw.set_sun(crate::Sun {
            ambient: 0.0,
            ..crate::Sun::new(glam::Vec3::Z)
        });
        draw.mesh(&cube, glam::Mat4::IDENTITY, &wall);
        let lamp_at = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::splat(0.15),
            glam::Quat::IDENTITY,
            glam::Vec3::new(0.35, 0.0, 0.8),
        );
        draw.mesh(&cube, lamp_at, &lamp);
        renderer.render_offscreen(&draw, &target);
        center_pixel(&read_pixels(renderer, &target, size), size)
    }

    #[test]
    fn offscreen_glowing_meshes_light_their_surroundings_on_every_preset() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping glow light test");
            return;
        };
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::High,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let dark = wall_beside_glow(&mut renderer, 0.0);
            let lit = wall_beside_glow(&mut renderer, 1.0);
            // (Beautiful's bloom spills a little of the glow itself.)
            assert!(
                dark[0] < 60 && dark[1] < 10,
                "{quality:?}: unlit wall {dark:?}"
            );
            assert!(
                lit[0] > dark[0] + 40 && lit[0] > lit[1] + 30,
                "{quality:?}: the red glow should light the wall red: {lit:?} vs {dark:?}"
            );
        }
    }

    /// The center of a dark wall lit by a red-glowing cube off to the side,
    /// with (or without) a plain cube halfway between them.
    fn wall_behind_blocker(renderer: &mut Renderer, blocked: bool) -> [u8; 4] {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let grey = renderer.create_pbr_material(
            &crate::MaterialParams::with_color([0.5, 0.5, 0.5, 1.0]),
            crate::MaterialTextures::default(),
        );
        let lamp = renderer.create_pbr_material(
            &crate::MaterialParams {
                emissive: [6.0, 0.0, 0.0],
                ..crate::MaterialParams::with_color([0.0, 0.0, 0.0, 1.0])
            },
            crate::MaterialTextures::default(),
        );
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 1.6),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun {
            ambient: 0.0,
            ..crate::Sun::new(glam::Vec3::Z)
        });
        draw.mesh(&cube, glam::Mat4::IDENTITY, &grey);
        let small = |at: glam::Vec3| {
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::splat(0.15),
                glam::Quat::IDENTITY,
                at,
            )
        };
        draw.mesh(&cube, small(glam::Vec3::new(0.5, 0.0, 1.1)), &lamp);
        if blocked {
            draw.mesh(&cube, small(glam::Vec3::new(0.25, 0.0, 0.8)), &grey);
        }
        renderer.render_offscreen(&draw, &target);
        center_pixel(&read_pixels(renderer, &target, size), size)
    }

    /// A glowing lamp beside a cube, with another cube at `blocker`.
    fn lamp_scene(renderer: &Renderer, blocker: glam::Vec3) -> DrawList {
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let grey = renderer.create_pbr_material(
            &crate::MaterialParams::with_color([0.5, 0.5, 0.5, 1.0]),
            crate::MaterialTextures::default(),
        );
        let lamp = renderer.create_pbr_material(
            &crate::MaterialParams {
                emissive: [6.0, 0.0, 0.0],
                ..crate::MaterialParams::with_color([0.0, 0.0, 0.0, 1.0])
            },
            crate::MaterialTextures::default(),
        );
        let small = |at: glam::Vec3| {
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::splat(0.15),
                glam::Quat::IDENTITY,
                at,
            )
        };
        let mut draw = DrawList::new();
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 1.6),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun {
            shadows: true,
            ..crate::Sun::new(glam::Vec3::new(0.3, -1.0, -0.2))
        });
        draw.mesh(&cube, glam::Mat4::IDENTITY, &grey);
        draw.mesh(&cube, small(glam::Vec3::new(0.5, 0.0, 1.1)), &lamp);
        draw.mesh(&cube, small(blocker), &grey);
        draw
    }

    #[test]
    fn offscreen_forward_frames_compile_only_the_shadows_they_sample() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping forward variant test");
            return;
        };
        let draw = lamp_scene(&renderer, glam::Vec3::new(0.25, 0.0, 0.8));
        let none = pipeline::ForwardShadows {
            sun: false,
            points: false,
        };
        let sun = pipeline::ForwardShadows {
            sun: true,
            points: false,
        };
        for (quality, expected) in [
            (GraphicsQuality::Low, none),
            (GraphicsQuality::Medium, sun),
            (GraphicsQuality::High, pipeline::ForwardShadows::ALL),
            (GraphicsQuality::Ultra, pipeline::ForwardShadows::ALL),
        ] {
            renderer.set_quality(quality);
            let prepared = renderer.prepare_frame(&draw, (64, 64), false);
            assert_eq!(prepared.forward_shadows(), expected, "{quality:?}");
            // Low and Medium never build (or run) point shadow code.
            let mixes = pipeline::ForwardShadows::for_quality(quality);
            assert_eq!(mixes.iter().any(|m| m.points), quality.point_shadows() > 0);
            assert_eq!(mixes.iter().any(|m| m.sun), quality.shadows_enabled());
        }
        // Without shadowed point lights in view, High skips their code too.
        renderer.set_quality(GraphicsQuality::High);
        let mut unlit = lamp_scene(&renderer, glam::Vec3::new(0.25, 0.0, 0.8));
        unlit.meshes.truncate(1);
        let prepared = renderer.prepare_frame(&unlit, (64, 64), false);
        assert_eq!(prepared.forward_shadows(), sun);
    }

    #[test]
    fn offscreen_point_shadow_faces_redraw_only_when_what_they_see_changes() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping point shadow cache test");
            return;
        };
        renderer.set_quality(GraphicsQuality::High);
        let stale = |renderer: &mut Renderer, draw: &DrawList| {
            let prepared = renderer.prepare_frame(draw, (64, 64), false);
            renderer
                .stale_point_shadow_layers(draw, &prepared)
                .iter()
                .filter(|s| **s)
                .count()
        };
        let draw = lamp_scene(&renderer, glam::Vec3::new(0.25, 0.0, 0.8));
        assert_eq!(stale(&mut renderer, &draw), 6, "first frame draws the cube");
        assert_eq!(stale(&mut renderer, &draw), 0, "a still scene draws none");
        // The blocker moves (still on the lamp's -X side, toward the
        // camera): only the faces that see it before or after redraw.
        let moved = lamp_scene(&renderer, glam::Vec3::new(0.25, 0.05, 0.85));
        let redrawn = stale(&mut renderer, &moved);
        assert!((1..6).contains(&redrawn), "{redrawn} faces redrawn");
        assert_eq!(stale(&mut renderer, &moved), 0);
        // A new preset's maps start empty: everything draws again.
        renderer.set_quality(GraphicsQuality::Ultra);
        assert_eq!(stale(&mut renderer, &moved), 6);
    }

    #[test]
    fn offscreen_objects_block_glow_light_on_presets_with_point_shadows() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping point shadow test");
            return;
        };
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::High,
            GraphicsQuality::Ultra,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let open = wall_behind_blocker(&mut renderer, false);
            let shaded = wall_behind_blocker(&mut renderer, true);
            assert!(
                open[0] > 60,
                "{quality:?}: the glow should reach the wall: {open:?}"
            );
            if quality.point_shadows() == 0 {
                assert!(
                    shaded[0] + 20 > open[0],
                    "{quality:?} has no point shadows: {shaded:?} vs {open:?}"
                );
            } else {
                assert!(
                    shaded[0] + 40 < open[0],
                    "{quality:?}: the blocker should shade the wall: {shaded:?} vs {open:?}"
                );
            }
        }
    }

    /// The center of a dark wall under a red spotlight off to the side,
    /// aimed at it (or straight ahead, missing it), with (or without) a
    /// plain cube halfway between them.
    fn wall_under_spot(renderer: &mut Renderer, aimed: bool, blocked: bool) -> [u8; 4] {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let grey = renderer.create_pbr_material(
            &crate::MaterialParams::with_color([0.5, 0.5, 0.5, 1.0]),
            crate::MaterialTextures::default(),
        );
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 1.6),
            glam::Vec3::ZERO,
        ));
        draw.set_sun(crate::Sun {
            ambient: 0.0,
            ..crate::Sun::new(glam::Vec3::Z)
        });
        draw.mesh(&cube, glam::Mat4::IDENTITY, &grey);
        let at = glam::Vec3::new(0.5, 0.0, 1.1);
        let aim = if aimed {
            glam::Vec3::new(0.0, 0.0, 0.5) - at
        } else {
            glam::Vec3::NEG_Z
        };
        draw.point_light(crate::PointLight::spot(at, aim, [0.5, 0.0, 0.0], 20.0));
        if blocked {
            draw.mesh(
                &cube,
                glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::splat(0.15),
                    glam::Quat::IDENTITY,
                    glam::Vec3::new(0.25, 0.0, 0.8),
                ),
                &grey,
            );
        }
        renderer.render_offscreen(&draw, &target);
        center_pixel(&read_pixels(renderer, &target, size), size)
    }

    #[test]
    fn offscreen_spotlights_light_only_inside_their_cone_and_cast_shadows() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping spotlight test");
            return;
        };
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::High,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let lit = wall_under_spot(&mut renderer, true, false);
            let missed = wall_under_spot(&mut renderer, false, false);
            let shaded = wall_under_spot(&mut renderer, true, true);
            assert!(
                lit[0] > 60,
                "{quality:?}: the spot should light the wall: {lit:?}"
            );
            assert!(
                missed[0] < 10,
                "{quality:?}: outside the cone stays dark: {missed:?}"
            );
            if quality.point_shadows() == 0 {
                assert!(
                    u32::from(shaded[0]) + 20 > u32::from(lit[0]),
                    "{quality:?} has no point shadows: {shaded:?} vs {lit:?}"
                );
            } else {
                assert!(
                    u32::from(shaded[0]) + 40 < u32::from(lit[0]),
                    "{quality:?}: the blocker should shade the wall: {shaded:?} vs {lit:?}"
                );
            }
        }
    }

    #[test]
    fn point_lights_beyond_the_preset_cap_keep_the_ones_near_the_camera() {
        let mut draw = DrawList::new();
        let camera = Camera3D::new(glam::Vec3::ZERO, glam::Vec3::NEG_Z);
        for i in 0..40 {
            #[allow(clippy::cast_precision_loss)] // small counter
            let x = i as f32 * 10.0;
            draw.point_light(crate::PointLight {
                range: 2.0,
                ..crate::PointLight::new(glam::Vec3::new(x, 0.0, 0.0), [1.0; 3])
            });
        }
        // An unlit light is dropped outright.
        draw.point_light(crate::PointLight::new(glam::Vec3::ZERO, [0.0; 3]));
        let kept = frame_point_lights(&draw, &camera, 4, 2);
        assert_eq!(kept.len(), 4);
        let xs: Vec<f32> = kept.iter().map(|l| l.light.position.x).collect();
        assert_eq!(xs, [0.0, 10.0, 20.0, 30.0], "nearest first");
        // The two nearest get the shadow layers, six each.
        let slots: Vec<Option<usize>> = kept.iter().map(|l| l.shadow.map(|(s, _)| s)).collect();
        assert_eq!(slots, [Some(0), Some(6), None, None]);
        // Spotlights take one layer each: one point light's room shadows
        // six of them.
        let mut spots = DrawList::new();
        for i in 0..8 {
            #[allow(clippy::cast_precision_loss)] // small counter
            let x = i as f32;
            spots.point_light(crate::PointLight::spot(
                glam::Vec3::new(x, 2.0, 0.0),
                glam::Vec3::NEG_Y,
                [1.0; 3],
                30.0,
            ));
        }
        let kept = frame_point_lights(&spots, &camera, 8, 1);
        let layers: Vec<Option<usize>> = kept.iter().map(|l| l.shadow.map(|(s, _)| s)).collect();
        assert_eq!(layers.iter().flatten().count(), 6);
        assert!(layers.iter().flatten().all(|&l| l < 6));
        assert_eq!(
            frame_point_lights(&draw, &camera, 100, 0).len(),
            crate::MAX_POINT_LIGHTS
        );
    }

    #[test]
    fn offscreen_simulated_ripples_shape_and_foam_the_water_on_every_preset() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping ripple test");
            return;
        };
        let size = renderer.size();
        for quality in [GraphicsQuality::Low, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let calm = water_frame(&mut renderer, None);
            // A flat field changes nothing.
            let flat = water_frame(&mut renderer, Some(ring_ripples(0.0, 0.0, 1)));
            let diff = |a: &[u8], b: &[u8]| {
                a.iter()
                    .zip(b)
                    .map(|(&x, &y)| u64::from(x.abs_diff(y)))
                    .sum::<u64>()
            };
            assert_eq!(diff(&calm, &flat), 0, "{quality:?}");
            // A ring bends the light; foam at its heart turns it white.
            let ringed = water_frame(&mut renderer, Some(ring_ripples(0.08, 0.0, 2)));
            assert!(
                diff(&calm, &ringed) > 2000,
                "{quality:?}: the ring should show"
            );
            let foamy = water_frame(&mut renderer, Some(ring_ripples(0.0, 1.0, 3)));
            let [r, g, b, _] = center_pixel(&foamy, size);
            let [r0, g0, b0, _] = center_pixel(&calm, size);
            assert!(
                u32::from(r) + u32::from(g) + u32::from(b)
                    > u32::from(r0) + u32::from(g0) + u32::from(b0) + 120,
                "{quality:?}: foam should whiten the water, {:?} vs {:?}",
                [r, g, b],
                [r0, g0, b0]
            );
        }
        // The same snapshot isn't uploaded twice.
        let ripples = ring_ripples(0.05, 0.0, 9);
        renderer.upload_ripples(Some(&ripples));
        let key = renderer.ripple_field_key;
        assert_eq!(key, Some((ripples.texels.as_ptr() as usize, 9)));
        renderer.upload_ripples(Some(&ripples.clone()));
        assert_eq!(renderer.ripple_field_key, key);
    }

    /// A dark wall at z = -1 seen from z = 5, plus `batches`.
    fn particle_frame(renderer: &mut Renderer, batches: &[crate::ParticleBatch]) -> Vec<u8> {
        let size = renderer.size();
        let target = target(renderer.context().device(), size);
        let cube = renderer.create_mesh(&MeshData::cube(1.0));
        let wall = renderer.create_material([0.05, 0.05, 0.05, 1.0], None);
        let mut draw = DrawList::new();
        draw.set_clear_color(Color::BLACK);
        draw.set_camera_3d(Camera3D::new(
            glam::Vec3::new(0.0, 0.0, 5.0),
            glam::Vec3::ZERO,
        ));
        draw.mesh(
            &cube,
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(20.0, 20.0, 0.2),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.0, 0.0, -1.1),
            ),
            &wall,
        );
        for batch in batches {
            draw.particles(batch.clone());
        }
        renderer.render_offscreen(&draw, &target);
        read_pixels(renderer, &target, size)
    }

    fn one_particle(
        position: [f32; 3],
        color: [f32; 4],
        blend: crate::ParticleBlend,
        soft: f32,
    ) -> crate::ParticleBatch {
        crate::ParticleBatch {
            instances: vec![crate::ParticleInstance {
                position,
                size: 2.0,
                color,
                uv: crate::ParticleSprite::Dot.uv(0),
                ..crate::ParticleInstance::default()
            }],
            blend,
            soft,
            ..crate::ParticleBatch::default()
        }
    }

    #[test]
    fn offscreen_particles_glow_cover_hide_behind_walls_fade_softly_and_streak() {
        use crate::ParticleBlend::{Additive, Alpha};
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping particle test");
            return;
        };
        let size = renderer.size();
        let sum = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        for quality in [GraphicsQuality::Low, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let wall = center_pixel(&particle_frame(&mut renderer, &[]), size);
            // Additive: brighter, in its colour.
            let fire = one_particle([0.0; 3], [1.0, 0.5, 0.1, 1.0], Additive, 0.0);
            let glow = center_pixel(&particle_frame(&mut renderer, &[fire]), size);
            assert!(
                glow[0] > wall[0] + 80 && glow[0] > glow[1] && glow[1] > glow[2],
                "{quality:?}: fire should glow orange, {glow:?} over {wall:?}"
            );
            // Alpha: covers the wall in its colour.
            let blue = one_particle([0.0; 3], [0.1, 0.2, 1.0, 1.0], Alpha, 0.0);
            let cover = center_pixel(&particle_frame(&mut renderer, &[blue]), size);
            assert!(
                cover[2] > wall[2] + 80 && cover[2] > cover[0],
                "{quality:?}: smoke should cover, {cover:?}"
            );
            // Behind the wall: hidden.
            let hidden = one_particle([0.0, 0.0, -3.0], [1.0; 4], Additive, 0.0);
            let behind = center_pixel(&particle_frame(&mut renderer, &[hidden]), size);
            assert!(sum(behind) <= sum(wall) + 3, "{quality:?}: {behind:?}");
            // Just in front of the wall: soft ones fade into it.
            let near = |soft| one_particle([0.0, 0.0, -0.9], [1.0; 4], Additive, soft);
            let hard = sum(center_pixel(
                &particle_frame(&mut renderer, &[near(0.0)]),
                size,
            ));
            let soft = sum(center_pixel(
                &particle_frame(&mut renderer, &[near(1.0)]),
                size,
            ));
            assert!(
                soft + 100 < hard && soft > sum(wall),
                "{quality:?}: soft {soft} vs hard {hard} (wall {})",
                sum(wall)
            );
            // A streak stretches a small particle sideways.
            let mut spark = one_particle([0.0; 3], [1.0; 4], Additive, 0.0);
            spark.instances[0].size = 0.2;
            let off_center = |pixels: &[u8]| {
                let (w, h) = (size.0 as usize, size.1 as usize);
                let i = ((h / 2) * w + w / 2 - 12) * 4;
                u32::from(pixels[i]) + u32::from(pixels[i + 1]) + u32::from(pixels[i + 2])
            };
            let dot = off_center(&particle_frame(&mut renderer, &[spark.clone()]));
            spark.instances[0].streak = [2.0, 0.0, 0.0];
            let streak = off_center(&particle_frame(&mut renderer, &[spark]));
            assert!(
                streak > dot + 100,
                "{quality:?}: the streak should reach left, {streak} vs {dot}"
            );
        }
    }

    #[test]
    fn offscreen_diving_tints_the_view_and_caustics_dance_on_the_floor() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping underwater test");
            return;
        };
        let size = renderer.size();
        // The bottom half of the frame: all pool floor.
        let floor = |frame: &[u8]| frame[frame.len() / 2..].to_vec();
        let diff = |a: &[u8], b: &[u8]| {
            a.iter()
                .zip(b)
                .map(|(&x, &y)| u64::from(x.abs_diff(y)))
                .sum::<u64>()
        };
        for quality in [
            GraphicsQuality::Low,
            GraphicsQuality::Medium,
            GraphicsQuality::Beautiful,
        ] {
            renderer.set_quality(quality);
            let still = dive_frame(&mut renderer, 0.0, 1.0);
            // Grey sand, under water, turns blue-green.
            let [r, g, b, _] = center_pixel(&still, size);
            assert!(
                u32::from(g) + u32::from(b) > 2 * u32::from(r) + 20,
                "{quality:?}: the deep should be blue-green, got {:?}",
                [r, g, b]
            );
            // Without caustics the floor holds still; with them it moves
            // (Medium graphics and up: Low skips them).
            let still_later = dive_frame(&mut renderer, 0.0, 1.7);
            let lit = dive_frame(&mut renderer, 1.0, 1.0);
            let lit_later = dive_frame(&mut renderer, 1.0, 1.7);
            let calm = diff(&floor(&still), &floor(&still_later));
            let dancing = diff(&floor(&lit), &floor(&lit_later));
            if quality == GraphicsQuality::Low {
                assert_eq!(dancing, calm, "Low: no caustics");
                continue;
            }
            assert!(
                dancing > calm + 5000,
                "{quality:?}: caustics should move, {dancing} vs {calm}"
            );
            // And they brighten the floor where they gather.
            let bright = |frame: &[u8]| {
                frame
                    .chunks_exact(4)
                    .map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]))
                    .max()
                    .unwrap_or(0)
            };
            assert!(
                bright(&floor(&lit)) > bright(&floor(&still)),
                "{quality:?}: caustics should light the floor"
            );
        }
    }

    #[test]
    fn offscreen_water_tints_the_scene_on_every_preset() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping water test");
            return;
        };
        for quality in [GraphicsQuality::Low, GraphicsQuality::Beautiful] {
            renderer.set_quality(quality);
            let [r, g, b, _] = water_center(&mut renderer);
            // The default water is blue-green over a near-black floor.
            assert!(
                u32::from(g) + u32::from(b) > 2 * u32::from(r) + 10,
                "{quality:?}: water should tint the floor blue-green, got {:?}",
                [r, g, b]
            );
        }
    }
}
