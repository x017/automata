//! Model thumbnails for the Assets browser.
//!
//! Thumbnails are drawn on the CPU: a small z-buffered rasterizer with the
//! model's textures and a fixed key light, viewed from the front-right and
//! a little above. Kits of low-poly props render in a millisecond or two at
//! thumbnail size, it works without a GPU (CI, software-only machines), and
//! it never competes with the scene view for the device.
//!
//! [`ThumbCache`] keeps one image per model file and size, and redraws when
//! the file or its import settings change.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use automata_assets::Model;
use glam::camera::rh::{proj::directx, view};
use glam::{Vec2, Vec3, Vec4};

/// An RGBA8 thumbnail: pixels, width, height.
pub type Thumb = (Vec<u8>, u32, u32);

/// The key light's direction (towards the light).
const LIGHT: Vec3 = Vec3::new(0.45, 0.8, 0.55);

/// Renders `model` into a `size`×`size` RGBA8 image with a transparent
/// background, framed to fit.
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn render(model: &Model, size: u32) -> Thumb {
    let size = size.clamp(8, 1024);
    let n = (size * size) as usize;
    let mut px = vec![0u8; n * 4];
    let mut depth = vec![f32::INFINITY; n];
    if !model.min.is_finite() || !model.max.is_finite() {
        return (px, size, size);
    }
    let center = (model.min + model.max) * 0.5;
    let radius = ((model.max - model.min) * 0.5).length().max(1e-4);
    // Front-right, slightly above: the way kits show their props.
    let dir = Vec3::new(0.85, 0.6, 1.0).normalize();
    let fov = 30f32.to_radians();
    let dist = radius / (fov * 0.5).sin();
    let eye = center + dir * dist;
    let view = view::look_at_mat4(eye, center, Vec3::Y);
    let proj = directx::perspective(
        fov,
        1.0,
        (dist - radius * 1.5).max(1e-3),
        dist + radius * 1.5,
    );
    let clip = proj * view;
    let light = LIGHT.normalize();
    let half = size as f32 * 0.5;
    let to_screen = |p: Vec3| -> Option<(Vec2, f32)> {
        let c = clip * Vec4::new(p.x, p.y, p.z, 1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let ndc = c.truncate() / c.w;
        Some((Vec2::new((ndc.x + 1.0) * half, (1.0 - ndc.y) * half), ndc.z))
    };
    for part in &model.parts {
        let verts = &part.mesh.vertices;
        for tri in part.mesh.indices.chunks_exact(3) {
            let Some(v) = [tri[0], tri[1], tri[2]]
                .iter()
                .map(|&i| verts.get(i as usize))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let p: Vec<Vec3> = v.iter().map(|v| Vec3::from(v.position)).collect();
            let Some(s) = p.iter().map(|&p| to_screen(p)).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            // Flat light from the face normal (vertex normals agree for the
            // low-poly models this is for, and it reads crisply when small).
            let normal = face.normalize_or_zero();
            let toward = (eye - p[0]).dot(normal);
            let lit_normal = if toward < 0.0 { -normal } else { normal };
            let shade = 0.42 + 0.58 * lit_normal.dot(light).max(0.0);
            raster_triangle(
                &mut px,
                &mut depth,
                size,
                [s[0], s[1], s[2]],
                [v[0].uv, v[1].uv, v[2].uv],
                |uv| sample(part, uv),
                shade,
            );
        }
    }
    (px, size, size)
}

/// The part's colour at `uv` (texture × base colour), RGBA 0..1.
fn sample(part: &automata_assets::ModelPart, uv: Vec2) -> Vec4 {
    let base = Vec4::from(part.color);
    let Some(tex) = &part.texture else {
        return base;
    };
    if tex.width == 0 || tex.height == 0 {
        return base;
    }
    let u = uv.x - uv.x.floor();
    let v = uv.y - uv.y.floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (x, y) = (
        ((u * tex.width as f32) as u32).min(tex.width - 1),
        ((v * tex.height as f32) as u32).min(tex.height - 1),
    );
    let i = ((y * tex.width + x) * 4) as usize;
    let Some(c) = tex.rgba.get(i..i + 4) else {
        return base;
    };
    // sRGB texels and base colour multiply in display space here; a
    // thumbnail doesn't need linear-light accuracy.
    base * Vec4::new(
        f32::from(c[0]),
        f32::from(c[1]),
        f32::from(c[2]),
        f32::from(c[3]),
    ) / 255.0
}

/// Fills one triangle (screen positions with NDC depth) into `px`,
/// keeping the nearest surface per pixel.
fn raster_triangle(
    px: &mut [u8],
    depth: &mut [f32],
    size: u32,
    s: [(Vec2, f32); 3],
    uv: [[f32; 2]; 3],
    color_at: impl Fn(Vec2) -> Vec4,
    shade: f32,
) {
    let (a, b, c) = (s[0].0, s[1].0, s[2].0);
    let area = (b - a).perp_dot(c - a);
    if area.abs() < 1e-8 {
        return;
    }
    let limit = size as f32 - 1.0;
    let lo = a.min(b).min(c).max(Vec2::ZERO).floor();
    let hi = a.max(b).max(c).min(Vec2::splat(limit)).ceil();
    if lo.x > hi.x || lo.y > hi.y {
        return;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (x0, y0, x1, y1) = (lo.x as u32, lo.y as u32, hi.x as u32, hi.y as u32);
    let uv = uv.map(Vec2::from);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = (b - p).perp_dot(c - p) / area;
            let w1 = (c - p).perp_dot(a - p) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let z = w0 * s[0].1 + w1 * s[1].1 + w2 * s[2].1;
            let i = (y * size + x) as usize;
            if z >= depth[i] {
                continue;
            }
            let col = color_at(uv[0] * w0 + uv[1] * w1 + uv[2] * w2);
            if col.w < 0.5 {
                continue; // cut-out leaves, fences
            }
            depth[i] = z;
            let lit = (col.truncate() * shade).clamp(Vec3::ZERO, Vec3::ONE) * 255.0;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            px[i * 4..i * 4 + 4].copy_from_slice(&[
                lit.x.round() as u8,
                lit.y.round() as u8,
                lit.z.round() as u8,
                255,
            ]);
        }
    }
}

