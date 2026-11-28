//! The custom post effect pass (`shaders/post_api.wgsl` around a user's
//! `post` function, see `shader.rs`).
//!
//! When a frame has a post shader, everything that would reach the target
//! before the HUD (the scene, or the graded / anti-aliased image, and the
//! world's sprites) goes into [`PostShaderPass::input`] instead; the pass
//! then draws the effect over the whole target, and the HUD sprites follow
//! on top, untouched.

use std::sync::Weak;

use wgpu::util::DeviceExt;

use crate::shader::ShaderInner;
use crate::Shader;

/// `PostShaderData` in `post_api.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct PostShaderUniform {
    size: [f32; 4],
    misc: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
}

/// The image the frame renders into for the effect to read.
struct Input {
    format: wgpu::TextureFormat,
    size: (u32, u32),
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

/// The post effect pass: its image, uniform and pipelines.
pub(crate) struct PostShaderPass {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    input: Option<Input>,
    /// Per shader (by id, kept while it lives) and target format.
    pipelines: Vec<(
        u64,
        Weak<ShaderInner>,
        wgpu::TextureFormat,
        wgpu::RenderPipeline,
    )>,
}

impl PostShaderPass {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            layout: crate::pipeline::post_shader_layout(device),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post shader"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..wgpu::SamplerDescriptor::default()
            }),
            uniform: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("post shader"),
                contents: bytemuck::bytes_of(&PostShaderUniform::default()),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }),
            input: None,
            pipelines: Vec::new(),
        }
    }

    /// Gets ready to run `shader` over a `size` frame in `format` this
    /// frame: its pipeline, the input image, the uniform. False when it
    /// can't (the GPU refused the shader, which is then marked broken).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shader: &Shader,
        params: [f32; crate::SHADER_PARAMS],
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        size: (u32, u32),
        time: f32,
    ) -> bool {
        if shader.broken() {
            return false;
        }
        self.pipelines
            .retain(|(_, shader, _, _)| shader.strong_count() > 0);
        let id = shader.id();
        let exists = self
            .pipelines
            .iter()
            .any(|(i, _, f, _)| *i == id && *f == format);
        if !exists {
            let (pipeline, error) = crate::shader::catching(device, || {
                crate::pipeline::create_post_shader_pipeline(
                    device,
                    &self.layout,
                    &shader.inner.module,
                    format,
                    depth_format,
                )
            });
            if let Some(why) = error {
                shader.fail(why);
                return false;
            }
            self.pipelines.push((
                id,
                std::sync::Arc::downgrade(&shader.inner),
                format,
                pipeline,
            ));
        }
        let size = (size.0.max(1), size.1.max(1));
        if !self
            .input
            .as_ref()
            .is_some_and(|i| i.format == format && i.size == size)
        {
            self.input = Some(self.make_input(device, format, size));
        }
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        #[allow(clippy::cast_precision_loss)]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let uniform = PostShaderUniform {
            size: [w, h, 1.0 / w, 1.0 / h],
            misc: [
                finite(time),
                f32::from(u8::from(!format.is_srgb())),
                0.0,
                0.0,
            ],
            p0: [params[0], params[1], params[2], params[3]].map(finite),
            p1: [params[4], params[5], params[6], params[7]].map(finite),
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        true
    }

    fn make_input(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Input {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post shader input"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post shader"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        Input {
            format,
            size,
            view,
            bind,
        }
    }

    /// The image this frame renders into (after [`Self::prepare`]).
    pub fn input(&self) -> Option<&wgpu::TextureView> {
        self.input.as_ref().map(|i| &i.view)
    }

    /// Draws the effect over the pass's target.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, shader: &Shader) {
        let Some(input) = &self.input else { return };
        let id = shader.id();
        let Some((_, _, _, pipeline)) = self
            .pipelines
            .iter()
            .find(|(i, _, f, _)| *i == id && *f == input.format)
        else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &input.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
