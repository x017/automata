//! The [`Renderer`]: pipelines, GPU resource factories, and frame execution.

use std::collections::HashMap;
use std::sync::Arc;

use wgpu::util::DeviceExt;

use crate::camera;
use crate::draw::{DrawList, MeshDraw, Sprite};
use crate::error::Result;
use crate::light::LightUniform;
use crate::material::{Material, MaterialInner};
use crate::mesh::{Mesh, MeshData, MeshInner};
use crate::pipeline::{self, PipelineShared, SpriteVertex};
use crate::texture::{Texture, TextureInner};
use crate::{GraphicsQuality, RenderContext, Sun};

/// Depth buffer format used by the mesh pass.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// Where a frame is rendered: the window surface or an offscreen texture.
enum FrameTarget {
    /// Windowed: the surface texture is presented after rendering.
    Surface(wgpu::SurfaceTexture),
    /// Offscreen: a regular texture view owned by the caller.
    Offscreen(wgpu::TextureView),
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
    mesh_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    sprite_batch: SpriteBatch,
    white_texture: Texture,
    depth_texture: Option<wgpu::Texture>,
    shadow_texture: wgpu::Texture,
    shadow_sampler: wgpu::Sampler,
    shadow_map_size: u32,
    shadows_enabled: bool,
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
        let mesh_pipeline = pipeline::create_mesh_pipeline(device, &shared, ctx.format());
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
        let shadow_pipeline = pipeline::create_shadow_pipeline(device, &shared);
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
        let mesh_pipeline = pipeline::create_mesh_pipeline(device, &shared, ctx.format());
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
        let shadow_pipeline = pipeline::create_shadow_pipeline(device, &shared);
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
        })
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
        let device = self.ctx.device();
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
                bounding_radius: data
                    .vertices
                    .iter()
                    .map(|v| glam::Vec3::from(v.position).length())
                    .fold(0.0_f32, f32::max),
            }),
        }
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
    pub fn render_offscreen(&mut self, draw: &DrawList, target: &wgpu::Texture) {
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.execute_frame(draw, FrameTarget::Offscreen(view));
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
            label: Some("oberon-viewport"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
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

    /// Reads a render target's pixels back to the CPU as tightly packed
    /// RGBA8 (row-major, top-left origin). Blocks until the GPU finishes.
    ///
    /// Intended for editors and tooling that composite engine output into
    /// their own window; not for per-frame game use.
    #[must_use]
    pub fn read_texture_rgba(&self, target: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
        read_texture_rgba(self, target, size)
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
    /// Renders the frame into `target`, then presents if it's the surface.
    fn execute_frame(&mut self, draw: &DrawList, target: FrameTarget) {
        let viewport = {
            let s = self.ctx.size();
            (s.0.max(1), s.1.max(1))
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
                FrameTarget::Offscreen(view) => view.clone(),
            };
            let depth_view = self
                .depth_texture
                .as_ref()
                .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));

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
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_bind_group(0, &prepared.camera_3d_bind, &[]);
                pass.set_bind_group(3, &prepared.light_bind, &[]);
                for (mesh_draw, model_bind) in draw.meshes.iter().zip(&prepared.model_bind_groups) {
                    Self::record_mesh(&mut pass, mesh_draw, model_bind);
                }
            }

            if !draw.sprites.is_empty() {
                pass.set_pipeline(&self.sprite_pipeline);
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
            FrameTarget::Offscreen(_) => {
                let _ = self.ctx.device().poll(wgpu::PollType::wait_indefinitely());
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
            .map(|m| {
                self.uniform_bind_group(
                    Some("model"),
                    bytemuck::cast_slice(&m.model.to_cols_array()),
                    &self.shared.model_layout,
                )
            })
            .collect();

        // Sunlight: the draw list's sun, or a default shadowless light.
        let sun = draw.sun.unwrap_or_default();
        let light_view_proj = light_view_proj(draw, &sun);
        let render_shadows = sun.shadows && self.shadows_enabled && !draw.meshes.is_empty();
        let light_bind = self.light_bind_group(&sun, light_view_proj, render_shadows, shadow_view);

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
        light_view_proj: glam::Mat4,
        render_shadows: bool,
        shadow_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let device = self.ctx.device();
        let light_uniform = LightUniform::new(
            &Sun {
                shadows: render_shadows,
                ..*sun
            },
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
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(0, &light_vp_bind, &[]);
        for (mesh_draw, model_bind) in draw.meshes.iter().zip(model_bind_groups) {
            pass.set_bind_group(1, model_bind, &[]);
            pass.set_vertex_buffer(0, mesh_draw.mesh.inner.vertex_buffer.slice(..));
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
        } = mesh_draw;
        pass.set_bind_group(1, model_bind, &[]);
        pass.set_bind_group(2, &material.inner.bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.inner.vertex_buffer.slice(..));
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
        let radius = mesh_draw.mesh.inner.bounding_radius * scale.max_element();
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
    /// sprite run.
    fn texture_bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture: &Texture,
    ) -> wgpu::BindGroup {
        let id = texture.batch_id();
        let mut cache = self.bind_group_cache.borrow_mut();
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
/// (row-major, top-left origin). Blocks until the GPU readback completes.
fn read_texture_rgba(renderer: &Renderer, target: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
    let device = renderer.context().device();
    let queue = renderer.context().queue();
    let bytes_per_row = 4 * size.0;
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
    loop {
        let _ = device.poll(wgpu::PollType::Poll);
        if let Ok(received) = rx.try_recv() {
            return received.expect("readback failed");
        }
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_millis(10)),
        });
        if let Ok(received) = rx.try_recv() {
            return received.expect("readback failed");
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
        read_texture_rgba(renderer, target, size)
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

    /// Renders the shadow test scene with the given sun, returning the
    /// average pixel luminance.
    fn shadow_scene_luminance(renderer: &mut Renderer, sun: crate::Sun) -> f32 {
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

        let pixels = read_pixels(renderer, &target, size);
        let sum: u64 = pixels
            .chunks_exact(4)
            .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
            .sum();
        sum as f32 / (pixels.len() / 4) as f32
    }

    #[test]
    fn offscreen_shadow_darkens_occluded_ground() {
        let Some(mut renderer) = pollster::block_on(offscreen()) else {
            eprintln!("no GPU adapter; skipping shadow render test");
            return;
        };

        let sun = crate::Sun::new(glam::Vec3::new(-0.35, -1.0, -0.2));
        let lit = shadow_scene_luminance(&mut renderer, sun);
        let shadowed = shadow_scene_luminance(
            &mut renderer,
            crate::Sun {
                shadows: true,
                ..sun
            },
        );

        assert!(
            shadowed < lit * 0.97,
            "shadowed scene should be darker: lit={lit}, shadowed={shadowed}"
        );
    }
}
