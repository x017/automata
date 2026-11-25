//! Particles: thousands of small camera-facing quads (fire, smoke, sparks,
//! dust) drawn in instanced batches.
//!
//! The renderer only draws them: the scene simulates emitters on the CPU
//! and hands each frame's particles over as [`ParticleBatch`]es (see
//! [`crate::DrawList::particles`]). A batch shares a look: its texture,
//! blending, and whether it's lit and soft. Every particle is one
//! [`ParticleInstance`]: 64 bytes, four vertices from a triangle strip, no
//! vertex buffer of its own.
//!
//! Particles draw after the opaque scene and the water, depth-tested but
//! not writing depth. Alpha-blended batches go first, each sorted back to
//! front, then additive ones (whose order doesn't matter). Soft particles
//! read the depth buffer to fade where they meet the scene. On Beautiful
//! they're HDR: bright additive ones bloom.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::pipeline::{HDR_DEPTH_FORMAT, HDR_FORMAT};
use crate::texture::Texture;

/// WGSL source of the particle shader (with the shared lighting code).
pub(crate) const PARTICLE_SHADER: &str = concat!(
    include_str!("shaders/tonemap.wgsl"),
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/particles.wgsl")
);

/// The most particles drawn in one frame; batches past it are cut short.
pub const MAX_PARTICLES_PER_FRAME: usize = 65_536;

/// How a batch's particles combine with what's behind them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ParticleBlend {
    /// Covers what's behind by its alpha: smoke, dust, splashes.
    #[default]
    Alpha,
    /// Adds its light: fire, sparks, magic. Order doesn't matter, and on
    /// Beautiful bright ones bloom.
    Additive,
}

/// One particle as the GPU sees it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct ParticleInstance {
    /// Centre, in world space.
    pub position: [f32; 3],
    /// Width (and height) in metres.
    pub size: f32,
    /// Linear RGBA, multiplying the texture.
    pub color: [f32; 4],
    /// The texture rectangle: `[u0, v0, u1, v1]` (see [`ParticleSprite::uv`]).
    pub uv: [f32; 4],
    /// A world-space streak: the quad stretches back along it, facing the
    /// camera (sparks, rain). Zero for a plain camera-facing quad.
    pub streak: [f32; 3],
    /// Spin in the view plane, radians (ignored when streaked).
    pub rotation: f32,
}

/// A frame's worth of particles that share a look.
#[derive(Debug, Clone, Default)]
pub struct ParticleBatch {
    /// The particles. Alpha batches needn't be sorted: the renderer sorts
    /// them back to front.
    pub instances: Vec<ParticleInstance>,
    /// How they blend.
    pub blend: ParticleBlend,
    /// Lit by the sun, sky and point lights (smoke, dust), or glowing at
    /// their own colour (fire, sparks).
    pub lit: bool,
    /// Over how many metres they fade into the scene behind them (0 = a
    /// hard edge where they cross it).
    pub soft: f32,
    /// Their texture; `None` is the built-in sprite sheet (see
    /// [`ParticleSprite`]).
    pub texture: Option<Texture>,
    /// Tiles the particles around the camera in boxes this big (metres
    /// per side): each particle is drawn at its copy nearest the camera,
    /// fading out toward the box's faces, so a small box of dust fills
    /// the air wherever the camera goes. `None` draws them where they are.
    pub wrap: Option<[f32; 3]>,
}

/// The built-in particle sprites, all white (particles tint them).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ParticleSprite {
    /// A soft round blob.
    #[default]
    Dot,
    /// A small bright point with a sharp core (sparks, embers).
    Spark,
    /// A four-pointed twinkle (magic).
    Star,
    /// A tongue of flame, pointing up.
    Flame,
    /// A billowy puff of smoke or cloud; four variants.
    Puff,
}

/// Cells across and down the built-in sprite sheet.
const SHEET_COLS: u32 = 4;
const SHEET_ROWS: u32 = 2;
/// Pixels per sprite cell.
const CELL: u32 = 64;

