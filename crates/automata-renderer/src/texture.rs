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

    /// Creates a repeating, mipmapped texture from RGBA8 data: for 3D model
    /// surfaces, which tile their UVs and are seen at every distance. The
    /// mip chain is box-filtered on the CPU.
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4`.
    #[must_use]
    pub(crate) fn from_rgba_mipmapped(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
        pixelated: bool,
    ) -> Self {
        assert_eq!(
            rgba.len(),
            (width * height * 4) as usize,
            "RGBA data length must be width * height * 4"
        );
        let levels = 32 - width.max(height).max(1).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("model texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut level: Vec<u8> = rgba.to_vec();
        let (mut w, mut h) = (width, height);
        for mip in 0..levels {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &level,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            if mip + 1 < levels {
                (level, w, h) = downsample(&level, w, h);
            }
        }
        // Pixelated textures (palette atlases, pixel art) keep hard texel
        // edges up close. Anisotropic filtering requires linear filters
        // everywhere, so it's only on for smooth textures.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("model sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: if pixelated {
                wgpu::FilterMode::Nearest
            } else {
                wgpu::FilterMode::Linear
            },
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: if pixelated { 1 } else { 8 },
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

/// Halves an RGBA8 image (2×2 box filter; odd edges repeat the last
/// texel). Returns the new pixels and size.
fn downsample(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0_u8; (nw * nh * 4) as usize];
    let at = |x: u32, y: u32, c: u32| {
        u32::from(src[((y.min(h - 1) * w + x.min(w - 1)) * 4 + c) as usize])
    };
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let (sx, sy) = (x * 2, y * 2);
                let sum =
                    at(sx, sy, c) + at(sx + 1, sy, c) + at(sx, sy + 1, c) + at(sx + 1, sy + 1, c);
                out[((y * nw + x) * 4 + c) as usize] =
                    u8::try_from((sum + 2) / 4).unwrap_or(u8::MAX);
            }
        }
    }
    (out, nw, nh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_averages_and_handles_odd_sizes() {
        // 2x1: black and white average to grey in a 1x1 level.
        let src = [0, 0, 0, 255, 255, 255, 255, 255];
        let (out, w, h) = downsample(&src, 2, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, [128, 128, 128, 255]);
        // 3x3 shrinks to 1x1 without reading out of bounds.
        let (out, w, h) = downsample(&[7; 36], 3, 3);
        assert_eq!((w, h, out), (1, 1, vec![7; 4]));
    }
}
