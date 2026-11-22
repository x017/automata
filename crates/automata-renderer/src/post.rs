//! The post chains (shaders in `shaders/post.wgsl`).
//!
//! **Beautiful**: HDR scene targets, bloom, tone mapping, colour grading
//! and FXAA.
//!
//! Frame flow: the scene renders into [`PostTargets::hdr`] (with
//! [`PostTargets::depth`]); water, if any, refracts through copies of both;
//! then [`Post::record_bloom_and_resolve`] builds the bloom mip chain, god
//! rays and the depth-of-field blur when the [`Lens`] asks for them, and
//! resolves them all into an LDR image, and [`Post::draw_fxaa`] writes
//! the anti-aliased result into the caller's target (inside the caller's
//! pass, so HUD sprites can follow on top, untouched by post effects).
//!
//! **Medium to Ultra** ([`LdrTargets`]), only when the Environment's post
//! effects change something: the forward path renders into
//! [`LdrTargets::scene`] instead of the target, [`Post::record_ldr_bloom`]
//! builds a cheap quarter-size bloom (High and Ultra), and
//! [`Post::draw_grade`] grades the image into the caller's pass.

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

/// Cheap bloom mips (a quarter resolution down).
const LDR_BLOOM_LEVELS: u32 = 3;

/// How strongly the cheap bloom adds on at `PostEffects::bloom` 1.
const LDR_BLOOM: f32 = 2.4;

/// The post chain uniform (`PostUniform` in post.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    texel: [f32; 4],
    params: [f32; 4],
    output: [f32; 4],
    /// xy: the sun's screen position (uv), z: lens flare strength (0 =
    /// none), w: the sun disk's radius (uv height).
    sun: [f32; 4],
    /// rgb: sunlight color, a: aspect ratio (width / height).
    sun_color: [f32; 4],
    /// The grade: see [`Grade`].
    grade: [f32; 4],
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    /// x: time (s), y: cheap bloom strength, z: 1 = grade.
    misc: [f32; 4],
    /// x: god rays, y: depth of field, z: focus distance, w: focus range.
    lens: [f32; 4],
    /// x: near, y: far (m), zw: the full-size viewport (px).
    depth: [f32; 4],
    /// x: outline width (px), y: pixel block (px, 1 = off), z: palette
    /// levels (0 = off).
    style: [f32; 4],
    /// rgb: the outline colour.
    outline_color: [f32; 4],
}

/// The Environment's screen-space style this frame: outlines, pixelation
/// and the palette (Medium graphics and up), in pixels of the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stylize {
    /// Outline width (px, 0 = none).
    pub outline: f32,
    pub outline_color: [f32; 3],
    /// Pixel block size (px, 1 = no pixelation).
    pub pixel: f32,
    /// Levels per channel (0 = full colour).
    pub palette: f32,
    /// The camera's clip planes, to turn depth into distance.
    pub near: f32,
    pub far: f32,
    /// The viewport (px).
    pub size: (u32, u32),
}

impl Default for Stylize {
    fn default() -> Self {
        Self {
            outline: 0.0,
            outline_color: [0.0; 3],
            pixel: 1.0,
            palette: 0.0,
            near: 0.1,
            far: 1000.0,
            size: (1, 1),
        }
    }
}

impl Stylize {
    /// `env`'s style for a `size` viewport seen through `camera`. Sizes are
    /// given for a 720-pixel-tall screen and scale with it; outlines get as
    /// wide as the pixel blocks, so they survive pixelation. No outlines
    /// without a 3D camera.
    #[must_use]
    pub fn new(
        env: &crate::Environment,
        camera: Option<&crate::Camera3D>,
        size: (u32, u32),
    ) -> Self {
        let scale = size.1.max(1) as f32 / 720.0;
        let pixel = if env.pixelate > 1.0 {
            (env.pixelate.min(16.0) * scale).round().max(1.0)
        } else {
            1.0
        };
        let outline = match camera {
            Some(_) if env.outline > 0.0 => env.outline.min(4.0) * scale.max(0.5) * pixel,
            _ => 0.0,
        };
        let palette = if env.palette >= 2 {
            f32::from(u8::try_from(env.palette.min(32)).unwrap_or(32))
        } else {
            0.0
        };
        Self {
            outline,
            outline_color: env.outline_color,
            pixel,
            palette,
            near: camera.map_or(0.1, |c| c.near),
            far: camera.map_or(1000.0, |c| c.far),
            size,
        }
    }

