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
/// asks for one.
fn draws_sky(draw: &DrawList) -> bool {
    draw.camera_3d.is_some() && draw.environment.is_some_and(|e| e.sky)
}

/// Per-frame GPU resources derived from a draw list.
struct PreparedFrame {
    camera_2d_bind: wgpu::BindGroup,
    camera_3d_bind: wgpu::BindGroup,
    model_bind_groups: Vec<wgpu::BindGroup>,
    sprite_runs: Vec<SpriteRun>,
    light_bind: wgpu::BindGroup,
    shadows: ShadowCascades,
    /// World bounding sphere per mesh (shadow culling).
    mesh_spheres: Vec<Sphere>,
    render_shadows: bool,
    /// The point lights shaded this frame (with their shadow slots).
    points: Vec<crate::light::FramePoint>,
    /// One bind group per water draw.
    water_binds: Vec<(wgpu::BindGroup, u32)>,
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
    mesh_pipeline: pipeline::PipelinePair,
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
    quality: GraphicsQuality,
    /// The unit grid water surfaces draw with (built on first use).
    water_mesh: Option<Mesh>,
    /// Water pipelines by target format and refraction (Beautiful).
    water_pipelines: Vec<(wgpu::TextureFormat, bool, wgpu::RenderPipeline)>,
    /// Sky dome pipelines per color format (built on first use).
    sky_pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// Mesh pipelines targeting the HDR scene (Beautiful), built lazily.
    hdr_mesh_pipeline: Option<pipeline::PipelinePair>,
    /// Bloom, tone mapping and FXAA (Beautiful).
    post: crate::post::Post,
    /// 1x1 stand-ins for the scene copies forward-path water binds.
    scene_placeholder: wgpu::BindGroup,
    /// Animation time source when a draw list sets none.
    clock: std::time::Instant,
    /// Scratch buffer for CPU->surface frame blits (reused across frames).
    blit_scratch: Vec<u8>,
    /// `(mesh, sprite)` pipelines targeting [`OFFSCREEN_FORMAT`], built lazily
    /// when it differs from the surface format. Lets a windowed renderer also
    /// draw into offscreen targets on the same device (Oberon's viewport).
    offscreen_pipelines: Option<(pipeline::PipelinePair, wgpu::RenderPipeline)>,
    /// Depth buffer for offscreen targets whose size differs from the
    /// surface's, cached by size.
    offscreen_depth: Option<((u32, u32), wgpu::Texture)>,
    /// Overlay-over-scene compositing for CPU-drawn UIs.
    compositor: Compositor,
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
    fn build(ctx: RenderContext, size: (u32, u32), quality: GraphicsQuality) -> Self {
        let device = ctx.device();
        let shared = PipelineShared::new(device);
        let sprite_pipeline = pipeline::create_sprite_pipeline(device, &shared, ctx.format());
        let mesh_pipeline =
            pipeline::create_mesh_pipelines(device, &shared, ctx.format(), DEPTH_FORMAT);
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
        let depth_texture = Some(create_depth_texture(device, size));
        let shadow_pipeline = pipeline::create_shadow_pipelines(device, &shared);
        let compositor = Compositor::new(device);
        let shadow = ShadowResources::new(device, quality);
        let point_shadows = DepthArray::for_point_shadows(device, quality);
        let post = crate::post::Post::new(device);
        let scene_placeholder = placeholder_scene_bind_group(device, &shared);
        Self {
            shared,
            sprite_pipeline,
            mesh_pipeline,
            shadow_pipeline,
            sprite_batch,
            white_texture,
            flat_normal_texture,
            linear_white_texture,
            material_sampler,
            water_ripples,
            depth_texture,
            shadow_texture: shadow.texture,
            shadow_sampler: shadow.sampler,
            shadow_array_view: shadow.array_view,
            shadow_layer_views: shadow.layer_views,
            shadow_map_size: quality.shadow_map_size(),
            shadows_enabled: quality.shadows_enabled(),
            point_shadows,
            quality,
            water_mesh: None,
            water_pipelines: Vec::new(),
            sky_pipelines: Vec::new(),
            hdr_mesh_pipeline: None,
            post,
            scene_placeholder,
            clock: std::time::Instant::now(),
            blit_scratch: Vec::new(),
            offscreen_pipelines: None,
            offscreen_depth: None,
            compositor,
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
        if quality.shadow_map_size() != self.shadow_map_size
            || quality.shadow_cascades() != layers
            || quality.is_beautiful() != self.quality.is_beautiful()
        {
            let shadow = ShadowResources::new(self.ctx.device(), quality);
            // (Also rebuilt when Beautiful toggles: its sampler filters.)
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
        }
        if !quality.is_beautiful() {
            self.post.release();
            self.hdr_mesh_pipeline = None;
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
        Material {
            inner: Arc::new(MaterialInner {
                bind_group,
                params: *params,
                texture: base,
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
        if self.ctx.format() != OFFSCREEN_FORMAT && self.offscreen_pipelines.is_none() {
            let device = self.ctx.device();
            let mesh = pipeline::create_mesh_pipelines(
                device,
                &self.shared,
                OFFSCREEN_FORMAT,
                DEPTH_FORMAT,
            );
            let sprite = pipeline::create_sprite_pipeline(device, &self.shared, OFFSCREEN_FORMAT);
            self.offscreen_pipelines = Some((mesh, sprite));
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
    fn camera_bind_group(&self, view_proj: glam::Mat4, view_pos: glam::Vec3) -> wgpu::BindGroup {
        let mut contents = [0.0_f32; 20];
        contents[..16].copy_from_slice(&view_proj.to_cols_array());
        contents[16..19].copy_from_slice(&view_pos.to_array());
        contents[19] = 1.0;
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
        self.ensure_frame_pipelines(draw, viewport, format, beautiful);

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
        // --- and six per shadowed point light ---
        self.record_point_shadow_passes(&mut encoder, draw, &prepared);

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
            if self.hdr_mesh_pipeline.is_none() {
                self.hdr_mesh_pipeline = Some(pipeline::create_mesh_pipelines(
                    device,
                    &self.shared,
                    pipeline::HDR_FORMAT,
                    pipeline::HDR_DEPTH_FORMAT,
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
        if !draw.waters.is_empty() {
            let (water_format, depth) = if beautiful {
                (pipeline::HDR_FORMAT, pipeline::HDR_DEPTH_FORMAT)
            } else {
                (format, DEPTH_FORMAT)
            };
            let exists = self
                .water_pipelines
                .iter()
                .any(|(f, r, _)| *f == water_format && *r == beautiful);
            if !exists {
                let p = pipeline::create_water_pipeline(
                    device,
                    &self.shared,
                    water_format,
                    depth,
                    beautiful,
                );
                self.water_pipelines.push((water_format, beautiful, p));
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
    ) -> Option<&wgpu::RenderPipeline> {
        self.water_pipelines
            .iter()
            .find(|(f, r, _)| *f == format && *r == refract)
            .map(|(_, _, p)| p)
    }

    /// The forward path: meshes, water (blended) and sprites in one pass,
    /// straight into the target.
    #[allow(clippy::too_many_arguments)]
    fn record_forward(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
        view: &wgpu::TextureView,
        depth_view: Option<&wgpu::TextureView>,
        format: wgpu::TextureFormat,
        offscreen: bool,
    ) {
        // Offscreen targets use pipelines built for their color format.
        let (mesh_pipeline, sprite_pipeline) = match &self.offscreen_pipelines {
            Some((mesh, sprite)) if offscreen => (mesh, sprite),
            _ => (&self.mesh_pipeline, &self.sprite_pipeline),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..wgpu::RenderPassDescriptor::default()
        });
        Self::record_meshes(&mut pass, draw, prepared, mesh_pipeline);
        if depth_view.is_some() {
            self.record_sky(&mut pass, draw, prepared, format);
        }
        if let Some(water) = self.water_pipeline(format, false) {
            self.record_waters(&mut pass, prepared, water, &self.scene_placeholder);
        }
        self.record_sprites(&mut pass, draw, prepared, sprite_pipeline);
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
        let (Some(targets), Some(mesh_pipeline)) = (&self.post.targets, &self.hdr_mesh_pipeline)
        else {
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
            .water_pipeline(pipeline::HDR_FORMAT, true)
            .filter(|_| !draw.waters.is_empty());
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
                        store: if water.is_some() {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..wgpu::RenderPassDescriptor::default()
            });
            Self::record_meshes(&mut pass, draw, prepared, mesh_pipeline);
            self.record_sky(&mut pass, draw, prepared, pipeline::HDR_FORMAT);
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
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..wgpu::RenderPassDescriptor::default()
            });
            self.record_waters(&mut pass, prepared, water, &scene_bind);
        }

        let env = draw.environment.unwrap_or_default();
        let flare = draw.camera_3d.filter(|_| draws_sky(draw)).and_then(|cam| {
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
        self.post.record_bloom_and_resolve(
            device,
            encoder,
            env.exposure.max(0.0),
            beautiful_tonemap(draw.environment),
            flare,
        );

        // Final: FXAA into the target, then sprites on top.
        let sprite_pipeline = match &self.offscreen_pipelines {
            Some((_, sprite)) if format == OFFSCREEN_FORMAT && format != self.ctx.format() => {
                sprite
            }
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
        self.post.draw_fxaa(device, &mut pass, format);
        self.record_sprites(&mut pass, draw, prepared, sprite_pipeline);
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
    ) {
        let mut current: Option<bool> = None;
        for (mesh_draw, model_bind) in draw.meshes.iter().zip(&prepared.model_bind_groups) {
            let skinned = mesh_draw.joints.is_some();
            if current != Some(skinned) {
                pass.set_pipeline(pipelines.get(skinned));
                pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
                pass.set_bind_group(3, &prepared.light_bind, &[]);
                current = Some(skinned);
            }
            Self::record_mesh(pass, mesh_draw, model_bind);
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
    fn water_bind_groups(
        &self,
        draw: &DrawList,
        beautiful: bool,
        time: f32,
    ) -> Vec<(wgpu::BindGroup, u32)> {
        draw.waters
            .iter()
            .map(|w| {
                let uniform = WaterUniform::new(w, beautiful, time);
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
                    ],
                });
                (bind, indices)
            })
            .collect()
    }

    /// Builds the frame's GPU resources from the draw list: camera, model,
    /// light and water bind groups, the sprite batch, and the sun's shadow
    /// cascades.
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
        );
        let camera_3d = draw.camera_3d.unwrap_or_default();
        let camera_3d_bind =
            self.camera_bind_group(camera_3d.view_proj(viewport), camera_3d.position);

        let sprite_runs = batch_sprites(&draw.sprites);
        self.sprite_batch.write(self.ctx.queue(), &draw.sprites);

        // Per-mesh model bind groups (shared by the shadow and main passes).
        let model_bind_groups = draw
            .meshes
            .iter()
            .map(|m| match &m.joints {
                Some(joints) => self.skinned_model_bind_group(m.model, joints),
                None => self.uniform_bind_group(
                    Some("model"),
                    bytemuck::cast_slice(&m.model.to_cols_array()),
                    &self.shared.model_layout,
                ),
            })
            .collect();

        // Sunlight: the draw list's sun, or a default shadowless light.
        let sun = draw.sun.unwrap_or_default();
        let (mesh_spheres, scene) = scene_bounds(draw);
        let direction = sun.normalized_direction();
        let cascades = self.quality.shadow_cascades() as usize;
        let shadows = if cascades > 1 {
            let aspect = viewport.0 as f32 / viewport.1.max(1) as f32;
            ShadowCascades::fit_camera(
                &camera_3d,
                aspect,
                scene,
                direction,
                self.shadow_map_size,
                cascades,
            )
        } else {
            ShadowCascades::fit_scene(scene, direction, self.shadow_map_size)
        };
        let render_shadows = sun.shadows && self.shadows_enabled && !draw.meshes.is_empty();
        let env = draw.environment.unwrap_or_default();
        let c = draw.clear_color;
        let time = draw
            .time
            .unwrap_or_else(|| self.clock.elapsed().as_secs_f32());
        #[allow(clippy::cast_possible_truncation)] // colors fit f32
        let background = [c.r as f32, c.g as f32, c.b as f32];
        let frame = FrameView {
            camera: camera_3d,
            viewport,
            background,
            time,
            beautiful,
        };
        let points = frame_point_lights(
            draw,
            &camera_3d,
            self.quality.max_point_lights(),
            self.quality.point_shadows(),
        );
        let light_bind =
            self.light_bind_group(&sun, &env, &shadows, render_shadows, &frame, &points);
        let water_binds = self.water_bind_groups(draw, beautiful, time);

        PreparedFrame {
            camera_2d_bind,
            camera_3d_bind,
            model_bind_groups,
            sprite_runs,
            light_bind,
            shadows,
            mesh_spheres,
            render_shadows,
            points,
            water_binds,
        }
    }

    /// Builds the frame's light bind group (uniform + shadow map array +
    /// comparison sampler). `render_shadows` gates shadow sampling in the
    /// shader — when false the shadow maps are bound but never read.
    fn light_bind_group(
        &self,
        sun: &Sun,
        env: &crate::Environment,
        shadows: &ShadowCascades,
        render_shadows: bool,
        frame: &FrameView,
        points: &[crate::light::FramePoint],
    ) -> wgpu::BindGroup {
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
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light"),
            layout: &self.shared.light_layout,
            entries: &[
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
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.shadow_layer_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.point_shadows.array_view),
                },
            ],
        })
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
            let mut current: Option<bool> = None;
            let draws = draw
                .meshes
                .iter()
                .zip(&prepared.model_bind_groups)
                .zip(&prepared.mesh_spheres);
            for ((mesh_draw, model_bind), sphere) in draws {
                if shadows.count > 1 && !shadows.touches(cascade, *sphere) {
                    continue;
                }
                let skinned = mesh_draw.joints.is_some();
                if current != Some(skinned) {
                    pass.set_pipeline(self.shadow_pipeline.get(skinned));
                    pass.set_bind_group(0, &light_vp_bind, &[]);
                    current = Some(skinned);
                }
                pass.set_bind_group(1, model_bind, &[]);
                pass.set_vertex_buffer(0, mesh_draw.mesh.inner.vertex_buffer.slice(..));
                if let (true, Some(skin)) = (skinned, &mesh_draw.mesh.inner.skin_buffer) {
                    pass.set_vertex_buffer(1, skin.slice(..));
                }
                pass.set_index_buffer(
                    mesh_draw.mesh.inner.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..mesh_draw.mesh.inner.index_count, 0, 0..1);
            }
        }
    }

    /// Records the point lights' shadow cubes: per shadowed light and face,
    /// every mesh in range inside the face's view, except the one the light
    /// glows from (its own surface would block all its light).
    fn record_point_shadow_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        prepared: &PreparedFrame,
    ) {
        for point in &prepared.points {
            let Some((slot, view)) = &point.shadow else {
                continue;
            };
            let range = point.light.effective_range();
            for (face, vp) in view.faces.iter().enumerate() {
                let Some(layer_view) = self.point_shadows.views.get(slot * 6 + face) else {
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
                let mut current: Option<bool> = None;
                let draws = draw
                    .meshes
                    .iter()
                    .zip(&prepared.model_bind_groups)
                    .zip(&prepared.mesh_spheres)
                    .enumerate();
                for (i, ((mesh_draw, model_bind), sphere)) in draws {
                    if point.source == Some(i)
                        || !crate::light::PointShadow::face_sees(
                            face,
                            point.light.position,
                            sphere.center,
                            sphere.radius,
                            range,
                        )
                    {
                        continue;
                    }
                    let skinned = mesh_draw.joints.is_some();
                    if current != Some(skinned) {
                        pass.set_pipeline(self.shadow_pipeline.get(skinned));
                        pass.set_bind_group(0, &light_vp_bind, &[]);
                        current = Some(skinned);
                    }
                    pass.set_bind_group(1, model_bind, &[]);
                    pass.set_vertex_buffer(0, mesh_draw.mesh.inner.vertex_buffer.slice(..));
                    if let (true, Some(skin)) = (skinned, &mesh_draw.mesh.inner.skin_buffer) {
                        pass.set_vertex_buffer(1, skin.slice(..));
                    }
                    pass.set_index_buffer(
                        mesh_draw.mesh.inner.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..mesh_draw.mesh.inner.index_count, 0, 0..1);
                }
            }
        }
    }

    /// Records one depth-tested mesh draw into the pass.
    fn record_mesh(
        pass: &mut wgpu::RenderPass<'_>,
        mesh_draw: &MeshDraw,
        model_bind: &wgpu::BindGroup,
    ) {
        let MeshDraw {
            mesh,
            model: _,
            material,
            joints,
        } = mesh_draw;
        pass.set_bind_group(1, model_bind, &[]);
        pass.set_bind_group(2, &material.inner.bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.inner.vertex_buffer.slice(..));
        if let (Some(_), Some(skin)) = (joints, &mesh.inner.skin_buffer) {
            pass.set_vertex_buffer(1, skin.slice(..));
        }
        pass.set_index_buffer(mesh.inner.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.inner.index_count, 0, 0..1);
    }
}

/// Creates the depth buffer for the given size (no-op for empty sizes).
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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
        // Beautiful's soft taps each compare a bilinear 2x2 footprint; the
        // other presets keep single compares (cheap on every GPU).
        let filter = if quality.is_beautiful() {
            wgpu::FilterMode::Linear
        } else {
            wgpu::FilterMode::Nearest
        };
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

/// The point lights a frame shades: the draw list's plus one per glowing
/// mesh, at most `max` of them, the first `shadows` of those that cast
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
    let mut slot = 0;
    for p in &mut lights {
        if slot < shadows.min(crate::MAX_POINT_SHADOWS) && p.light.shadows {
            p.shadow = Some((slot, crate::light::PointShadow::for_light(&p.light)));
            slot += 1;
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
            let (scale, _, center) = mesh_draw.model.to_scale_rotation_translation();
            Sphere {
                center,
                radius: posed_radius(mesh_draw) * scale.max_element(),
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
        atlas.prepare(&renderer, [("H", 48.0)]);
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
        renderer.render_offscreen(&draw, &target);
        center_pixel(&read_pixels(renderer, &target, size), size)
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
        // The two nearest get the shadow slots.
        let slots: Vec<Option<usize>> = kept.iter().map(|l| l.shadow.map(|(s, _)| s)).collect();
        assert_eq!(slots, [Some(0), Some(1), None, None]);
        assert_eq!(
            frame_point_lights(&draw, &camera, 100, 0).len(),
            crate::MAX_POINT_LIGHTS
        );
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
