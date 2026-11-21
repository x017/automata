//! The [`Renderer`]: pipelines, GPU resource factories, and frame execution.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;
use wgpu::util::DeviceExt;

use crate::camera;
use crate::composite::Compositor;
use crate::draw::{DrawList, MeshDraw, Sprite};
use crate::error::Result;
use crate::light::LightUniform;
use crate::material::{Material, MaterialInner};
use crate::mesh::{Mesh, MeshData, MeshInner, SkinVertex};
use crate::pipeline::{self, PipelineShared, SpriteVertex};
use crate::texture::{Texture, TextureInner};
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

/// Per-frame GPU resources derived from a draw list.
struct PreparedFrame {
    camera_2d_bind: wgpu::BindGroup,
    camera_3d_bind: wgpu::BindGroup,
    model_bind_groups: Vec<wgpu::BindGroup>,
    sprite_runs: Vec<SpriteRun>,
    light_bind: wgpu::BindGroup,
    light_view_proj: glam::Mat4,
    render_shadows: bool,
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
    depth_texture: Option<wgpu::Texture>,
    shadow_texture: wgpu::Texture,
    shadow_sampler: wgpu::Sampler,
    shadow_map_size: u32,
    shadows_enabled: bool,
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
        let device = ctx.device();
        let shared = PipelineShared::new(device);
        let sprite_pipeline = pipeline::create_sprite_pipeline(device, &shared, ctx.format());
        let mesh_pipeline = pipeline::create_mesh_pipelines(device, &shared, ctx.format());
        let sprite_batch = SpriteBatch::new(device);
        let white_texture = Texture::from_rgba(
            device,
            ctx.queue(),
            1,
            1,
            &[255, 255, 255, 255],
            wgpu::FilterMode::Linear,
        );
        let depth_texture = Some(create_depth_texture(device, size));
        let shadow_pipeline = pipeline::create_shadow_pipelines(device, &shared);
        let compositor = Compositor::new(device);
        let (shadow_texture, shadow_sampler) =
            create_shadow_resources(device, quality.shadow_map_size());

        Ok(Self {
            ctx,
            shared,
            sprite_pipeline,
            mesh_pipeline,
            shadow_pipeline,
            sprite_batch,
            white_texture,
            depth_texture,
            shadow_texture,
            shadow_sampler,
            shadow_map_size: quality.shadow_map_size(),
            shadows_enabled: quality.shadows_enabled(),
            blit_scratch: Vec::new(),
            offscreen_pipelines: None,
            offscreen_depth: None,
            compositor,
        })
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
        let device = ctx.device();
        let shared = PipelineShared::new(device);
        let sprite_pipeline = pipeline::create_sprite_pipeline(device, &shared, ctx.format());
        let mesh_pipeline = pipeline::create_mesh_pipelines(device, &shared, ctx.format());
        let sprite_batch = SpriteBatch::new(device);
        let white_texture = Texture::from_rgba(
            device,
            ctx.queue(),
            1,
            1,
            &[255, 255, 255, 255],
            wgpu::FilterMode::Linear,
        );
        let depth_texture = Some(create_depth_texture(device, size));
        let shadow_pipeline = pipeline::create_shadow_pipelines(device, &shared);
        let compositor = Compositor::new(device);
        let (shadow_texture, shadow_sampler) =
            create_shadow_resources(device, quality.shadow_map_size());

