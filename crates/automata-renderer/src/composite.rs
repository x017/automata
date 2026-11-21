//! GPU compositing of a CPU-drawn overlay over a rendered scene.
//!
//! Editors that draw their UI on the CPU (Oberon) used to read the 3D view
//! back from the GPU, blend it into the UI image and upload the result:
//! three full-frame copies and a stall every frame. The compositor instead
//! uploads only the UI image and lets the GPU show the scene texture
//! through its transparent pixels ([`crate::Renderer::present_composited`]).

use wgpu::util::DeviceExt;

/// WGSL source of the composite pass.
pub(crate) const COMPOSITE_SHADER: &str = include_str!("shaders/composite.wgsl");

/// Composite pipelines (one per target format) and the overlay texture.
pub(crate) struct Compositor {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    overlay: Option<(wgpu::Texture, (u32, u32))>,
    /// Bound in place of the scene when there is none.
    empty_scene: wgpu::Texture,
}

/// The composite pass's uniform (matches `Params` in the shader).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    origin: [i32; 2],
    size: [i32; 2],
    srgb_target: u32,
    has_scene: u32,
    pad: [u32; 2],
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Compositor {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
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
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_SHADER.into()),
        });
        let empty_scene = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composite empty scene"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            layout,
            shader,
            pipelines: Vec::new(),
            overlay: None,
            empty_scene,
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> usize {
        if let Some(i) = self.pipelines.iter().position(|(f, _)| *f == format) {
            return i;
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("composite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.push((format, pipeline));
        self.pipelines.len() - 1
    }

    /// Uploads `overlay` (premultiplied RGBA8, `size`) and composites it
    /// over `scene` (placed at `origin`) into `target`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        overlay: &[u8],
        scene: Option<(&wgpu::Texture, (i32, i32))>,
    ) {
        if self.overlay.as_ref().map(|(_, s)| *s) != Some(size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("composite overlay"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.overlay = Some((texture, size));
        }
        let Some((overlay_texture, _)) = &self.overlay else {
            return;
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: overlay_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            overlay,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.0),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        let overlay_view = overlay_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (scene_texture, origin) = scene.unwrap_or((&self.empty_scene, (0, 0)));
        let scene_view = scene_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let params = Params {
            origin: [origin.0, origin.1],
            size: [
                i32::try_from(scene_texture.width()).unwrap_or(i32::MAX),
                i32::try_from(scene_texture.height()).unwrap_or(i32::MAX),
            ],
            srgb_target: u32::from(format.is_srgb()),
            has_scene: u32::from(scene.is_some()),
            pad: [0; 2],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("composite params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&overlay_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let pipeline = self.pipeline(device, format);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("composite"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..wgpu::RenderPassDescriptor::default()
            });
            pass.set_pipeline(&self.pipelines[pipeline].1);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_wgsl_validates() {
        let module = naga::front::wgsl::parse_str(COMPOSITE_SHADER).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(std::mem::size_of::<Params>(), 32);
    }
}