impl ParticleSprite {
    /// Every built-in sprite.
    pub const ALL: [Self; 5] = [Self::Dot, Self::Spark, Self::Star, Self::Flame, Self::Puff];

    /// How many variants it comes in (particles pick one at random).
    #[must_use]
    pub fn variants(self) -> u32 {
        match self {
            Self::Puff => 4,
            _ => 1,
        }
    }

    /// The texture rectangle of variant `variant` (wrapping) in the
    /// built-in sheet.
    #[must_use]
    pub fn uv(self, variant: u32) -> [f32; 4] {
        let (col, row) = match self {
            Self::Dot => (0, 0),
            Self::Spark => (1, 0),
            Self::Star => (2, 0),
            Self::Flame => (3, 0),
            Self::Puff => (variant % 4, 1),
        };
        sheet_cell(col, row, SHEET_COLS, SHEET_ROWS)
    }

    /// Its name in scene files and scripts.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Dot => "Dot",
            Self::Spark => "Spark",
            Self::Star => "Star",
            Self::Flame => "Flame",
            Self::Puff => "Puff",
        }
    }
}

/// The rectangle of cell `(col, row)` in a sheet of `cols` x `rows`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // small counts
pub fn sheet_cell(col: u32, row: u32, cols: u32, rows: u32) -> [f32; 4] {
    let (w, h) = (1.0 / cols.max(1) as f32, 1.0 / rows.max(1) as f32);
    let (u, v) = (col as f32 * w, row as f32 * h);
    [u, v, u + w, v + h]
}

/// A hash of integer lattice point `(x, y)` with `seed` into [0, 1).
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    #[allow(clippy::cast_sign_loss)] // bit mixing
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    #[allow(clippy::cast_precision_loss)] // 24 bits
    let v = (h & 0x00ff_ffff) as f32;
    v / 16_777_216.0
}

/// Smooth value noise at `(x, y)`.
#[allow(clippy::cast_possible_truncation)] // floor of small values
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(xi, yi, seed) + (hash(xi + 1, yi, seed) - hash(xi, yi, seed)) * sx;
    let b = hash(xi, yi + 1, seed) + (hash(xi + 1, yi + 1, seed) - hash(xi, yi + 1, seed)) * sx;
    a + (b - a) * sy
}

/// Four octaves of value noise, about 0..1.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq) = (0.0, 0.5, 1.0);
    for octave in 0..4 {
        sum += amp * value_noise(x * freq, y * freq, seed + octave);
        amp *= 0.5;
        freq *= 2.0;
    }
    sum
}

/// The alpha of a built-in sprite at `(x, y)` in its cell, both -1..1
/// (y up); `variant` picks a puff.
fn sprite_alpha(sprite: ParticleSprite, x: f32, y: f32, variant: u32) -> f32 {
    let r = (x * x + y * y).sqrt();
    match sprite {
        ParticleSprite::Dot => {
            let t = (1.0 - r).max(0.0);
            t * t * (3.0 - 2.0 * t)
        }
        ParticleSprite::Spark => {
            let core = (1.0 - r * 3.0).clamp(0.0, 1.0);
            let glow = (1.0 - r).max(0.0).powi(3);
            (core + glow * 0.6).min(1.0)
        }
        ParticleSprite::Star => {
            let arms = (1.0 - (x.abs() * y.abs()).sqrt() * 6.0).max(0.0) * (1.0 - r).max(0.0);
            let core = (1.0 - r * 2.5).max(0.0);
            (arms * arms + core * core).min(1.0)
        }
        ParticleSprite::Flame => {
            // A teardrop: round at the bottom, narrowing to a point on top.
            let v = f32::midpoint(y, 1.0); // 0 bottom .. 1 top
            let width = (1.0 - v).sqrt() * (v * 4.0).min(1.0) * 0.8;
            let across = if width > 0.0 { x.abs() / width } else { 2.0 };
            let edge = (1.0 - across).clamp(0.0, 1.0);
            let wobble = 0.75 + 0.25 * fbm(x * 3.0 + 7.0, y * 2.0, 11);
            (edge * edge * (3.0 - 2.0 * edge) * wobble).min(1.0)
        }
        ParticleSprite::Puff => {
            let seed = 101 + variant * 17;
            let n = fbm(x * 2.2 + 3.0, y * 2.2 + 5.0, seed);
            let edge = (1.0 - r * (0.75 + 0.6 * n)).clamp(0.0, 1.0);
            (edge * edge * (0.55 + 0.9 * n)).clamp(0.0, 1.0)
        }
    }
}