    /// Whether outlines are drawn.
    #[must_use]
    pub fn outlines(&self) -> bool {
        self.outline > 0.0
    }

    /// Whether the final pass pixelates or limits the palette.
    #[must_use]
    pub fn pixel_art(&self) -> bool {
        self.pixel > 1.0 || self.palette >= 2.0
    }

    fn fill(&self, mut u: PostUniform) -> PostUniform {
        u.style = [self.outline, self.pixel, self.palette, 0.0];
        u.outline_color = [
            self.outline_color[0],
            self.outline_color[1],
            self.outline_color[2],
            1.0,
        ];
        u.depth = [self.near, self.far, self.size.0 as f32, self.size.1 as f32];
        u
    }
}

/// Beautiful's camera effects this frame: god rays and depth of field.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Lens {
    /// God ray strength (0 = none), already faded by where the sun is.
    pub god_rays: f32,
    /// Depth-of-field strength (0 = none).
    pub depth_of_field: f32,
    /// The distance in focus and the sharp range around it (m).
    pub focus: f32,
    pub range: f32,
    /// The camera's clip planes, to turn depth into distance.
    pub near: f32,
    pub far: f32,
}

impl Lens {
    /// The Environment's settings through `camera` (focus on its target
    /// unless the Environment sets a distance), with the god rays faded by
    /// `sun`: none without one, and fading as it leaves the screen or
    /// sinks below the horizon.
    pub fn new(
        env: &crate::Environment,
        camera: &crate::Camera3D,
        sun: Option<&SunFlare>,
        sun_height: f32,
    ) -> Self {
        let god_rays = sun.map_or(0.0, |s| {
            let edge = |v: f32| (1.0 - ((v - 0.5).abs() - 0.5).max(0.0) / 0.6).clamp(0.0, 1.0);
            let up = (sun_height * 8.0).clamp(0.0, 1.0);
            env.god_rays.clamp(0.0, 2.0) * edge(s.uv[0]) * edge(s.uv[1]) * up
        });
        let focus = if env.focus_distance > 0.0 {
            env.focus_distance
        } else {
            camera.position.distance(camera.target)
        };
        Self {
            god_rays,
            depth_of_field: env.depth_of_field.clamp(0.0, 4.0),
            focus: focus.max(0.01),
            range: env.focus_range.max(0.0),
            near: camera.near,
            far: camera.far,
        }
    }
}

/// The grading and lens effects a post pass applies, packed for the
/// shader from the Environment's [`crate::PostEffects`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Grade {
    tone: [f32; 4],
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    /// x: time, z: 1 = grade (y is set per pass).
    misc: [f32; 4],
}

impl Grade {
    /// `post` at `time` seconds (for the grain), with the values clamped
    /// to their ranges.
    pub fn new(post: &crate::PostEffects, time: f32) -> Self {
        let rgb = |c: [f32; 3], lo: f32, hi: f32, a: f32| {
            [
                c[0].clamp(lo, hi),
                c[1].clamp(lo, hi),
                c[2].clamp(lo, hi),
                a,
            ]
        };
        Self {
            tone: [
                post.contrast.clamp(0.0, 4.0),
                post.saturation.clamp(0.0, 4.0),
                post.temperature.clamp(-1.0, 1.0),
                post.tint.clamp(-1.0, 1.0),
            ],
            lift: rgb(post.lift, -1.0, 1.0, post.vignette.clamp(0.0, 1.0)),
            gamma: rgb(post.gamma, 0.05, 8.0, post.grain.clamp(0.0, 1.0)),
            gain: rgb(
                post.gain,
                0.0,
                8.0,
                post.chromatic_aberration.clamp(0.0, 1.0),
            ),
            misc: [
                time.rem_euclid(3600.0),
                0.0,
                f32::from(u8::from(!post.is_neutral_grade())),
                0.0,
            ],
        }
    }

    /// Whether the pass has nothing to grade or fringe.
    fn is_noop(&self) -> bool {
        self.misc[2] < 0.5 && self.gain[3] <= 0.0
    }

