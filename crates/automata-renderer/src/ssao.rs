//! Screen-space ambient occlusion and contact shadows for the Beautiful
//! preset (shader: `shaders/ssao.wgsl`).
//!
//! Frame flow: the opaque meshes render depth-only into
//! [`SsaoTargets::depth`] (the scene pass keeps its own depth buffer, so
//! transparent meshes draw exactly as before); one half-resolution pass
//! turns that into ambient occlusion, a contact-shadow term and the view
//! depth; two depth-aware blurs smooth it. The Beautiful mesh shader reads
//! the result ([`Ssao::output_view`]) and upsamples it by depth.

use wgpu::util::DeviceExt;

use crate::pipeline::HDR_DEPTH_FORMAT;
use crate::post::{fullscreen_pass, fullscreen_pipeline};

/// WGSL source of the occlusion passes.
pub(crate) const SSAO_SHADER: &str = include_str!("shaders/ssao.wgsl");

/// The occlusion texture: r ambient occlusion, g contact shadow, b view
/// depth (for the depth-aware blur and upsampling).
pub(crate) const SSAO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// How far around a surface point geometry occludes it (m).
pub(crate) const AO_RADIUS: f32 = 1.5;

/// How far contact shadows look toward the sun (m).
pub(crate) const CONTACT_LENGTH: f32 = 0.45;

/// The occlusion uniform (`SsaoUniform` in ssao.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoUniform {
    view_proj: [f32; 16],
    camera: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    sun: [f32; 4],
    size: [f32; 4],
    params: [f32; 4],
}

/// What a frame's occlusion pass needs to know.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SsaoFrame {
    pub camera: crate::Camera3D,
    /// Direction the sunlight travels.
    pub sun_direction: glam::Vec3,
    /// Compute ambient occlusion.
    pub ambient_occlusion: bool,
    /// Compute contact shadows (the sun casts shadows).
    pub contact_shadows: bool,
}

impl SsaoFrame {
    /// Whether any of it is needed (otherwise the output is cleared to
    /// "unoccluded" and the prepass skipped).
    pub fn active(&self) -> bool {
        self.ambient_occlusion || self.contact_shadows
    }
}

/// Size-dependent textures.
pub(crate) struct SsaoTargets {
    size: (u32, u32),
    /// Depth of the opaque meshes, full resolution.
    pub depth: wgpu::Texture,
    /// Half resolution: the raw result, then (after both blurs) the final.
    out: wgpu::Texture,
    /// Half resolution: between the two blur directions.
    tmp: wgpu::Texture,
}

/// The occlusion passes and their targets.
pub(crate) struct Ssao {
    layout: wgpu::BindGroupLayout,
    ao: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    pub targets: Option<SsaoTargets>,
}

impl Ssao {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
                        // Depth read with textureLoad: unfilterable float, since GLSL
                        // (WebGL2) can't textureLoad a depth texture.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao"),
            source: wgpu::ShaderSource::Wgsl(SSAO_SHADER.into()),
        });
        let ao = fullscreen_pipeline(device, &layout, &shader, "fs_ao", SSAO_FORMAT, None, None);
        let blur =
            fullscreen_pipeline(device, &layout, &shader, "fs_blur", SSAO_FORMAT, None, None);
        Self {
            layout,
            ao,
            blur,
            targets: None,
        }
    }

    /// Makes sure the targets match the viewport `size`.
    pub fn ensure(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        use wgpu::TextureUsages as U;
        let size = (size.0.max(1), size.1.max(1));
        if self.targets.as_ref().map(|t| t.size) == Some(size) {
            return;
        }
        let texture = |label, format, usage, (w, h): (u32, u32)| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let half = half_size(size);
        let rw = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        self.targets = Some(SsaoTargets {
            size,
            depth: texture("ssao depth", HDR_DEPTH_FORMAT, rw, size),
            out: texture("ssao", SSAO_FORMAT, rw, half),
            tmp: texture("ssao blur", SSAO_FORMAT, rw, half),
        });
    }

    /// Frees the targets (when leaving the Beautiful preset).
    pub fn release(&mut self) {
        self.targets = None;
    }

    /// The finished occlusion texture the mesh shader reads.
    pub fn output_view(&self) -> Option<wgpu::TextureView> {
        self.targets
            .as_ref()
            .map(|t| t.out.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// The prepass depth target.
    pub fn depth_view(&self) -> Option<wgpu::TextureView> {
        self.targets
            .as_ref()
            .map(|t| t.depth.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// Records the occlusion pass and both blurs (after the depth
    /// prepass), or when `frame` needs none of it, clears the output to
    /// "unoccluded".
    pub fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &SsaoFrame,
    ) {
        let Some(t) = &self.targets else {
            return;
        };
        let view = |tex: &wgpu::Texture| tex.create_view(&wgpu::TextureViewDescriptor::default());
        let (out, tmp, depth) = (view(&t.out), view(&t.tmp), view(&t.depth));
        if !frame.active() {
            let mut _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssao clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 60_000.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..wgpu::RenderPassDescriptor::default()
            });
            return;
        }
        let base = uniform(frame, t.size);
        let bind = |src: &wgpu::TextureView, step: [f32; 2]| {
            let mut u = base;
            u.size[2] = step[0];
            u.size[3] = step[1];
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ssao"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                ],
            })
        };
        let load = wgpu::LoadOp::Clear(wgpu::Color::WHITE);
        // The source texture isn't read by the occlusion pass; any view
        // other than its target will do.
        fullscreen_pass(
            encoder,
            "ssao",
            &out,
            load,
            &self.ao,
            &bind(&tmp, [0.0, 0.0]),
        );
        fullscreen_pass(
            encoder,
            "ssao blur x",
            &tmp,
            load,
            &self.blur,
            &bind(&out, [1.0, 0.0]),
        );
        fullscreen_pass(
            encoder,
            "ssao blur y",
            &out,
            load,
            &self.blur,
            &bind(&tmp, [0.0, 1.0]),
        );
    }
}