fn stamp(path: &Path) -> (Option<SystemTime>, Option<SystemTime>) {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    (m(path), m(&automata_assets::import::settings_path(path)))
}

type Stamp = (Option<SystemTime>, Option<SystemTime>);

/// Thumbnails by model file and pixel size, redrawn when the file or its
/// import settings change.
#[derive(Default)]
pub struct ThumbCache {
    entries: HashMap<(PathBuf, u32), (Stamp, Option<Thumb>)>,
}

impl ThumbCache {
    /// The cached thumbnail of `path` at `size` pixels, if it is current.
    /// `None` means it still has to be drawn ([`Self::draw`]); `Some(None)`
    /// means the model can't be loaded.
    #[must_use]
    pub fn get(&self, path: &Path, size: u32) -> Option<Option<&Thumb>> {
        let (st, thumb) = self.entries.get(&(path.to_path_buf(), size))?;
        (*st == stamp(path)).then_some(thumb.as_ref())
    }

    /// Draws (or redraws) the thumbnail of `path` at `size` pixels.
    pub fn draw(&mut self, path: &Path, size: u32) {
        let st = stamp(path);
        let thumb = automata_assets::load(&path.to_string_lossy())
            .ok()
            .map(|model| render(&model, size));
        self.entries.insert((path.to_path_buf(), size), (st, thumb));
    }

    /// Forgets every thumbnail.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_assets::ModelPart;
    use automata_renderer::{MeshData, Vertex};

    fn cube(color: [f32; 4]) -> Model {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (n, u, v) in [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ] {
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = (n + u * a + v * b) * 0.5;
                vertices.push(Vertex::new(p.to_array(), n.to_array(), [0.0, 0.0]));
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Model::new(
            PathBuf::from("cube.obj"),
            vec![ModelPart {
                name: "cube".into(),
                mesh: MeshData { vertices, indices },
                color,
                texture: None,
                pixelated: false,
                skin: None,
            }],
        )
    }

    #[test]
    fn a_cube_fills_the_middle_and_leaves_the_corners_clear() {
        let (px, w, h) = render(&cube([1.0, 0.0, 0.0, 1.0]), 64);
        assert_eq!((w, h, px.len()), (64, 64, 64 * 64 * 4));
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        let mid = at(32, 32);
        assert_eq!(mid[3], 255, "the centre is covered");
        assert!(mid[0] > 80 && mid[1] == 0 && mid[2] == 0, "{mid:?}");
        assert_eq!(at(0, 0)[3], 0, "corners stay transparent");
        assert_eq!(at(63, 63)[3], 0);
        // Three faces show with different shading.
        let shades: std::collections::BTreeSet<u8> = px
            .chunks_exact(4)
            .filter(|p| p[3] == 255)
            .map(|p| p[0])
            .collect();
        assert!(shades.len() >= 3, "{shades:?}");
    }

    #[test]
    fn the_cache_redraws_when_the_import_settings_change() {
        let dir = std::env::temp_dir().join(format!("oberon-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tri.obj");
        std::fs::write(&file, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        let mut cache = ThumbCache::default();
        assert!(cache.get(&file, 32).is_none());
        cache.draw(&file, 32);
        assert!(cache.get(&file, 32).is_some_and(|t| t.is_some()));
        let settings = automata_assets::import::ImportSettings {
            rotation: [0.0, 90.0, 0.0],
            ..Default::default()
        };
        automata_assets::import::write_settings(&file, &settings).unwrap();
        assert!(
            cache.get(&file, 32).is_none(),
            "stale after the sidecar appears"
        );
        let missing = dir.join("missing.obj");
        cache.draw(&missing, 32);
        assert_eq!(cache.get(&missing, 32), Some(None));
        std::fs::remove_dir_all(&dir).ok();
    }
}