    fn into_uniform(self, mut u: PostUniform, bloom: f32) -> PostUniform {
        u.grade = self.tone;
        u.lift = self.lift;
        u.gamma = self.gamma;
        u.gain = self.gain;
        u.misc = [self.misc[0], bloom, self.misc[2], 0.0];
        u
    }
}

/// What the Medium-to-Ultra post chain does this frame, if anything.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LdrChain {
    pub grade: Grade,
    /// Cheap bloom strength (0 = none).
    pub bloom: f32,
    /// Outlines, pixelation and the palette.
    pub style: Stylize,
}

impl LdrChain {
    /// The chain `post` asks for on `quality` (None: render straight to
    /// the target, as without post effects).
    pub fn new(
        quality: crate::GraphicsQuality,
        post: &crate::PostEffects,
        time: f32,
        style: Stylize,
    ) -> Option<Self> {
        if !quality.post_effects() || quality.is_beautiful() {
            return None;
        }
        let grade = Grade::new(post, time);
        let bloom = if quality.bloom() {
            post.bloom.clamp(0.0, 4.0) * LDR_BLOOM / LDR_BLOOM_LEVELS as f32
        } else {
            0.0
        };
        (bloom > 0.0 || !grade.is_noop() || style.outlines() || style.pixel_art()).then_some(Self {
            grade,
            bloom,
            style,
        })
    }
}

/// Medium to Ultra's targets: the scene image and the cheap bloom chain.
pub(crate) struct LdrTargets {
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    /// What the forward path draws into (the target's format).
    pub scene: wgpu::Texture,
    bloom: wgpu::Texture,
    bloom_levels: u32,
}

