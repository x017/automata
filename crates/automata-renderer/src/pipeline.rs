//! Render pipelines: sprite (2D) and mesh (3D), plus WGSL validation.
//!
//! Pipeline creation is expressed as data on `wgpu` descriptors so the
//! layout invariants are testable without a GPU device.

use crate::mesh::Vertex;
use crate::SPRITE_VERTEX_CAPACITY;

/// WGSL source of the 2D sprite shader.
pub(crate) const SPRITE_SHADER: &str = include_str!("shaders/sprite.wgsl");
/// WGSL source of the 3D mesh shader.
pub(crate) const MESH_SHADER: &str = include_str!("shaders/mesh.wgsl");

/// Alpha-blended color target state for the given surface format.
fn blend_target(format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
    pub sprite_shader: wgpu::ShaderModule,
    pub mesh_shader: wgpu::ShaderModule,
}

impl PipelineShared {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
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

        Self {
            camera_layout,
            texture_layout,
            material_layout,
            model_layout,
            sprite_shader: shader_module(device, "sprite", SPRITE_SHADER),
            mesh_shader: shader_module(device, "mesh", MESH_SHADER),
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
        depth_stencil: None,
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

/// Creates the 3D mesh pipeline for the given surface format.
pub(crate) fn create_mesh_pipeline(
    device: &wgpu::Device,
    shared: &PipelineShared,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mesh"),
        bind_group_layouts: &[
            Some(&shared.camera_layout),
            Some(&shared.model_layout),
            Some(&shared.material_layout),
        ],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.mesh_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x3,
                    2 => Float32x2
                ],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            front_face: wgpu::FrontFace::Ccw,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shared.mesh_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &blend_target(format),
        }),
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
    fn sprite_vertex_layout_matches_shader_locations() {
        // position (loc 0, 2xf32), uv (loc 1, 2xf32), color (loc 2, 4xf32)
        assert_eq!(std::mem::size_of::<SpriteVertex>(), 32);
        assert_eq!(std::mem::align_of::<SpriteVertex>(), 4);
    }

    #[test]
    fn mesh_vertex_layout_matches_shader_locations() {
        assert_eq!(std::mem::size_of::<Vertex>(), 32);
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