        Ok(Self {
            ctx,
            shared,
            sprite_pipeline,
            mesh_pipeline,
            shadow_pipeline,
            sprite_batch,
            white_texture,
            depth_texture,
            shadow_texture,
            shadow_sampler,
            shadow_map_size: quality.shadow_map_size(),
            shadows_enabled: quality.shadows_enabled(),
            blit_scratch: Vec::new(),
            offscreen_pipelines: None,
            offscreen_depth: None,
            compositor,
        })
    }

    /// Switches the quality preset at runtime (recreates the shadow map).
    pub fn set_quality(&mut self, quality: GraphicsQuality) {
        let size = quality.shadow_map_size();
        if size != self.shadow_map_size {
            let (texture, sampler) = create_shadow_resources(self.ctx.device(), size);
            self.shadow_texture = texture;
            self.shadow_sampler = sampler;
            self.shadow_map_size = size;
        }
        self.shadows_enabled = quality.shadows_enabled();
    }

    /// The active quality preset.
    #[must_use]
    pub fn quality(&self) -> GraphicsQuality {
        match (self.shadows_enabled, self.shadow_map_size) {
            (false, _) => GraphicsQuality::Low,
            (true, s) if s <= 1024 => GraphicsQuality::Medium,
            (true, s) if s <= 2048 => GraphicsQuality::High,
            _ => GraphicsQuality::Ultra,
        }
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
    /// `None`.
    #[must_use]
    pub fn create_material(&self, color: [f32; 4], texture: Option<Texture>) -> Material {
        let device = self.ctx.device();
        let texture = texture.unwrap_or_else(|| self.white_texture.clone());
        let color_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material color"),
            contents: bytemuck::cast_slice(&color),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout: &self.shared.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.inner.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&texture.inner.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: color_buffer.as_entire_binding(),
                },
            ],
        });
        Material {
            inner: Arc::new(MaterialInner {
                bind_group,
                color,
                texture,
            }),
        }
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
            let mesh = pipeline::create_mesh_pipelines(device, &self.shared, OFFSCREEN_FORMAT);
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
    #[allow(clippy::too_many_lines)] // one linear sequence of GPU passes
    fn execute_frame(&mut self, draw: &DrawList, target: FrameTarget) {
        let (viewport, offscreen) = match &target {
            FrameTarget::Offscreen { size, .. } => (*size, true),
            FrameTarget::Surface(_) => {
                let s = self.ctx.size();
                ((s.0.max(1), s.1.max(1)), false)
            }
        };
        let mut encoder =
            self.ctx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("frame"),
                });

        let shadow_view = self
            .shadow_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let prepared = self.prepare_frame(draw, viewport, &shadow_view);

        // --- Shadow pass: depth from the sun's view, for shadow mapping ---
        if prepared.render_shadows {
            self.record_shadow_pass(
                &mut encoder,
                draw,
                &prepared.model_bind_groups,
                &shadow_view,
                prepared.light_view_proj,
            );
        }

        // --- Main pass: meshes (depth-tested, lit), then sprites ---
        // The view borrows the surface texture only for the scope of the
        // pass; present() re-acquires it by value afterwards.
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
            // Offscreen targets use pipelines built for their color format.
            let (mesh_pipeline, sprite_pipeline) = match &self.offscreen_pipelines {
                Some((mesh, sprite)) if offscreen => (mesh, sprite),
                _ => (&self.mesh_pipeline, &self.sprite_pipeline),
            };

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(draw.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: depth_view.as_ref().map(|dv| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view: dv,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }
                }),
                ..wgpu::RenderPassDescriptor::default()
            });

            if !draw.meshes.is_empty() {
                let mut current: Option<bool> = None;
                for (mesh_draw, model_bind) in draw.meshes.iter().zip(&prepared.model_bind_groups) {
                    let skinned = mesh_draw.joints.is_some();
                    if current != Some(skinned) {
                        pass.set_pipeline(mesh_pipeline.get(skinned));
                        pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
                        pass.set_bind_group(3, &prepared.light_bind, &[]);
                        current = Some(skinned);
                    }
                    Self::record_mesh(&mut pass, mesh_draw, model_bind);
                }
            }

            if !draw.sprites.is_empty() {
                pass.set_pipeline(sprite_pipeline);
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

    /// Builds the frame's GPU resources from the draw list: camera, model,
    /// and light bind groups, the sprite batch, and the sun's shadow setup.
    fn prepare_frame(
        &mut self,
        draw: &DrawList,
        viewport: (u32, u32),
        shadow_view: &wgpu::TextureView,
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
        let light_view_proj = light_view_proj(draw, &sun);
        let render_shadows = sun.shadows && self.shadows_enabled && !draw.meshes.is_empty();
        let env = draw.environment.unwrap_or_default();
        let light_bind =
            self.light_bind_group(&sun, &env, light_view_proj, render_shadows, shadow_view);

        PreparedFrame {
            camera_2d_bind,
            camera_3d_bind,
            model_bind_groups,
            sprite_runs,
            light_bind,
            light_view_proj,
            render_shadows,
        }
    }

    /// Builds the frame's light bind group (sun uniform + shadow map +
    /// comparison sampler). `render_shadows` gates shadow sampling in the
    /// shader — when false the shadow map is bound but never read.
    fn light_bind_group(
        &self,
        sun: &Sun,
        env: &crate::Environment,
        light_view_proj: glam::Mat4,
        render_shadows: bool,
        shadow_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let device = self.ctx.device();
        let light_uniform = LightUniform::with_environment(
            &Sun {
                shadows: render_shadows,
                ..*sun
            },
            env,
            light_view_proj,
            1.0 / self.shadow_map_size as f32,
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
                    resource: wgpu::BindingResource::TextureView(shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                },
            ],
        })
    }

    /// Records the depth-only shadow pass: every mesh drawn from the sun's
    /// viewpoint into the shadow map.
    fn record_shadow_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &DrawList,
        model_bind_groups: &[wgpu::BindGroup],
        shadow_view: &wgpu::TextureView,
        light_view_proj: glam::Mat4,
    ) {
        let light_vp_bind = self.uniform_bind_group(
            Some("shadow light"),
            bytemuck::cast_slice(&light_view_proj.to_cols_array()),
            &self.shared.shadow_light_layout,
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: shadow_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..wgpu::RenderPassDescriptor::default()
        });
        let mut current: Option<bool> = None;
        for (mesh_draw, model_bind) in draw.meshes.iter().zip(model_bind_groups) {
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

/// The shadow map depth texture and its comparison sampler.
fn create_shadow_resources(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow map"),
        size: wgpu::Extent3d {
            width: size.max(1),
            height: size.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: pipeline::SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow comparison"),
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..wgpu::SamplerDescriptor::default()
    });
    (texture, sampler)
}

