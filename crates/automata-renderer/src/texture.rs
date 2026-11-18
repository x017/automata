//! Textures: GPU images sampled by sprites and materials.

use std::sync::Arc;

use crate::error::Result;

/// A texture uploaded to the GPU, with its own sampler.
///
/// Cheap to clone: the underlying GPU resources are reference-counted.
#[derive(Debug, Clone)]
pub struct Texture {
    pub(crate) inner: Arc<TextureInner>,
}

/// GPU-resident texture data.
#[derive(Debug)]
pub(crate) struct TextureInner {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub size: (u32, u32),
}

impl Texture {
    /// The texture size in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.inner.size
    }

    /// Creates a texture from raw RGBA8 data (4 bytes per pixel).
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4`.
    #[must_use]
    pub(crate) fn from_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
        filter: wgpu::FilterMode,
    ) -> Self {
        assert_eq!(
            rgba.len(),
            (width * height * 4) as usize,
            "RGBA data length must be width * height * 4"
        );

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            mag_filter: filter,
            min_filter: filter,
            ..wgpu::SamplerDescriptor::default()
        });

        Self {
            inner: Arc::new(TextureInner {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                sampler,
                size: (width, height),
            }),
        }
    }

    /// Decodes an image file (PNG or JPEG) into a texture.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Image`] if the bytes are not a decodable image.
    pub(crate) fn from_image_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
    ) -> Result<Self> {
        let image = image::load_from_memory(bytes)?.to_rgba8();
        let (width, height) = image.dimensions();
        Ok(Self::from_rgba(
            device,
            queue,
            width,
            height,
            &image,
            wgpu::FilterMode::Linear,
        ))
    }

    /// A stable identity for batching: textures sharing storage batch
    /// together.
    pub(crate) fn batch_id(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }
}
