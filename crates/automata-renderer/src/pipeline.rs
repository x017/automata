//! Render pipelines: sprite (2D) and mesh (3D), plus WGSL validation.
//!
//! Pipeline creation is expressed as data on `wgpu` descriptors so the
//! layout invariants are testable without a GPU device.

use crate::mesh::{SkinVertex, Vertex};
use crate::SPRITE_VERTEX_CAPACITY;

/// WGSL source of the 2D sprite shader.
pub(crate) const SPRITE_SHADER: &str = include_str!("shaders/sprite.wgsl");
/// WGSL source of the 3D mesh shader (with the shared lighting code).
pub(crate) const MESH_SHADER: &str = concat!(
    include_str!("shaders/tonemap.wgsl"),
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/mesh.wgsl")
);
/// WGSL source of the water shader (with the shared lighting code).
pub(crate) const WATER_SHADER: &str = concat!(
    include_str!("shaders/tonemap.wgsl"),
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/water.wgsl")
);
/// WGSL source of the Beautiful post chain (bloom, tone mapping, FXAA).
pub(crate) const POST_SHADER: &str = concat!(
    include_str!("shaders/tonemap.wgsl"),
    include_str!("shaders/post.wgsl")
);
/// WGSL source of the depth-only shadow pass shader.
pub(crate) const SHADOW_SHADER: &str = include_str!("shaders/shadow.wgsl");

/// Depth texture format of the shadow map (sampled as
/// `texture_depth_2d_array`, one layer per cascade).
pub(crate) const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Color format of the Beautiful path's HDR scene target.
pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Depth format of the Beautiful path: copyable, so water can read the
/// scene's depth.
pub(crate) const HDR_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Opaque (replacing) color target state.
fn opaque_target(format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

/// Alpha-blended color target state for the given surface format.
fn blend_target(format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

/// The HDR scene's color target: alpha-blended color, with alpha
/// accumulating coverage (the post chain tone maps by it).
fn coverage_target(format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendState::ALPHA_BLENDING.color,
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        }),
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

/// Creates a shader module from WGSL source.
fn shader_module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// All resources shared by both pipelines: shaders and bind group layouts.
pub(crate) struct PipelineShared {
    pub camera_layout: wgpu::BindGroupLayout,
    pub texture_layout: wgpu::BindGroupLayout,
    pub material_layout: wgpu::BindGroupLayout,
    pub model_layout: wgpu::BindGroupLayout,
    /// The model matrix plus the joint palette, for skinned draws.
    pub skinned_model_layout: wgpu::BindGroupLayout,
    pub light_layout: wgpu::BindGroupLayout,
    pub shadow_light_layout: wgpu::BindGroupLayout,
    /// The water uniform (vertex + fragment).
    pub water_layout: wgpu::BindGroupLayout,
    /// The scene color and depth copies water refracts through.
    pub scene_layout: wgpu::BindGroupLayout,
    pub sprite_shader: wgpu::ShaderModule,
    pub mesh_shader: wgpu::ShaderModule,
    pub shadow_shader: wgpu::ShaderModule,
    pub water_shader: wgpu::ShaderModule,
}

impl PipelineShared {
    #[allow(clippy::too_many_lines)] // one descriptor per layout
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        // The camera uniform carries the view-projection (vertex stage) and
        // the world-space camera position (fragment specular), so both
        // stages need visibility.
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let skinned_model_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skinned model"),
                entries: &[
                    uniform_entry(0, wgpu::ShaderStages::VERTEX),
                    uniform_entry(1, wgpu::ShaderStages::VERTEX),
                ],
            });
        // Mesh lighting: sun uniform + shadow map + comparison sampler.
        let light_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // The first cascade as a plain 2D texture: the forward
                // path's lookup (cheaper than an array on some GPUs).
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
        // The shadow pass needs only the light-space transform (vertex).
        let shadow_light_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow light"),
                entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
            });
        let water_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                // The baked ripple texture (water::ripple_texels) + sampler.
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
            ],
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene copy"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
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
            ],
        });

        Self {
            camera_layout,
            texture_layout,
            material_layout,
            model_layout,
            skinned_model_layout,
            light_layout,
            shadow_light_layout,
            water_layout,
            scene_layout,
            sprite_shader: shader_module(device, "sprite", SPRITE_SHADER),
            mesh_shader: shader_module(device, "mesh", MESH_SHADER),
            shadow_shader: shader_module(device, "shadow", SHADOW_SHADER),
            water_shader: shader_module(device, "water", WATER_SHADER),
        }
    }
}

