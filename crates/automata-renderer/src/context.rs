//! GPU device, surface, and frame presentation.

use tracing::{debug, info, warn};

use crate::error::{Error, Result};

/// Owns the GPU device, queue, and window surface.
///
/// `RenderContext` is created once per window and drives frame presentation.
/// Phase 2 renders a plain clear color; Phase 3 adds geometry, materials,
/// and cameras on top of the same device and surface.
///
/// Transient frame-acquisition outcomes (timeout, occlusion, outdated or
/// lost surfaces) are handled internally by skipping or reconfiguring; only
/// validation failures are reported as errors.
#[derive(Debug)]
pub struct RenderContext {
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: Option<wgpu::SurfaceConfiguration>,
    size: (u32, u32),
    /// The target format (surface format, or the offscreen format).
    format: wgpu::TextureFormat,
}

impl RenderContext {
    /// Creates a render context for a window surface.
    ///
    /// `surface_target` is anything wgpu can render to — in practice an
    /// `Arc<winit::window::Window>`, which keeps the surface alive
    /// independently of the caller. `size` is the surface's initial physical
    /// size in pixels. `vsync` selects between [`wgpu::PresentMode::AutoVsync`]
    /// and [`wgpu::PresentMode::AutoNoVsync`].
    ///
    /// This is an `async` function because GPU device discovery is
    /// asynchronous; the engine blocks on it during startup.
    ///
    /// # Errors
    ///
    /// - [`Error::CreateSurface`] — the surface could not be created.
    /// - [`Error::RequestAdapter`] — no suitable GPU adapter was found.
    /// - [`Error::RequestDevice`] — the adapter rejected the device request.
    /// - [`Error::IncompatibleSurface`] — the adapter cannot present to the
    ///   surface.
    pub async fn new(
        surface_target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: (u32, u32),
        vsync: bool,
    ) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(surface_target.into())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await?;
        info!(
            adapter = adapter.get_info().name.as_str(),
            backend = ?adapter.get_info().backend,
            "GPU adapter selected"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("automata-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        let (width, height) = (size.0.max(1), size.1.max(1));
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or(Error::IncompatibleSurface)?;
        config.present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        // Allow CPU-composited UIs (Oberon) to upload a finished frame to the
        // swapchain with `Queue::write_texture` in addition to normal rendering.
        config.usage |= wgpu::TextureUsages::COPY_DST;
        surface.configure(&device, &config);
        info!(
            format = ?config.format,
            present_mode = ?config.present_mode,
            width,
            height,
            "surface configured"
        );

        Ok(Self {
            format: config.format,
            surface: Some(surface),
            device,
            queue,
            config: Some(config),
            size: (width, height),
        })
    }

    /// Creates a render context without a window surface.
    ///
    /// Used by tests and headless tooling; frames are rendered into a
    /// texture supplied by the caller via offscreen rendering.
    ///
    /// # Errors
    ///
    /// Returns an error if no suitable GPU adapter or device is available.
    pub async fn new_offscreen(size: (u32, u32)) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                force_fallback_adapter: true, // accept software adapters (llvmpipe/lavapipe)
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await?;
        info!(
            adapter = adapter.get_info().name.as_str(),
            backend = ?adapter.get_info().backend,
            "offscreen GPU adapter selected"
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("automata-offscreen"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        let (width, height) = (size.0.max(1), size.1.max(1));
        Ok(Self {
            surface: None,
            device,
            queue,
            config: None,
            size: (width, height),
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
        })
    }

    /// The GPU device.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The GPU command queue.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The surface's current size in physical pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The target texture format (the surface format when windowed).
    #[must_use]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Acquires the next frame's surface texture.
    ///
    /// Transient acquisition outcomes are handled internally; `Ok(None)`
    /// means this frame should be skipped (occluded/outdated surface, or
    /// the context is surfaceless).
    ///
    /// # Errors
    ///
    /// Returns [`Error::SurfaceValidation`] on a validation failure.
    pub(crate) fn acquire_frame(&mut self) -> Result<Option<wgpu::SurfaceTexture>> {
        let Some(surface) = &self.surface else {
            return Ok(None);
        };
        let texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                warn!("surface suboptimal; reconfiguring");
                self.reconfigure();
                t
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure();
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err(Error::SurfaceValidation),
        };
        Ok(Some(texture))
    }

    /// Presents an acquired frame's surface texture.
    pub(crate) fn present(&mut self, texture: wgpu::SurfaceTexture) {
        self.queue.present(texture);
    }

    /// Reconfigures the surface (helper for acquire/resize paths).
    fn reconfigure(&mut self) {
        if let (Some(surface), Some(config)) = (&self.surface, &self.config) {
            surface.configure(&self.device, config);
        }
    }

    /// Resizes the surface to match the window.
    ///
    /// Zero-sized windows (e.g. minimized) are ignored: a swapchain cannot
    /// have zero extent, and frames are skipped while minimized anyway.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == self.size {
            return;
        }
        self.size = (width, height);
        if let Some(config) = &mut self.config {
            config.width = width;
            config.height = height;
        }
        self.reconfigure();
        debug!(width, height, "surface resized");
    }

    /// Acquires the next frame, clears it to `color`, and presents it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SurfaceValidation`] if frame acquisition raised a
    /// validation error. Transient outcomes (timeout, occlusion, outdated or
    /// lost surfaces) are handled internally and never reported.
    pub fn render_clear(&mut self, color: wgpu::Color) -> Result<()> {
        let Some(frame) = self.acquire_frame()? else {
            return Ok(());
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("automata-clear"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("automata-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..wgpu::RenderPassDescriptor::default()
            });
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Device creation works wherever a GPU or software adapter (e.g.
    /// llvmpipe on CI) exists; the test skips gracefully otherwise, since
    /// sandboxes and some CI machines have no graphics driver at all.
    #[test]
    fn device_creation_where_supported() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let options = wgpu::RequestAdapterOptions::<'_, '_> {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: true, // accept software adapters too
            compatible_surface: None,
            apply_limit_buckets: false,
        };
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&options)) else {
            eprintln!("no GPU adapter available; skipping device test");
            return;
        };

        let result = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("automata-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }));

        let (_device, _queue) = result.expect("device creation failed on an available adapter");
        eprintln!("created device on adapter: {}", adapter.get_info().name);
    }
}