/// Half the viewport, rounded up so every full-resolution pixel pair has
/// a texel.
fn half_size(size: (u32, u32)) -> (u32, u32) {
    (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1))
}

fn uniform(frame: &SsaoFrame, size: (u32, u32)) -> SsaoUniform {
    let cam = frame.camera;
    let aspect = size.0 as f32 / size.1.max(1) as f32;
    let forward = (cam.target - cam.position)
        .try_normalize()
        .unwrap_or(glam::Vec3::NEG_Z);
    let right = forward
        .cross(cam.up)
        .try_normalize()
        .unwrap_or(glam::Vec3::X);
    let up = right.cross(forward);
    let tan = (cam.fov_y_degrees * 0.5).to_radians().tan();
    let toward_sun = -frame.sun_direction.normalize_or_zero();
    let on = |b: bool| f32::from(u8::from(b));
    let p = cam.position;
    let r = right * tan * aspect;
    let u = up * tan;
    SsaoUniform {
        view_proj: cam.view_proj(size).to_cols_array(),
        camera: [p.x, p.y, p.z, cam.near],
        forward: [forward.x, forward.y, forward.z, cam.far],
        right: [r.x, r.y, r.z, AO_RADIUS],
        up: [u.x, u.y, u.z, CONTACT_LENGTH],
        sun: [
            toward_sun.x,
            toward_sun.y,
            toward_sun.z,
            on(frame.contact_shadows),
        ],
        size: [size.0 as f32, size.1 as f32, 0.0, 0.0],
        params: [on(frame.ambient_occlusion), 0.0, 0.0, 0.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_matches_the_shader_layout() {
        assert_eq!(std::mem::size_of::<SsaoUniform>(), 16 * 4 + 7 * 16);
    }

    #[test]
    fn the_shaders_depth_linearization_matches_the_projection() {
        let cam = crate::Camera3D::default();
        let (n, f) = (cam.near, cam.far);
        let forward = (cam.target - cam.position).normalize();
        for vz in [n * 2.0, 1.0, 7.5, 40.0, f * 0.5] {
            let clip = cam.view_proj((16, 9)) * (cam.position + forward * vz).extend(1.0);
            let d = clip.z / clip.w;
            // linear_depth() in ssao.wgsl.
            let back = n * f / (f - d * (f - n));
            assert!((back - vz).abs() < vz * 1e-3, "{vz} -> {d} -> {back}");
            assert!((clip.w - vz).abs() < 1e-3);
        }
    }

    #[test]
    fn half_size_rounds_up() {
        assert_eq!(half_size((1280, 720)), (640, 360));
        assert_eq!(half_size((5, 3)), (3, 2));
        assert_eq!(half_size((1, 1)), (1, 1));
    }

    #[test]
    fn the_view_rays_reach_the_screen_corners() {
        let frame = SsaoFrame {
            camera: crate::Camera3D::default(),
            sun_direction: glam::Vec3::NEG_Y,
            ambient_occlusion: true,
            contact_shadows: false,
        };
        let u = uniform(&frame, (200, 100));
        let v = |a: [f32; 4]| glam::Vec3::new(a[0], a[1], a[2]);
        // The top-right corner's ray, pushed to depth 10, projects there.
        let corner = v(u.camera) + (v(u.forward) + v(u.right) + v(u.up)) * 10.0;
        let clip = frame.camera.view_proj((200, 100)) * corner.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            (ndc.x - 1.0).abs() < 1e-4 && (ndc.y - 1.0).abs() < 1e-4,
            "{ndc}"
        );
        assert!((clip.w - 10.0).abs() < 1e-3);
        assert_eq!(u.sun[1], 1.0);
        assert_eq!(u.sun[3], 0.0);
    }
}