/// A single-buffer uniform bind group layout entry.
fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Creates the 2D sprite pipeline for the given surface format.
pub(crate) fn create_sprite_pipeline(
    device: &wgpu::Device,
    shared: &PipelineShared,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite"),
        bind_group_layouts: &[Some(&shared.camera_layout), Some(&shared.texture_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.sprite_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SpriteVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2,
                    1 => Float32x2,
                    2 => Float32x4
                ],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..wgpu::PrimitiveState::default()
        },
        // The main pass always has a depth buffer, so the pipeline must
        // declare one too. Sprites draw on top of the 3D scene in submission
        // order: they ignore depth and never write it.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: crate::renderer::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shared.sprite_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &blend_target(format),
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The mesh vertex buffer layout (slot 0).
const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2
];
/// The skinning vertex buffer layout (slot 1, skinned pipelines only).
const SKIN_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
    3 => Uint16x4,
    4 => Float32x4
];

/// The vertex buffers of a mesh pipeline: geometry, plus joints and weights
/// when `skinned`.
fn mesh_buffers(skinned: bool) -> Vec<Option<wgpu::VertexBufferLayout<'static>>> {
    let mut buffers = vec![Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &MESH_ATTRIBUTES,
    })];
    if skinned {
        buffers.push(Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SkinVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SKIN_ATTRIBUTES,
        }));
    }
    buffers
}

/// The plain and skinned variants of a pipeline.
pub(crate) struct PipelinePair {
    pub plain: wgpu::RenderPipeline,
    pub skinned: wgpu::RenderPipeline,
}

impl PipelinePair {
    /// The variant for a draw.
    pub(crate) fn get(&self, skinned: bool) -> &wgpu::RenderPipeline {
        if skinned {
            &self.skinned
        } else {
            &self.plain
        }
    }
}

/// Both 3D mesh pipelines for the given color and depth formats.
pub(crate) fn create_mesh_pipelines(
    device: &wgpu::Device,
    shared: &PipelineShared,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
) -> PipelinePair {
    PipelinePair {
        plain: create_mesh_pipeline(device, shared, format, depth_format, false),
        skinned: create_mesh_pipeline(device, shared, format, depth_format, true),
    }
}