/// The built-in sprite sheet: RGBA8, `CELL * SHEET_COLS` x `CELL *
/// SHEET_ROWS`, white with each sprite in its alpha.
#[must_use]
#[allow(clippy::many_single_char_names)] // pixel coordinates
pub(crate) fn sprite_sheet() -> (u32, u32, Vec<u8>) {
    let (w, h) = (CELL * SHEET_COLS, CELL * SHEET_ROWS);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut put = |sprite: ParticleSprite, variant: u32, col: u32, row: u32| {
        for py in 0..CELL {
            for px in 0..CELL {
                // Pixel centres, a 2-pixel transparent margin for filtering.
                #[allow(clippy::cast_precision_loss)] // small
                let to_unit = |p: u32| ((p as f32 + 0.5) / CELL as f32 * 2.0 - 1.0) * 1.07;
                let (x, y) = (to_unit(px), -to_unit(py));
                // Every sprite ends inside its cell's circle.
                let window = ((1.0 - (x * x + y * y).sqrt()) * 8.0).clamp(0.0, 1.0);
                let a = sprite_alpha(sprite, x, y, variant) * window;
                let i = (((row * CELL + py) * w + col * CELL + px) * 4) as usize;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0..255
                let byte = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, byte]);
            }
        }
    };
    put(ParticleSprite::Dot, 0, 0, 0);
    put(ParticleSprite::Spark, 0, 1, 0);
    put(ParticleSprite::Star, 0, 2, 0);
    put(ParticleSprite::Flame, 0, 3, 0);
    for v in 0..4 {
        put(ParticleSprite::Puff, v, v, 1);
    }
    (w, h, rgba)
}

/// One batch's slice of the frame's instances.
#[derive(Debug)]
pub(crate) struct ParticleRun {
    range: Range<u32>,
    additive: bool,
    lit: bool,
    soft: f32,
    texture: Option<Texture>,
}

/// The particle pipelines, the built-in sprite sheet, and the instance
/// buffer.
#[derive(Debug)]
pub(crate) struct Particles {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    /// Pipelines by target format and additive blending.
    pipelines: Vec<(wgpu::TextureFormat, bool, wgpu::RenderPipeline)>,
    sheet: Texture,
    instances: wgpu::Buffer,
    capacity: usize,
}

