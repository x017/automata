//! The Beautiful preset's post chain: HDR scene targets, bloom, tone
//! mapping and FXAA (shaders in `shaders/post.wgsl`).
//!
//! Frame flow: the scene renders into [`PostTargets::hdr`] (with
//! [`PostTargets::depth`]); water, if any, refracts through copies of both;
//! then [`Post::record_bloom_and_resolve`] builds the bloom mip chain and
//! resolves HDR + bloom into an LDR image, and [`Post::draw_fxaa`] writes
//! the anti-aliased result into the caller's target (inside the caller's
//! pass, so HUD sprites can follow on top, untouched by post effects).

use wgpu::util::DeviceExt;

use crate::pipeline::{HDR_DEPTH_FORMAT, HDR_FORMAT, POST_SHADER};

/// Format of the resolved image FXAA reads: gamma-2 encoded, luma in alpha.
const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The most bloom mips (half resolution down).
const MAX_BLOOM_LEVELS: u32 = 6;

/// How strongly the (thresholded) bloom adds onto the image. The upsample
/// chain sums every mip level, so this is divided by the level count:
/// a large bright area gets the same glow at any resolution, instead of
/// one level's worth per level.
const BLOOM: f32 = 1.2;

/// The post chain uniform (`PostUniform` in post.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    texel: [f32; 4],
    params: [f32; 4],
    output: [f32; 4],
    /// xy: the sun's screen position (uv), z: lens flare strength (0 =
    /// none), w: the sun disk's radius (uv height).
    sun: [f32; 4],
    /// rgb: sunlight color, a: aspect ratio (width / height).
    sun_color: [f32; 4],
}

/// Where the sun is on screen, for the lens flare.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SunFlare {
    /// Screen position, uv (0..1, y down); may lie off screen.
    pub uv: [f32; 2],
    /// The disk's radius as a fraction of the screen height.
    pub radius: f32,
    /// Flare strength.
    pub strength: f32,
    /// Sunlight color.
    pub color: [f32; 3],
}

impl SunFlare {
    /// Projects the sun (light traveling along `direction`) through
    /// `camera`; `None` when it's behind the camera.
    pub fn new(
        camera: &crate::Camera3D,
        viewport: (u32, u32),
        direction: glam::Vec3,
        sun_size_degrees: f32,
        strength: f32,
        color: [f32; 3],
    ) -> Option<Self> {
        let clip = camera.view_proj(viewport) * (-direction.normalize_or_zero()).extend(0.0);
        if clip.w <= 1e-4 || strength <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        let half_fov = (camera.fov_y_degrees * 0.5).to_radians().tan().max(1e-4);
        let radius = (sun_size_degrees * 0.5).to_radians().tan() / half_fov * 0.5;
        Some(Self {
            uv: [ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5],
            radius,
            strength,
            color,
        })
    }

    /// How much the flare shows by screen position: full on screen,
    /// fading out just past the edges.
    fn on_screen(&self) -> f32 {
        let edge = |v: f32| (1.0 - ((v - 0.5).abs() - 0.5).max(0.0) / 0.15).clamp(0.0, 1.0);
        edge(self.uv[0]) * edge(self.uv[1])
    }
}

/// Size-dependent textures, rebuilt when the viewport size changes.
pub(crate) struct PostTargets {
    pub size: (u32, u32),
    /// The scene's HDR color.
    pub hdr: wgpu::Texture,
    /// The scene's depth (copyable, for water).
    pub depth: wgpu::Texture,
    /// Copies water reads while drawing over `hdr` / `depth`.
    pub scene_copy: wgpu::Texture,
    pub depth_copy: wgpu::Texture,
    bloom: wgpu::Texture,
    bloom_levels: u32,
    ldr: wgpu::Texture,
    /// 1x1: how much of the sun disk is unblocked this frame.
    sun_visibility: wgpu::Texture,
}

impl PostTargets {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        use wgpu::TextureUsages as U;
        let (w, h) = (size.0.max(1), size.1.max(1));
        let texture = |label, format, usage, size: (u32, u32), mips| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let bloom_size = ((w / 2).max(1), (h / 2).max(1));
        let bloom_levels = bloom_level_count(bloom_size);
        Self {
            size: (w, h),
            hdr: texture(
                "hdr scene",
                HDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC,
                (w, h),
                1,
            ),
            depth: texture(
                "hdr depth",
                HDR_DEPTH_FORMAT,
                U::RENDER_ATTACHMENT | U::COPY_SRC,
                (w, h),
                1,
            ),
            scene_copy: texture(
                "scene copy",
                HDR_FORMAT,
                U::COPY_DST | U::TEXTURE_BINDING,
                (w, h),
                1,
            ),
            depth_copy: texture(
                "depth copy",
                HDR_DEPTH_FORMAT,
                U::COPY_DST | U::TEXTURE_BINDING,
                (w, h),
                1,
            ),
            bloom: texture(
                "bloom",
                HDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                bloom_size,
                bloom_levels,
            ),
            bloom_levels,
            ldr: texture(
                "ldr",
                LDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                (w, h),
                1,
            ),
            sun_visibility: texture(
                "sun visibility",
                HDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                (1, 1),
                1,
            ),
        }
    }

    fn bloom_mip(&self, level: u32) -> wgpu::TextureView {
        self.bloom.create_view(&wgpu::TextureViewDescriptor {
            label: Some("bloom mip"),
            base_mip_level: level,
            mip_level_count: Some(1),
            ..wgpu::TextureViewDescriptor::default()
        })
    }
}