/// The sun's view-projection matrix, fit around the frame's meshes so the
/// shadow map covers the visible scene.
fn light_view_proj(draw: &DrawList, sun: &Sun) -> glam::Mat4 {
    use glam::camera::rh::{proj::directx, view};

    if draw.meshes.is_empty() {
        return glam::Mat4::IDENTITY;
    }
    // World-space AABB over the meshes' bounding spheres.
    let mut min = glam::Vec3::splat(f32::MAX);
    let mut max = glam::Vec3::splat(f32::MIN);
    for mesh_draw in &draw.meshes {
        let (scale, _, center) = mesh_draw.model.to_scale_rotation_translation();
        let radius = posed_radius(mesh_draw) * scale.max_element();
        min = min.min(center - glam::Vec3::splat(radius));
        max = max.max(center + glam::Vec3::splat(radius));
    }
    let center = (min + max) * 0.5;
    let radius = ((max - min) * 0.5).length().max(1.0) * 1.1; // 10% margin

    let direction = sun.normalized_direction();
    let up = if direction.y.abs() > 0.95 {
        glam::Vec3::Z
    } else {
        glam::Vec3::Y
    };
    let eye = center - direction * (radius + 5.0);
    let view = view::look_at_mat4(eye, center, up);
    let proj = directx::orthographic(-radius, radius, -radius, radius, 0.1, radius * 2.0 + 10.0);
    proj * view
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
}