impl Particles {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Depth read with textureLoad: unfilterable float, since GLSL
                        // (WebGL2) can't textureLoad a depth texture.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(PARTICLE_SHADER.into()),
        });
        let (w, h, rgba) = sprite_sheet();
        let sheet = Texture::from_rgba_mipmapped(device, queue, w, h, &rgba, false);
        let capacity = 1024;
        Self {
            layout,
            shader,
            pipelines: Vec::new(),
            sheet,
            instances: Self::instance_buffer(device, capacity),
            capacity,
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle instances"),
            size: (capacity * std::mem::size_of::<ParticleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Builds the pipelines a frame drawing into `format` needs.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        light_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) {
        for additive in [false, true] {
            if self
                .pipelines
                .iter()
                .any(|(f, a, _)| *f == format && *a == additive)
            {
                continue;
            }
            let pipeline =
                self.create_pipeline(device, camera_layout, light_layout, format, additive);
            self.pipelines.push((format, additive, pipeline));
        }
    }

    fn create_pipeline(
        &self,
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        light_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        additive: bool,
    ) -> wgpu::RenderPipeline {
        let hdr = format == HDR_FORMAT;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles"),
            bind_group_layouts: &[
                Some(camera_layout),
                Some(&self.layout),
                None,
                Some(light_layout),
            ],
            immediate_size: 0,
        });
        let blend = blend_state(hdr, additive);
        let entry = match (hdr, additive) {
            (false, false) => "fs_ldr_alpha",
            (false, true) => "fs_ldr_add",
            (true, false) => "fs_hdr_alpha",
            (true, true) => "fs_hdr_add",
        };
        let depth_format = if hdr {
            HDR_DEPTH_FORMAT
        } else {
            crate::renderer::DEPTH_FORMAT
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(instance_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Uploads the frame's particles, alpha batches sorted back to front
    /// from `eye` (looking along `forward`), and returns what to draw.
    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batches: &[ParticleBatch],
        eye: glam::Vec3,
        forward: glam::Vec3,
    ) -> Vec<ParticleRun> {
        let (instances, runs) = order_batches(batches, eye, forward);
        if instances.is_empty() {
            return Vec::new();
        }
        if instances.len() > self.capacity {
            self.capacity = instances.len().next_power_of_two();
            self.instances = Self::instance_buffer(device, self.capacity);
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        runs
    }

    /// Records `runs` into `pass` (a pass drawing into `format`, its depth
    /// attached read-only and sampled from `depth`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        runs: &[ParticleRun],
        format: wgpu::TextureFormat,
        camera: &crate::Camera3D,
        camera_bind: &wgpu::BindGroup,
        light_bind: &wgpu::BindGroup,
        depth: &wgpu::TextureView,
    ) {
        if runs.is_empty() {
            return;
        }
        let (right, up) = billboard_axes(camera);
        pass.set_bind_group(0, camera_bind, &[]);
        pass.set_bind_group(3, light_bind, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        let mut current: Option<bool> = None;
        for run in runs {
            let Some((_, _, pipeline)) = self
                .pipelines
                .iter()
                .find(|(f, a, _)| *f == format && *a == run.additive)
            else {
                continue;
            };
            if current != Some(run.additive) {
                pass.set_pipeline(pipeline);
                current = Some(run.additive);
            }
            let uniform = [
                right.x,
                right.y,
                right.z,
                run.soft.max(0.0),
                up.x,
                up.y,
                up.z,
                if run.lit { 1.0 } else { 0.0 },
            ];
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particle batch"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let texture = run.texture.as_ref().unwrap_or(&self.sheet);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle batch"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&texture.inner.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&texture.inner.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                ],
            });
            pass.set_bind_group(1, &bind, &[]);
            pass.draw(0..4, run.range.clone());
        }
    }
}

/// The blend state for HDR or LDR targets, additive or alpha. On HDR
/// targets alpha accumulates coverage (the post chain tone maps by it); on
/// LDR ones additive particles leave it alone.
fn blend_state(hdr: bool, additive: bool) -> wgpu::BlendState {
    let coverage = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    match (hdr, additive) {
        (_, false) => wgpu::BlendState {
            color: wgpu::BlendState::ALPHA_BLENDING.color,
            alpha: coverage,
        },
        (true, true) => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: coverage,
        },
        (false, true) => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
    }
}

/// The per-instance vertex layout (slot 0), four `vec4`s.
fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ParticleInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// The camera's right and up, which particles face along.
fn billboard_axes(camera: &crate::Camera3D) -> (glam::Vec3, glam::Vec3) {
    let forward = (camera.target - camera.position).normalize_or(glam::Vec3::NEG_Z);
    let mut right = forward.cross(camera.up);
    if right.length_squared() < 1e-8 {
        right = forward.cross(glam::Vec3::Z);
    }
    let right = right.normalize();
    (right, right.cross(forward))
}