/// Bloom mips for a half-resolution base: halve until a side would drop
/// below 8 px.
fn bloom_level_count(base: (u32, u32)) -> u32 {
    let mut levels = 1;
    let mut side = base.0.min(base.1);
    while levels < MAX_BLOOM_LEVELS && side / 2 >= 8 {
        side /= 2;
        levels += 1;
    }
    levels
}

/// Post pipelines and targets.
pub(crate) struct Post {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    prefilter: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    sun_visibility: wgpu::RenderPipeline,
    /// FXAA pipelines per target format.
    fxaa: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    sampler: wgpu::Sampler,
    placeholder: wgpu::TextureView,
    pub targets: Option<PostTargets>,
}

impl Post {
    // Layouts and five pipelines, declared once; splitting it hides that.
    #[allow(clippy::too_many_lines)]
    pub fn new(device: &wgpu::Device) -> Self {
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                texture_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(3),
                texture_entry(4),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(POST_SHADER.into()),
        });
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let prefilter = fullscreen_pipeline(
            device,
            &layout,
            &shader,
            "fs_prefilter",
            HDR_FORMAT,
            None,
            None,
        );
        let downsample = fullscreen_pipeline(
            device,
            &layout,
            &shader,
            "fs_downsample",
            HDR_FORMAT,
            None,
            None,
        );
        let upsample = fullscreen_pipeline(
            device,
            &layout,
            &shader,
            "fs_upsample",
            HDR_FORMAT,
            Some(additive),
            None,
        );
        let resolve = fullscreen_pipeline(
            device,
            &layout,
            &shader,
            "fs_resolve",
            LDR_FORMAT,
            None,
            None,
        );
        let sun_visibility = fullscreen_pipeline(
            device,
            &layout,
            &shader,
            "fs_sun_visibility",
            HDR_FORMAT,
            None,
            None,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..wgpu::SamplerDescriptor::default()
        });
        let placeholder = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("post placeholder"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            layout,
            shader,
            prefilter,
            downsample,
            upsample,
            resolve,
            sun_visibility,
            fxaa: Vec::new(),
            sampler,
            placeholder,
            targets: None,
        }
    }

    /// Makes sure the targets match `size` and an FXAA pipeline exists for
    /// `format`.
    pub fn ensure(&mut self, device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) {
        let size = (size.0.max(1), size.1.max(1));
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(PostTargets::new(device, size));
        }
        if !self.fxaa.iter().any(|(f, _)| *f == format) {
            let depth = Some(wgpu::DepthStencilState {
                format: crate::renderer::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            });
            let pipeline = fullscreen_pipeline(
                device,
                &self.layout,
                &self.shader,
                "fs_fxaa",
                format,
                None,
                depth,
            );
            self.fxaa.push((format, pipeline));
        }
    }

    /// Frees the size-dependent targets (when leaving the Beautiful preset).
    pub fn release(&mut self) {
        self.targets = None;
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        src: &wgpu::TextureView,
        uniform: PostUniform,
        extra: Option<&wgpu::TextureView>,
        extra2: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("post"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        extra.unwrap_or(&self.placeholder),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        extra2.unwrap_or(&self.placeholder),
                    ),
                },
            ],
        })
    }

    /// Bloom (down then up the mip chain) and the HDR → LDR resolve with
    /// `exposure` and tone mapping curve `tonemap` (the shader's index),
    /// plus the sun's lens flare if there is one.
    #[allow(clippy::too_many_lines)] // one pass sequence, read top to bottom
    pub fn record_bloom_and_resolve(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        exposure: f32,
        tonemap: f32,
        flare: Option<SunFlare>,
    ) {
        let Some(t) = &self.targets else {
            return;
        };
        let hdr_view = t.hdr.create_view(&wgpu::TextureViewDescriptor::default());
        let mips: Vec<wgpu::TextureView> = (0..t.bloom_levels).map(|i| t.bloom_mip(i)).collect();
        let size_of = |level: u32| {
            let w = (t.size.0 / 2).max(1) >> level;
            let h = (t.size.1 / 2).max(1) >> level;
            (w.max(1) as f32, h.max(1) as f32)
        };
        let strength = BLOOM / t.bloom_levels.max(1) as f32;
        let aspect = t.size.0 as f32 / t.size.1.max(1) as f32;
        let flare = flare
            .map(|f| SunFlare {
                strength: f.strength * f.on_screen(),
                ..f
            })
            .filter(|f| f.strength > 0.0);
        let (sun, sun_color) = flare.map_or(([0.0; 4], [0.0, 0.0, 0.0, aspect]), |f| {
            (
                [f.uv[0], f.uv[1], f.strength, f.radius],
                [f.color[0], f.color[1], f.color[2], aspect],
            )
        });
        let uniform = |texel: (f32, f32)| PostUniform {
            texel: [1.0 / texel.0, 1.0 / texel.1, 0.0, 0.0],
            params: [0.0, exposure, tonemap, strength],
            output: [0.0; 4],
            sun,
            sun_color,
        };
        // The sun's visibility: how much of its disk still blazes in the
        // HDR image (blocked by geometry or dimmed by clouds).
        let visibility = t
            .sun_visibility
            .create_view(&wgpu::TextureViewDescriptor::default());
        if flare.is_some() {
            let bind = self.bind(
                device,
                &hdr_view,
                uniform((t.size.0 as f32, t.size.1 as f32)),
                None,
                None,
            );
            fullscreen_pass(
                encoder,
                "sun visibility",
                &visibility,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.sun_visibility,
                &bind,
            );
        }
        // Down: the scene into mip 0, each mip into the next.
        for level in 0..t.bloom_levels {
            let (src, src_size) = if level == 0 {
                (&hdr_view, (t.size.0 as f32, t.size.1 as f32))
            } else {
                (&mips[level as usize - 1], size_of(level - 1))
            };
            let bind = self.bind(device, src, uniform(src_size), None, None);
            fullscreen_pass(
                encoder,
                "bloom down",
                &mips[level as usize],
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                if level == 0 {
                    &self.prefilter
                } else {
                    &self.downsample
                },
                &bind,
            );
        }
        // Up: each mip added onto the next larger one.
        for level in (1..t.bloom_levels).rev() {
            let bind = self.bind(
                device,
                &mips[level as usize],
                uniform(size_of(level)),
                None,
                None,
            );
            fullscreen_pass(
                encoder,
                "bloom up",
                &mips[level as usize - 1],
                wgpu::LoadOp::Load,
                &self.upsample,
                &bind,
            );
        }
        // Resolve: HDR + bloom, exposure, tone mapping.
        let ldr_view = t.ldr.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.bind(
            device,
            &hdr_view,
            uniform((t.size.0 as f32, t.size.1 as f32)),
            Some(&mips[0]),
            flare.map(|_| &visibility),
        );
        fullscreen_pass(
            encoder,
            "resolve",
            &ldr_view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            &self.resolve,
            &bind,
        );
    }

    /// Draws the anti-aliased frame into `pass` (whose color target has
    /// `format`).
    pub fn draw_fxaa(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        format: wgpu::TextureFormat,
    ) {
        let (Some(t), Some((_, pipeline))) =
            (&self.targets, self.fxaa.iter().find(|(f, _)| *f == format))
        else {
            return;
        };
        let ldr_view = t.ldr.create_view(&wgpu::TextureViewDescriptor::default());
        let uniform = PostUniform {
            texel: [1.0 / t.size.0 as f32, 1.0 / t.size.1 as f32, 0.0, 0.0],
            params: [0.0; 4],
            output: [f32::from(u8::from(!format.is_srgb())), 0.0, 0.0, 0.0],
            sun: [0.0; 4],
            sun_color: [0.0; 4],
        };
        let bind = self.bind(device, &ldr_view, uniform, None, None);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A fullscreen-triangle pipeline for one post pass.
pub(crate) fn fullscreen_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    entry: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    depth_stencil: Option<wgpu::DepthStencilState>,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Records one fullscreen pass.
pub(crate) fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..wgpu::RenderPassDescriptor::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_chain_stops_before_tiny_mips() {
        assert_eq!(bloom_level_count((960, 540)), 6);
        assert_eq!(bloom_level_count((32, 32)), 3); // 32, 16, 8
        assert_eq!(bloom_level_count((4, 4)), 1);
        assert_eq!(std::mem::size_of::<PostUniform>(), 80);
    }

    #[test]
    fn the_sun_projects_where_the_camera_looks() {
        let cam = crate::Camera3D::new(glam::Vec3::ZERO, glam::Vec3::new(0.0, 0.0, -10.0));
        // Light traveling toward +Z: the sun is straight ahead (-Z).
        let f = SunFlare::new(&cam, (800, 600), glam::Vec3::Z, 2.0, 1.0, [1.0; 3]).unwrap();
        assert!((f.uv[0] - 0.5).abs() < 1e-4 && (f.uv[1] - 0.5).abs() < 1e-4);
        assert!(f.radius > 0.0 && f.radius < 0.05);
        assert!((f.on_screen() - 1.0).abs() < 1e-6);
        // Behind the camera: no flare.
        assert!(SunFlare::new(&cam, (800, 600), glam::Vec3::NEG_Z, 2.0, 1.0, [1.0; 3]).is_none());
        // Far off to the side: faded out.
        let side = SunFlare::new(
            &cam,
            (800, 600),
            glam::Vec3::new(-3.0, 0.0, 1.0),
            2.0,
            1.0,
            [1.0; 3],
        )
        .unwrap();
        assert!(side.uv[0] > 1.15 && side.on_screen() == 0.0);
    }
}