impl LdrTargets {
    fn new(device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> Self {
        use wgpu::TextureUsages as U;
        let (w, h) = (size.0.max(1), size.1.max(1));
        let bloom_size = ((w / 4).max(1), (h / 4).max(1));
        let bloom_levels = bloom_level_count(bloom_size).min(LDR_BLOOM_LEVELS);
        let texture = |label, format, size: (u32, u32), mips| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        Self {
            size: (w, h),
            format,
            scene: texture("post scene", format, (w, h), 1),
            bloom: texture("cheap bloom", HDR_FORMAT, bloom_size, bloom_levels),
            bloom_levels,
        }
    }

    fn bloom_mip(&self, level: u32) -> wgpu::TextureView {
        self.bloom.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cheap bloom mip"),
            base_mip_level: level,
            mip_level_count: Some(1),
            ..wgpu::TextureViewDescriptor::default()
        })
    }
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
    /// `camera`; `None` when it's behind the camera. (`strength` 0 is no
    /// flare, but god rays still use the position.)
    pub fn new(
        camera: &crate::Camera3D,
        viewport: (u32, u32),
        direction: glam::Vec3,
        sun_size_degrees: f32,
        strength: f32,
        color: [f32; 3],
    ) -> Option<Self> {
        let clip = camera.view_proj(viewport) * (-direction.normalize_or_zero()).extend(0.0);
        if clip.w <= 1e-4 {
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
    /// The scene's depth (copyable, for water; sampled by soft particles).
    pub depth: wgpu::Texture,
    /// Copies water reads while drawing over `hdr` / `depth`.
    pub scene_copy: wgpu::Texture,
    pub depth_copy: wgpu::Texture,
    bloom: wgpu::Texture,
    bloom_levels: u32,
    ldr: wgpu::Texture,
    /// 1x1: how much of the sun disk is unblocked this frame.
    sun_visibility: wgpu::Texture,
    /// Quarter size: the sky around the sun, then the rays from it.
    rays_mask: wgpu::Texture,
    rays: wgpu::Texture,
    /// Half size: the depth-of-field blur.
    dof: wgpu::Texture,
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
                U::RENDER_ATTACHMENT | U::COPY_SRC | U::TEXTURE_BINDING,
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
            rays_mask: texture(
                "god rays mask",
                HDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                quarter(size),
                1,
            ),
            rays: texture(
                "god rays",
                HDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                quarter(size),
                1,
            ),
            dof: texture(
                "depth of field",
                HDR_FORMAT,
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                bloom_size,
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

/// A quarter of `size` (at least 1x1).
fn quarter(size: (u32, u32)) -> (u32, u32) {
    ((size.0 / 4).max(1), (size.1 / 4).max(1))
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
    ldr_prefilter: wgpu::RenderPipeline,
    rays_mask: wgpu::RenderPipeline,
    rays: wgpu::RenderPipeline,
    dof: wgpu::RenderPipeline,
    /// FXAA pipelines per target format.
    fxaa: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// Grade pipelines per target format (Medium to Ultra).
    grade: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// Outlines blended over the scene image, per format (HDR for
    /// Beautiful, the target's for Medium to Ultra).
    outline: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    sampler: wgpu::Sampler,
    placeholder: wgpu::TextureView,
    placeholder_depth: wgpu::TextureView,
    pub targets: Option<PostTargets>,
    pub ldr: Option<LdrTargets>,
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
                texture_entry(5),
                texture_entry(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
        let ldr_prefilter = fullscreen_pipeline(
            device,
            &layout,
            &shader,
            "fs_ldr_prefilter",
            HDR_FORMAT,
            None,
            None,
        );
        let pass =
            |entry| fullscreen_pipeline(device, &layout, &shader, entry, HDR_FORMAT, None, None);
        let rays_mask = pass("fs_rays_mask");
        let rays = pass("fs_rays");
        let dof = pass("fs_dof");
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
        let placeholder_depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("post placeholder depth"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_DEPTH_FORMAT,
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
            ldr_prefilter,
            rays_mask,
            rays,
            dof,
            fxaa: Vec::new(),
            grade: Vec::new(),
            outline: Vec::new(),
            sampler,
            placeholder,
            placeholder_depth,
            targets: None,
            ldr: None,
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
            let pipeline = self.final_pipeline(device, "fs_fxaa", format);
            self.fxaa.push((format, pipeline));
        }
        self.ensure_outline(device, HDR_FORMAT);
    }

    /// Makes sure an outline pipeline exists for `format`.
    fn ensure_outline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if !self.outline.iter().any(|(f, _)| *f == format) {
            // Blends the colour in by the edge's alpha and keeps the
            // image's own alpha (Beautiful's coverage).
            let blend = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            let pipeline = fullscreen_pipeline(
                device,
                &self.layout,
                &self.shader,
                "fs_outline",
                format,
                Some(blend),
                None,
            );
            self.outline.push((format, pipeline));
        }
    }

    /// Draws `style`'s outlines over `target` (whose format is `format`),
    /// from the scene's `depth`.
    pub fn record_outline(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        depth: &wgpu::TextureView,
        style: &Stylize,
    ) {
        let Some((_, pipeline)) = self.outline.iter().find(|(f, _)| *f == format) else {
            return;
        };
        let uniform = style.fill(PostUniform::default());
        let bind = self.bind_all(device, &self.placeholder, uniform, [None; 4], Some(depth));
        fullscreen_pass(
            encoder,
            "outlines",
            target,
            wgpu::LoadOp::Load,
            pipeline,
            &bind,
        );
    }

    /// A pipeline drawing into the caller's final pass (which has a depth
    /// buffer for the HUD sprites that follow).
    fn final_pipeline(
        &self,
        device: &wgpu::Device,
        entry: &str,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let depth = Some(wgpu::DepthStencilState {
            format: crate::renderer::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        fullscreen_pipeline(
            device,
            &self.layout,
            &self.shader,
            entry,
            format,
            None,
            depth,
        )
    }

    /// Makes sure Medium-to-Ultra's targets match `size` and `format`, and
    /// a grade pipeline exists for `format`.
    pub fn ensure_ldr(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) {
        let size = (size.0.max(1), size.1.max(1));
        if self.ldr.as_ref().map(|t| (t.size, t.format)) != Some((size, format)) {
            self.ldr = Some(LdrTargets::new(device, size, format));
        }
        if !self.grade.iter().any(|(f, _)| *f == format) {
            let pipeline = self.final_pipeline(device, "fs_grade", format);
            self.grade.push((format, pipeline));
        }
        self.ensure_outline(device, format);
    }

    /// Frees the size-dependent targets (when leaving the Beautiful preset).
    pub fn release(&mut self) {
        self.targets = None;
    }

    /// Frees Medium-to-Ultra's targets (Low, Beautiful, or no post effects).
    pub fn release_ldr(&mut self) {
        self.ldr = None;
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        src: &wgpu::TextureView,
        uniform: PostUniform,
        extra: Option<&wgpu::TextureView>,
        extra2: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        self.bind_all(device, src, uniform, [extra, extra2, None, None], None)
    }

    /// A bind group with every input: the source, up to four more images
    /// (bindings 3 to 6) and the scene's depth; missing ones are 1x1
    /// placeholders.
    fn bind_all(
        &self,
        device: &wgpu::Device,
        src: &wgpu::TextureView,
        uniform: PostUniform,
        extras: [Option<&wgpu::TextureView>; 4],
        depth: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("post"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let views = extras.map(|v| v.unwrap_or(&self.placeholder));
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
                    resource: wgpu::BindingResource::TextureView(views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(
                        depth.unwrap_or(&self.placeholder_depth),
                    ),
                },
            ],
        })
    }

    /// Bloom (down then up the mip chain), god rays and depth of field as
    /// `lens` asks, and the HDR → LDR resolve with `exposure` and tone
    /// mapping curve `tonemap` (the shader's index), plus the sun's lens
    /// flare if there is one (`sun` with a strength).
    #[allow(clippy::too_many_lines)] // one pass sequence, read top to bottom
    #[allow(clippy::too_many_arguments)]
    pub fn record_bloom_and_resolve(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        exposure: f32,
        tonemap: f32,
        sun: Option<SunFlare>,
        grade: Grade,
        bloom: f32,
        lens: Lens,
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
        let strength = BLOOM * bloom.clamp(0.0, 4.0) / t.bloom_levels.max(1) as f32;
        let aspect = t.size.0 as f32 / t.size.1.max(1) as f32;
        let flare = sun
            .map(|f| SunFlare {
                strength: f.strength * f.on_screen(),
                ..f
            })
            .filter(|f| f.strength > 0.0);
        let rays = sun.filter(|_| lens.god_rays > 0.0);
        let dof = lens.depth_of_field > 0.0;
        let (sun, sun_color) = sun.map_or(([0.0; 4], [0.0, 0.0, 0.0, aspect]), |f| {
            (
                [
                    f.uv[0],
                    f.uv[1],
                    flare.map_or(0.0, |f| f.strength),
                    f.radius,
                ],
                [f.color[0], f.color[1], f.color[2], aspect],
            )
        });
        let lens_uniform = [
            if rays.is_some() { lens.god_rays } else { 0.0 },
            lens.depth_of_field,
            lens.focus,
            lens.range,
        ];
        let depth_uniform = [lens.near, lens.far, t.size.0 as f32, t.size.1 as f32];
        let uniform = |texel: (f32, f32)| {
            grade.into_uniform(
                PostUniform {
                    texel: [1.0 / texel.0, 1.0 / texel.1, 0.0, 0.0],
                    params: [0.0, exposure, tonemap, strength],
                    sun,
                    sun_color,
                    lens: lens_uniform,
                    depth: depth_uniform,
                    ..PostUniform::default()
                },
                0.0,
            )
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
        // God rays: the sky around the sun, streaked toward it.
        let depth_view = t.depth.create_view(&wgpu::TextureViewDescriptor::default());
        let rays_view = t.rays.create_view(&wgpu::TextureViewDescriptor::default());
        if rays.is_some() {
            let mask_view = t
                .rays_mask
                .create_view(&wgpu::TextureViewDescriptor::default());
            let full = (t.size.0 as f32, t.size.1 as f32);
            let bind = self.bind_all(
                device,
                &hdr_view,
                uniform(full),
                [None; 4],
                Some(&depth_view),
            );
            fullscreen_pass(
                encoder,
                "god rays mask",
                &mask_view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.rays_mask,
                &bind,
            );
            let q = quarter(t.size);
            let bind = self.bind(
                device,
                &mask_view,
                uniform((q.0 as f32, q.1 as f32)),
                None,
                None,
            );
            fullscreen_pass(
                encoder,
                "god rays",
                &rays_view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.rays,
                &bind,
            );
        }
        // Depth of field: the scene blurred by its distance from the focus.
        let dof_view = t.dof.create_view(&wgpu::TextureViewDescriptor::default());
        if dof {
            let bind = self.bind_all(
                device,
                &hdr_view,
                uniform((t.size.0 as f32, t.size.1 as f32)),
                [None; 4],
                Some(&depth_view),
            );
            fullscreen_pass(
                encoder,
                "depth of field",
                &dof_view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.dof,
                &bind,
            );
        }
        // Resolve: HDR (over its blur) + bloom + rays, exposure, tone
        // mapping.
        let ldr_view = t.ldr.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.bind_all(
            device,
            &hdr_view,
            uniform((t.size.0 as f32, t.size.1 as f32)),
            [
                Some(&mips[0]),
                flare.map(|_| &visibility),
                rays.map(|_| &rays_view),
                dof.then_some(&dof_view),
            ],
            dof.then_some(&depth_view),
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
        style: &Stylize,
    ) {
        let (Some(t), Some((_, pipeline))) =
            (&self.targets, self.fxaa.iter().find(|(f, _)| *f == format))
        else {
            return;
        };
        let ldr_view = t.ldr.create_view(&wgpu::TextureViewDescriptor::default());
        let uniform = style.fill(PostUniform {
            texel: [1.0 / t.size.0 as f32, 1.0 / t.size.1 as f32, 0.0, 0.0],
            output: [f32::from(u8::from(!format.is_srgb())), 0.0, 0.0, 0.0],
            ..PostUniform::default()
        });
        let bind = self.bind(device, &ldr_view, uniform, None, None);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Medium to Ultra: the cheap bloom of [`LdrTargets::scene`] (High and
    /// Ultra; nothing when `chain.bloom` is 0).
    pub fn record_ldr_bloom(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        chain: &LdrChain,
    ) {
        let Some(t) = &self.ldr else {
            return;
        };
        if chain.bloom <= 0.0 {
            return;
        }
        let scene = t.scene.create_view(&wgpu::TextureViewDescriptor::default());
        let mips: Vec<wgpu::TextureView> = (0..t.bloom_levels).map(|i| t.bloom_mip(i)).collect();
        let size_of = |level: u32| {
            let w = (t.size.0 / 4).max(1) >> level;
            let h = (t.size.1 / 4).max(1) >> level;
            (w.max(1) as f32, h.max(1) as f32)
        };
        let uniform = |texel: (f32, f32)| PostUniform {
            texel: [1.0 / texel.0, 1.0 / texel.1, 0.0, 0.0],
            ..PostUniform::default()
        };
        for level in 0..t.bloom_levels {
            let (src, src_size) = if level == 0 {
                (&scene, (t.size.0 as f32, t.size.1 as f32))
            } else {
                (&mips[level as usize - 1], size_of(level - 1))
            };
            let bind = self.bind(device, src, uniform(src_size), None, None);
            fullscreen_pass(
                encoder,
                "cheap bloom down",
                &mips[level as usize],
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                if level == 0 {
                    &self.ldr_prefilter
                } else {
                    &self.downsample
                },
                &bind,
            );
        }
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
                "cheap bloom up",
                &mips[level as usize - 1],
                wgpu::LoadOp::Load,
                &self.upsample,
                &bind,
            );
        }
    }

    /// Medium to Ultra: grades [`LdrTargets::scene`] (plus the cheap bloom)
    /// into `pass`, whose color target has `format`.
    pub fn draw_grade(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        format: wgpu::TextureFormat,
        chain: &LdrChain,
    ) {
        let (Some(t), Some((_, pipeline))) =
            (&self.ldr, self.grade.iter().find(|(f, _)| *f == format))
        else {
            return;
        };
        let scene = t.scene.create_view(&wgpu::TextureViewDescriptor::default());
        let bloom = (chain.bloom > 0.0).then(|| t.bloom_mip(0));
        let uniform = chain.style.fill(chain.grade.into_uniform(
            PostUniform {
                texel: [1.0 / t.size.0 as f32, 1.0 / t.size.1 as f32, 0.0, 0.0],
                ..PostUniform::default()
            },
            chain.bloom,
        ));
        let bind = self.bind(device, &scene, uniform, bloom.as_ref(), None);
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
        assert_eq!(std::mem::size_of::<PostUniform>(), 224);
    }

    #[test]
    fn the_ldr_chain_runs_only_when_it_changes_something() {
        use crate::{GraphicsQuality as Q, PostEffects};
        let neutral = PostEffects::default();
        let plain = Stylize::default();
        let graded = PostEffects::preset("vivid").unwrap();
        let no_bloom = PostEffects {
            bloom: 0.0,
            ..neutral
        };
        // Low never, Beautiful has its own chain.
        assert!(LdrChain::new(Q::Low, &graded, 0.0, plain).is_none());
        assert!(LdrChain::new(Q::Beautiful, &graded, 0.0, plain).is_none());
        // Medium: only for a grade (no bloom there).
        assert!(LdrChain::new(Q::Medium, &neutral, 0.0, plain).is_none());
        let medium = LdrChain::new(Q::Medium, &graded, 0.0, plain).unwrap();
        assert!(medium.bloom == 0.0 && medium.grade.misc[2] > 0.5);
        // High: bloom by default, nothing with it off and no grade.
        let high = LdrChain::new(Q::High, &neutral, 0.0, plain).unwrap();
        assert!(high.bloom > 0.0 && high.grade.misc[2] < 0.5);
        assert!(LdrChain::new(Q::High, &no_bloom, 0.0, plain).is_none());
        // Chromatic aberration alone still needs the pass.
        let fringe = PostEffects {
            chromatic_aberration: 0.5,
            ..no_bloom
        };
        assert!(LdrChain::new(Q::Medium, &fringe, 0.0, plain).is_some());
        // So do outlines and pixel art, but not on Low.
        let env = crate::Environment {
            outline: 1.0,
            ..crate::Environment::default()
        };
        let cam = crate::Camera3D::default();
        let outlined = Stylize::new(&env, Some(&cam), (1280, 720));
        assert!(outlined.outlines() && !outlined.pixel_art());
        assert!(LdrChain::new(Q::High, &no_bloom, 0.0, outlined).is_some());
        assert!(LdrChain::new(Q::Low, &no_bloom, 0.0, outlined).is_none());
        let pixels = Stylize::new(
            &crate::Environment {
                pixelate: 3.0,
                ..crate::Environment::default()
            },
            Some(&cam),
            (1280, 720),
        );
        assert!(pixels.pixel_art() && !pixels.outlines());
        assert!(LdrChain::new(Q::Medium, &neutral, 0.0, pixels).is_some());
    }

    #[test]
    fn style_sizes_scale_with_the_screen() {
        let cam = crate::Camera3D::default();
        let env = crate::Environment {
            outline: 1.0,
            pixelate: 3.0,
            palette: 12,
            ..crate::Environment::default()
        };
        let hd = Stylize::new(&env, Some(&cam), (1280, 720));
        assert!((hd.pixel - 3.0).abs() < 1e-6 && (hd.outline - 3.0).abs() < 1e-6);
        assert!((hd.palette - 12.0).abs() < 1e-6);
        let uhd = Stylize::new(&env, Some(&cam), (3840, 2160));
        assert!((uhd.pixel - 9.0).abs() < 1e-6);
        // Blocks never shrink below a pixel; no outlines without a camera.
        let tiny = Stylize::new(&env, None, (160, 90));
        assert!((tiny.pixel - 1.0).abs() < 1e-6 && !tiny.outlines());
        // A one-level palette is no palette.
        let flat = Stylize::new(
            &crate::Environment {
                palette: 1,
                ..crate::Environment::default()
            },
            Some(&cam),
            (1280, 720),
        );
        assert!(!flat.pixel_art());
    }

    #[test]
    fn the_sun_projects_where_the_camera_looks() {
        let cam = crate::Camera3D::new(glam::Vec3::ZERO, glam::Vec3::new(0.0, 0.0, -10.0));
        // Light traveling toward +Z: the sun is straight ahead (-Z).
        let f = SunFlare::new(&cam, (800, 600), glam::Vec3::Z, 2.0, 1.0, [1.0; 3]).unwrap();
        assert!((f.uv[0] - 0.5).abs() < 1e-4 && (f.uv[1] - 0.5).abs() < 1e-4);
        assert!(f.radius > 0.0 && f.radius < 0.05);
        assert!((f.on_screen() - 1.0).abs() < 1e-6);
        // No flare, but still a position (for god rays).
        assert!(SunFlare::new(&cam, (800, 600), glam::Vec3::Z, 2.0, 0.0, [1.0; 3]).is_some());
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