/// The frame's instances in draw order, and each batch's run of them:
/// alpha batches farthest first, each sorted back to front, then additive
/// ones. At most [`MAX_PARTICLES_PER_FRAME`].
pub(crate) fn order_batches(
    batches: &[ParticleBatch],
    eye: glam::Vec3,
    forward: glam::Vec3,
) -> (Vec<ParticleInstance>, Vec<ParticleRun>) {
    let depth = |p: &ParticleInstance| (glam::Vec3::from(p.position) - eye).dot(forward);
    let mut alpha: Vec<(f32, &ParticleBatch)> = batches
        .iter()
        .filter(|b| b.blend == ParticleBlend::Alpha && !b.instances.is_empty())
        .map(|b| {
            #[allow(clippy::cast_precision_loss)] // counts
            let mean = b.instances.iter().map(depth).sum::<f32>() / b.instances.len() as f32;
            (mean, b)
        })
        .collect();
    alpha.sort_by(|a, b| b.0.total_cmp(&a.0));
    let additive = batches
        .iter()
        .filter(|b| b.blend == ParticleBlend::Additive && !b.instances.is_empty());
    let mut instances = Vec::new();
    let mut runs = Vec::new();
    for (sorted, batch) in alpha
        .into_iter()
        .map(|(_, b)| (true, b))
        .chain(additive.map(|b| (false, b)))
    {
        let room = MAX_PARTICLES_PER_FRAME - instances.len();
        if room == 0 {
            break;
        }
        let start = instances.len();
        instances.extend(batch.instances.iter().take(room));
        if let Some(size) = batch.wrap {
            for p in &mut instances[start..] {
                wrap_around(p, eye, size);
            }
        }
        if sorted {
            instances[start..].sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        }
        #[allow(clippy::cast_possible_truncation)] // capped at 65 536
        runs.push(ParticleRun {
            range: start as u32..instances.len() as u32,
            additive: batch.blend == ParticleBlend::Additive,
            lit: batch.lit,
            soft: batch.soft,
            texture: batch.texture.clone(),
        });
    }
    (instances, runs)
}