/// The water pipeline. `refract` (the Beautiful path) writes opaquely —
/// the shader composes the refracted scene itself; otherwise the surface
/// alpha-blends over the scene. Depth-tested, never depth-written.
pub(crate) fn create_water_pipeline(
    device: &wgpu::Device,
    shared: &PipelineShared,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    refract: bool,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("water"),
        bind_group_layouts: &[
            Some(&shared.camera_layout),
            Some(&shared.water_layout),
            Some(&shared.scene_layout),
            Some(&shared.light_layout),
        ],
        immediate_size: 0,
    });
    let targets = if refract {
        opaque_target(format)
    } else {
        blend_target(format)
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("water"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.water_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &mesh_buffers(false),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
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
            module: &shared.water_shader,
            entry_point: Some(if refract { "fs_refract" } else { "fs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Both shadow pipelines.
pub(crate) fn create_shadow_pipelines(
    device: &wgpu::Device,
    shared: &PipelineShared,
) -> PipelinePair {
    PipelinePair {
        plain: create_shadow_pipeline(device, shared, false),
        skinned: create_shadow_pipeline(device, shared, true),
    }
}

/// Creates the 3D mesh pipeline for the given surface format. The skinned
/// variant blends each vertex by its joints (`vs_skinned`).
fn create_mesh_pipeline(
    device: &wgpu::Device,
    shared: &PipelineShared,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    skinned: bool,
) -> wgpu::RenderPipeline {
    let model_layout = if skinned {
        &shared.skinned_model_layout
    } else {
        &shared.model_layout
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mesh"),
        bind_group_layouts: &[
            Some(&shared.camera_layout),
            Some(model_layout),
            Some(&shared.material_layout),
            Some(&shared.light_layout),
        ],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if skinned { "skinned mesh" } else { "mesh" }),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.mesh_shader,
            entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &mesh_buffers(skinned),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            front_face: wgpu::FrontFace::Ccw,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shared.mesh_shader,
            // Pipelines targeting the HDR scene are Beautiful's.
            entry_point: Some(if format == HDR_FORMAT {
                "fs_beautiful"
            } else {
                "fs_main"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &if format == HDR_FORMAT {
                coverage_target(format)
            } else {
                blend_target(format)
            },
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Creates the depth-only pipeline that renders meshes into the shadow map.
///
/// Front faces are culled (back faces are rasterized), which pushes
/// shadow-map depth slightly away from the light and reduces acne.
fn create_shadow_pipeline(
    device: &wgpu::Device,
    shared: &PipelineShared,
    skinned: bool,
) -> wgpu::RenderPipeline {
    let model_layout = if skinned {
        &shared.skinned_model_layout
    } else {
        &shared.model_layout
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow"),
        bind_group_layouts: &[Some(&shared.shadow_light_layout), Some(model_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if skinned { "skinned shadow" } else { "shadow" }),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.shadow_shader,
            entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &mesh_buffers(skinned),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Front),
            front_face: wgpu::FrontFace::Ccw,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}

/// Batched sprite vertex, matching the sprite pipeline's buffer layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SpriteVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// CPU mirror of a sprite quad expansion, used to validate the batching math
/// without a GPU (the actual expansion happens in [`crate::Renderer`]).
pub(crate) fn expand_sprite(
    position: glam::Vec2,
    size: glam::Vec2,
    rotation: f32,
    color: [f32; 4],
    uv_window: [f32; 4],
) -> [SpriteVertex; 4] {
    let half = size / 2.0;
    let (sin, cos) = rotation.sin_cos();
    let corners = [
        glam::Vec2::new(-half.x, -half.y),
        glam::Vec2::new(half.x, -half.y),
        glam::Vec2::new(half.x, half.y),
        glam::Vec2::new(-half.x, half.y),
    ];
    let uvs = [
        [uv_window[0], uv_window[1]],
        [uv_window[2], uv_window[1]],
        [uv_window[2], uv_window[3]],
        [uv_window[0], uv_window[3]],
    ];
    let mut out = [SpriteVertex {
        position: [0.0; 2],
        uv: [0.0; 2],
        color,
    }; 4];
    for (i, corner) in corners.iter().enumerate() {
        let rotated = glam::Vec2::new(
            corner.x * cos - corner.y * sin,
            corner.x * sin + corner.y * cos,
        );
        out[i].position = (position + rotated).into();
        out[i].uv = uvs[i];
    }
    out
}

/// Sprite index pattern for `n` quads: each quad is two triangles.
pub(crate) fn sprite_indices(count: usize) -> Vec<u16> {
    debug_assert!(
        count <= SPRITE_VERTEX_CAPACITY / 4,
        "sprite batch exceeds index capacity"
    );
    let mut indices = Vec::with_capacity(count * 6);
    for quad in 0..u16::try_from(count).expect("sprite batch fits u16") {
        let base = quad * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    indices
}

/// Validates a WGSL module with naga (a dev-dependency, so this only exists
/// in test builds).
#[cfg(test)]
pub(crate) fn validate_wgsl(label: &str, source: &str) -> Result<(), String> {
    let _ = label;
    naga::front::wgsl::parse_str(source)
        .map(|_| ())
        .map_err(|e| e.emit_to_string(source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_wgsl_is_valid() {
        validate_wgsl("sprite", SPRITE_SHADER).unwrap();
    }

    #[test]
    fn mesh_wgsl_is_valid() {
        validate_wgsl("mesh", MESH_SHADER).unwrap();
    }

    #[test]
    fn shadow_wgsl_is_valid() {
        validate_wgsl("shadow", SHADOW_SHADER).unwrap();
    }

    #[test]
    fn water_and_post_wgsl_validate() {
        for (source, entries) in [
            (WATER_SHADER, &["vs_main", "fs_main"][..]),
            (
                POST_SHADER,
                &[
                    "vs_fullscreen",
                    "fs_downsample",
                    "fs_upsample",
                    "fs_resolve",
                    "fs_fxaa",
                ][..],
            ),
        ] {
            let module = validate_module(source);
            for name in entries {
                assert!(
                    module.entry_points.iter().any(|e| e.name == *name),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn sprite_vertex_layout_matches_shader_locations() {
        // position (loc 0, 2xf32), uv (loc 1, 2xf32), color (loc 2, 4xf32)
        assert_eq!(std::mem::size_of::<SpriteVertex>(), 32);
        assert_eq!(std::mem::align_of::<SpriteVertex>(), 4);
    }

    #[test]
    fn mesh_vertex_layout_matches_shader_locations() {
        assert_eq!(std::mem::size_of::<Vertex>(), 32);
        // joints (loc 3, 4xu16) + weights (loc 4, 4xf32)
        assert_eq!(std::mem::size_of::<SkinVertex>(), 24);
        assert_eq!(SKIN_ATTRIBUTES[1].offset, 8);
    }

    /// Full naga validation (types, entry points), not just parsing.
    fn validate_module(source: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        module
    }

    #[test]
    fn skinned_entry_points_exist_and_validate() {
        for source in [MESH_SHADER, SHADOW_SHADER] {
            let module = validate_module(source);
            let names: Vec<&str> = module
                .entry_points
                .iter()
                .map(|e| e.name.as_str())
                .collect();
            assert!(names.contains(&"vs_main") && names.contains(&"vs_skinned"));
        }
        // The palette size in WGSL matches the Rust limit.
        let palette = format!("array<mat4x4<f32>, {}>", crate::MAX_JOINTS);
        assert!(MESH_SHADER.contains(&palette) && SHADOW_SHADER.contains(&palette));
    }

    #[test]
    fn sprite_indices_are_ccw() {
        let indices = sprite_indices(2);
        assert_eq!(indices, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn expand_sprite_centers_and_rotates() {
        let verts = expand_sprite(
            glam::Vec2::new(10.0, 20.0),
            glam::Vec2::new(4.0, 2.0),
            0.0,
            [1.0; 4],
            [0.0, 0.0, 1.0, 1.0],
        );
        assert_eq!(verts[0].position, [8.0, 19.0]);
        assert_eq!(verts[2].position, [12.0, 21.0]);
        assert_eq!(verts[0].uv, [0.0, 0.0]);
        assert_eq!(verts[2].uv, [1.0, 1.0]);
    }

    #[test]
    fn expand_sprite_applies_uv_window() {
        let verts = expand_sprite(
            glam::Vec2::ZERO,
            glam::Vec2::ONE,
            0.0,
            [1.0; 4],
            [0.25, 0.25, 0.75, 0.75],
        );
        assert_eq!(verts[0].uv, [0.25, 0.25]);
        assert_eq!(verts[1].uv, [0.75, 0.25]);
        assert_eq!(verts[3].uv, [0.25, 0.75]);
    }
}