/// Moves `p` to its copy nearest `eye` in a grid of `size` boxes, and
/// fades it over the outer fifth of the box so particles that wrap from
/// one side to the other don't pop.
fn wrap_around(p: &mut ParticleInstance, eye: glam::Vec3, size: [f32; 3]) {
    let mut fade = 1.0_f32;
    for axis in 0..3 {
        let side = size[axis].max(1e-3);
        let offset = (p.position[axis] - eye[axis]) / side;
        let local = offset - offset.round(); // -0.5..0.5
        p.position[axis] = eye[axis] + local * side;
        fade *= ((0.5 - local.abs()) / 0.1).clamp(0.0, 1.0);
    }
    p.color[3] *= fade;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(z: f32) -> ParticleInstance {
        ParticleInstance {
            position: [0.0, 0.0, z],
            size: 1.0,
            color: [1.0; 4],
            ..ParticleInstance::default()
        }
    }

    #[test]
    fn wrapped_particles_tile_around_the_camera() {
        let batch = ParticleBatch {
            instances: vec![at(-23.0), at(-0.5)],
            wrap: Some([10.0, 10.0, 10.0]),
            ..ParticleBatch::default()
        };
        let eye = glam::Vec3::new(0.0, 0.0, 1.0);
        let (instances, _) = order_batches(&[batch], eye, glam::Vec3::NEG_Z);
        // -23 is 24 m behind the eye: its copy 6 m farther in front is
        // nearer (within the 10 m box's half), and it faces outward so it
        // fades; -0.5 is well inside and keeps all its colour.
        let far = instances[0];
        assert!((far.position[2] - -3.0).abs() < 1e-4, "{:?}", far.position);
        assert!((far.color[3] - 1.0).abs() < 1e-6);
        let near = instances[1];
        assert!((near.position[2] - -0.5).abs() < 1e-4);
        let edge = ParticleBatch {
            instances: vec![at(5.6)],
            wrap: Some([10.0, 10.0, 10.0]),
            ..ParticleBatch::default()
        };
        let (instances, _) = order_batches(&[edge], eye, glam::Vec3::NEG_Z);
        assert!(instances[0].color[3] < 0.5, "near the box's face it fades");
    }

    #[test]
    fn instances_are_four_vec4s() {
        assert_eq!(std::mem::size_of::<ParticleInstance>(), 64);
        assert_eq!(instance_layout().attributes.len(), 4);
    }

    #[test]
    fn alpha_batches_draw_back_to_front_before_additive_ones() {
        let batch = |zs: &[f32], blend| ParticleBatch {
            instances: zs.iter().map(|&z| at(z)).collect(),
            blend,
            ..ParticleBatch::default()
        };
        // The camera at the origin looks down -Z: more negative is farther.
        let batches = [
            batch(&[-1.0, -5.0], ParticleBlend::Additive),
            batch(&[-2.0, -8.0, -4.0], ParticleBlend::Alpha),
            batch(&[-20.0], ParticleBlend::Alpha),
            batch(&[], ParticleBlend::Alpha),
        ];
        let (instances, runs) = order_batches(&batches, glam::Vec3::ZERO, glam::Vec3::NEG_Z);
        let zs: Vec<f32> = instances.iter().map(|p| p.position[2]).collect();
        assert_eq!(zs, [-20.0, -8.0, -4.0, -2.0, -1.0, -5.0]);
        assert_eq!(runs.len(), 3, "the empty batch draws nothing");
        assert_eq!(runs[0].range, 0..1);
        assert_eq!(runs[1].range, 1..4);
        assert!(runs[2].additive && runs[2].range == (4..6));
    }

    #[test]
    fn a_frame_draws_at_most_the_cap() {
        let big = ParticleBatch {
            instances: vec![at(-1.0); MAX_PARTICLES_PER_FRAME + 10],
            ..ParticleBatch::default()
        };
        let (instances, runs) =
            order_batches(&[big.clone(), big], glam::Vec3::ZERO, glam::Vec3::NEG_Z);
        assert_eq!(instances.len(), MAX_PARTICLES_PER_FRAME);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn the_sprite_sheet_holds_every_sprite_inside_its_cell() {
        let (w, h, rgba) = sprite_sheet();
        assert_eq!((w, h), (256, 128));
        let alpha = |x: u32, y: u32| rgba[((y * w + x) * 4 + 3) as usize];
        for sprite in ParticleSprite::ALL {
            for v in 0..sprite.variants() {
                let [u0, v0, u1, v1] = sprite.uv(v);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let px = |t: f32, size: u32| (t * size as f32) as u32;
                let (x0, y0, x1, y1) = (px(u0, w), px(v0, h), px(u1, w), px(v1, h));
                let (cx, cy) = (u32::midpoint(x0, x1), u32::midpoint(y0, y1));
                assert!(alpha(cx, cy + 4) > 100, "{sprite:?} {v} is visible");
                // A clear margin all round, so filtering doesn't bleed.
                for t in x0..x1 {
                    assert_eq!(alpha(t, y0), 0, "{sprite:?} top edge");
                    assert_eq!(alpha(t, y1 - 1), 0, "{sprite:?} bottom edge");
                }
                for t in y0..y1 {
                    assert_eq!(alpha(x0, t), 0, "{sprite:?} left edge");
                    assert_eq!(alpha(x1 - 1, t), 0, "{sprite:?} right edge");
                }
            }
        }
        // Puff variants differ.
        let [u0, ..] = ParticleSprite::Puff.uv(0);
        let [u1, ..] = ParticleSprite::Puff.uv(1);
        assert!((u0 - u1).abs() > 0.1);
    }

    #[test]
    fn billboards_face_the_camera_even_looking_straight_down() {
        let level = crate::Camera3D::new(glam::Vec3::new(0.0, 0.0, 5.0), glam::Vec3::ZERO);
        let (right, up) = billboard_axes(&level);
        assert!((right - glam::Vec3::X).length() < 1e-5, "{right}");
        assert!((up - glam::Vec3::Y).length() < 1e-5, "{up}");
        let down = crate::Camera3D::new(glam::Vec3::new(0.0, 10.0, 0.0), glam::Vec3::ZERO);
        let (right, up) = billboard_axes(&down);
        assert!(right.is_finite() && up.is_finite());
        assert!(right.dot(glam::Vec3::Y).abs() < 1e-5 && up.dot(glam::Vec3::Y).abs() < 1e-5);
    }

    #[test]
    fn particle_wgsl_is_valid() {
        crate::pipeline::validate_wgsl("particles", PARTICLE_SHADER).unwrap();
    }
}
