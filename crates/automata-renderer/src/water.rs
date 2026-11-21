//! Water surfaces: plain data a scene stores, plus the GPU-side uniform
//! and grid the water shader draws.
//!
//! A [`Water`] is a rectangle on the entity's local XZ plane, `size` wide,
//! animated by Gerstner waves. Every preset draws it; the
//! [`GraphicsQuality::Beautiful`](crate::GraphicsQuality::Beautiful)
//! preset adds refraction, depth absorption and shoreline foam.

use glam::Mat4;

use crate::mesh::{MeshData, Vertex};

/// Grid cells along each side of the water mesh (vertex wave resolution;
/// lighting detail is per pixel and doesn't depend on it).
pub(crate) const GRID_CELLS: u32 = 128;

/// A water surface.
///
/// Plain data so scene files can store it (`water: Some((size: (40.0,
/// 40.0), wave_height: 0.3))`); missing fields fall back to
/// [`Water::default`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Water {
    /// Width and depth of the surface (local X and Z, metres).
    pub size: [f32; 2],
    /// Color of deep water (linear RGB).
    pub deep_color: [f32; 3],
    /// Tint of shallow water and of light through wave crests.
    pub shallow_color: [f32; 3],
    /// How far you can see into the water (metres; Beautiful only).
    pub clarity: f32,
    /// Wave height, crest to rest (metres).
    pub wave_height: f32,
    /// Length of the main wave (metres); shorter waves follow it.
    pub wave_length: f32,
    /// Animation speed multiplier (1 = physically paced waves).
    pub wave_speed: f32,
    /// Direction the waves travel, degrees around +Y (0 = toward +X).
    pub wave_direction: f32,
    /// Shoreline foam amount, 0..=1 (Beautiful only).
    pub foam: f32,
}

impl Default for Water {
    fn default() -> Self {
        Self {
            size: [20.0, 20.0],
            deep_color: [0.01, 0.07, 0.11],
            shallow_color: [0.1, 0.42, 0.44],
            clarity: 2.5,
            wave_height: 0.12,
            wave_length: 5.0,
            wave_speed: 1.0,
            wave_direction: 30.0,
            foam: 0.6,
        }
    }
}

impl Water {
    /// A calm pond: low, short waves and clear water.
    #[must_use]
    pub fn pond() -> Self {
        Self {
            wave_height: 0.04,
            wave_length: 2.5,
            wave_speed: 0.7,
            clarity: 4.0,
            ..Self::default()
        }
    }

    /// Open sea: tall, long waves and deep blue water.
    #[must_use]
    pub fn ocean() -> Self {
        Self {
            size: [200.0, 200.0],
            deep_color: [0.005, 0.04, 0.09],
            shallow_color: [0.06, 0.32, 0.4],
            clarity: 6.0,
            wave_height: 0.6,
            wave_length: 14.0,
            wave_speed: 1.0,
            ..Self::default()
        }
    }
}

/// One water draw: the surface and its entity's transform.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WaterDraw {
    pub water: Water,
    pub model: Mat4,
}

/// Wave `i` relative to the main wave: direction offset (radians),
/// wavelength and amplitude (`WAVE_*` used to live in water.wgsl; the
/// per-wave constants are now folded on the CPU, once per draw).
const WAVE_ANGLE: [f32; WAVES] = [0.0, 0.61, -0.83, 1.71];
const WAVE_LENGTH: [f32; WAVES] = [1.0, 0.57, 0.33, 0.21];
const WAVE_AMP: [f32; WAVES] = [0.5, 0.25, 0.15, 0.1];
/// Gerstner waves summed per vertex and per pixel.
pub(crate) const WAVES: usize = 4;

/// The water shader's per-draw uniform (`WaterUniform` in water.wgsl).
///
/// Everything per wave that doesn't vary per pixel (wave vector, phase at
/// this time, amplitudes after the grid's level of detail, crest
/// steepness) is precomputed here so the shader's wave loop is a dot
/// product, a `sin` and a `cos`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct WaterUniform {
    pub model: [f32; 16],
    pub deep: [f32; 4],
    pub shallow: [f32; 4],
    /// x: wave height, y: refraction on, z: time (s), w: unused.
    pub params: [f32; 4],
    /// Per wave — xy: direction × wavenumber k, z: phase ωt, w: k.
    pub wave_k: [[f32; 4]; WAVES],
    /// Per wave — x/y: amplitude and steepness × amplitude on the vertex
    /// grid (short waves faded out), z/w: the same per pixel (no fade).
    pub wave_amp: [[f32; 4]; WAVES],
}

impl WaterUniform {
    pub fn new(draw: &WaterDraw, refraction: bool, time: f32) -> Self {
        let w = &draw.water;
        let size = glam::Vec2::from(w.size).max(glam::Vec2::splat(0.01));
        // The unit grid scaled to the surface's size, then the entity.
        let model = draw.model * Mat4::from_scale(glam::Vec3::new(size.x, 1.0, size.y));
        let (scale, _, _) = draw.model.to_scale_rotation_translation();
        let cell = (size.x * scale.x).max(size.y * scale.z) / GRID_CELLS as f32;
        // The grid can't represent waves shorter than ~4 cells: fade them
        // out of the displacement (the per-pixel normal keeps them).
        let min_length = cell * 4.0;
        let height = w.wave_height.max(0.0);
        let t = time * w.wave_speed;
        let mut wave_k = [[0.0; 4]; WAVES];
        let mut wave_amp = [[0.0; 4]; WAVES];
        for i in 0..WAVES {
            let wavelength = (w.wave_length.max(0.05) * WAVE_LENGTH[i]).max(0.05);
            let lod = smoothstep(min_length, min_length * 2.0, wavelength);
            let angle = w.wave_direction.to_radians() + WAVE_ANGLE[i];
            let k = std::f32::consts::TAU / wavelength;
            let omega = (9.81 * k).sqrt();
            // Wrap the phase so large times keep f32 precision.
            let phase = (omega * t).rem_euclid(std::f32::consts::TAU);
            wave_k[i] = [angle.cos() * k, angle.sin() * k, phase, k];
            // Steepness: sharp crests without loops.
            let steep = |amp: f32| {
                if amp > 0.0 {
                    (0.7 / (k * amp * 4.0)).min(1.0) * amp
                } else {
                    0.0
                }
            };
            let full = height * WAVE_AMP[i];
            let grid = full * lod;
            wave_amp[i] = [grid, steep(grid), full, steep(full)];
        }
        let [dr, dg, db] = w.deep_color;
        let [sr, sg, sb] = w.shallow_color;
        Self {
            model: model.to_cols_array(),
            deep: [dr, dg, db, w.clarity.max(0.01)],
            shallow: [sr, sg, sb, w.foam.clamp(0.0, 1.0)],
            params: [height, f32::from(u8::from(refraction)), time, 0.0],
            wave_k,
            wave_amp,
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Side of the baked ripple texture (texels).
pub(crate) const RIPPLE_SIZE: u32 = 128;

/// Bakes the tiling ripple texture the water shader samples instead of
/// evaluating noise per pixel: three octaves of periodic value noise.
/// RG holds the height's gradient (signed, `0.5` = flat, scaled so the
/// steepest texel is ±1), B the height (0..1, for foam), A is 1.
// Lattice coordinates and texel values are small and non-negative.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn ripple_texels() -> Vec<u8> {
    let n = RIPPLE_SIZE as usize;
    // (lattice cells per tile, amplitude)
    let octaves: [(u32, f32); 3] = [(8, 1.0), (18, 0.45), (38, 0.18)];
    let hash = |x: u32, y: u32, seed: u32| {
        let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed;
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h & 0xffff) as f32 / 65535.0
    };
    let mut height = vec![0.0_f32; n * n];
    for (octave, &(cells, amp)) in (1_u32..).zip(&octaves) {
        let seed = octave.wrapping_mul(0x9e37_79b9);
        for y in 0..n {
            for x in 0..n {
                let fx = x as f32 * cells as f32 / n as f32;
                let fy = y as f32 * cells as f32 / n as f32;
                let (ix, iy) = (fx.floor() as u32, fy.floor() as u32);
                let (tx, ty) = (fx.fract(), fy.fract());
                let (ux, uy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
                // Wrap the lattice so the tile repeats seamlessly.
                let at = |dx: u32, dy: u32| hash((ix + dx) % cells, (iy + dy) % cells, seed);
                let a = at(0, 0) + (at(1, 0) - at(0, 0)) * ux;
                let b = at(0, 1) + (at(1, 1) - at(0, 1)) * ux;
                height[y * n + x] += (a + (b - a) * uy) * amp;
            }
        }
    }
    let (lo, hi) = height
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
    let h_at = |x: usize, y: usize| height[(y % n) * n + (x % n)];
    let mut grad = vec![[0.0_f32; 2]; n * n];
    let mut steepest = 1e-6_f32;
    for y in 0..n {
        for x in 0..n {
            let gx = h_at(x + 1, y) - h_at(x + n - 1, y);
            let gy = h_at(x, y + 1) - h_at(x, y + n - 1);
            steepest = steepest.max(gx.abs()).max(gy.abs());
            grad[y * n + x] = [gx, gy];
        }
    }
    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut out = Vec::with_capacity(n * n * 4);
    for (g, h) in grad.iter().zip(&height) {
        out.extend_from_slice(&[
            unorm(g[0] / steepest * 0.5 + 0.5),
            unorm(g[1] / steepest * 0.5 + 0.5),
            unorm((h - lo) / (hi - lo).max(1e-6)),
            255,
        ]);
    }
    out
}

/// The unit water grid: `GRID_CELLS`² quads over `[-0.5, 0.5]` on XZ,
/// facing +Y.
pub(crate) fn grid_mesh() -> MeshData {
    let n = GRID_CELLS;
    let mut vertices = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    for z in 0..=n {
        for x in 0..=n {
            let (u, v) = (x as f32 / n as f32, z as f32 / n as f32);
            vertices.push(Vertex::new(
                [u - 0.5, 0.0, v - 0.5],
                [0.0, 1.0, 0.0],
                [u, v],
            ));
        }
    }
    let mut indices = Vec::with_capacity((n * n * 6) as usize);
    let row = n + 1;
    for z in 0..n {
        for x in 0..n {
            let i = z * row + x;
            // Counter-clockwise seen from above (+Y).
            indices.extend_from_slice(&[i, i + row, i + 1, i + 1, i + row, i + row + 1]);
        }
    }
    MeshData::new(vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_covers_the_unit_square_facing_up() {
        let grid = grid_mesh();
        assert_eq!(
            grid.vertices.len(),
            ((GRID_CELLS + 1) * (GRID_CELLS + 1)) as usize
        );
        assert_eq!(grid.indices.len(), (GRID_CELLS * GRID_CELLS * 6) as usize);
        let corner = |i: u32| glam::Vec3::from(grid.vertices[i as usize].position);
        // First triangle winds counter-clockwise seen from +Y.
        let [v0, v1, v2] = [0, 1, 2].map(|k| corner(grid.indices[k]));
        assert!((v1 - v0).cross(v2 - v0).y > 0.0);
        let xs: Vec<f32> = grid.vertices.iter().map(|v| v.position[0]).collect();
        assert_eq!(xs.iter().copied().fold(f32::MAX, f32::min), -0.5);
        assert_eq!(xs.iter().copied().fold(f32::MIN, f32::max), 0.5);
    }

    #[test]
    fn uniform_scales_the_grid_and_packs_waves() {
        let draw = WaterDraw {
            water: Water {
                size: [40.0, 10.0],
                wave_direction: 90.0,
                ..Water::default()
            },
            model: Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0)),
        };
        let u = WaterUniform::new(&draw, true, 4.5);
        let m = Mat4::from_cols_array(&u.model);
        let corner = m.transform_point3(glam::Vec3::new(0.5, 0.0, 0.5));
        assert!((corner - glam::Vec3::new(21.0, 2.0, 8.0)).length() < 1e-4);
        // The main wave travels along +Z (90°); its wavenumber is 2π/λ.
        let k = std::f32::consts::TAU / Water::default().wave_length;
        assert!(u.wave_k[0][0].abs() < 1e-4);
        assert!((u.wave_k[0][1] - k).abs() < 1e-4);
        assert!((u.wave_k[0][3] - k).abs() < 1e-6);
        assert_eq!(u.params[1], 1.0);
        assert_eq!(u.params[2], 4.5);
        assert_eq!(std::mem::size_of::<WaterUniform>() % 16, 0);
    }

    #[test]
    fn short_waves_fade_from_the_grid_but_not_the_pixels() {
        let draw = WaterDraw {
            water: Water {
                size: [400.0, 400.0], // 3.1 m cells: the 1 m waves can't fit
                ..Water::default()
            },
            model: Mat4::IDENTITY,
        };
        let u = WaterUniform::new(&draw, false, 0.0);
        let shortest = u.wave_amp[WAVES - 1];
        assert_eq!(shortest[0], 0.0, "faded from the vertex grid");
        assert!(shortest[2] > 0.0, "kept in the per-pixel normal");
    }

    #[test]
    fn ripple_texture_tiles_and_spans_its_range() {
        let t = ripple_texels();
        let n = RIPPLE_SIZE as usize;
        assert_eq!(t.len(), n * n * 4);
        let channel = |c: usize| t.iter().skip(c).step_by(4).copied();
        // Height spans 0..=255; slopes are centred on 0.5 and the steepest
        // one (on either axis) reaches the end of the range.
        assert_eq!(channel(2).min(), Some(0));
        assert_eq!(channel(2).max(), Some(255));
        let extreme = channel(0)
            .chain(channel(1))
            .map(|v| (i32::from(v) - 128).abs())
            .max();
        assert!(extreme >= Some(126), "steepest slope {extreme:?}");
        for c in 0..2 {
            let mean = channel(c).map(f64::from).sum::<f64>() / (n * n) as f64;
            assert!((mean - 127.5).abs() < 3.0, "channel {c} mean {mean}");
        }
        // Seamless: the last column continues into the first (no jump
        // larger than within the tile).
        let h = |x: usize, y: usize| i32::from(t[(y * n + x) * 4 + 2]);
        let inner = (0..n).map(|y| (h(1, y) - h(0, y)).abs()).max().unwrap();
        let seam = (0..n).map(|y| (h(0, y) - h(n - 1, y)).abs()).max().unwrap();
        assert!(seam <= inner * 2 + 2, "seam {seam} vs inner {inner}");
    }

    #[test]
    fn partial_scene_data_falls_back_to_defaults() {
        let w: Water = ron::from_str("(wave_height: 0.5)").unwrap();
        assert_eq!(w.wave_height, 0.5);
        assert_eq!(w.size, Water::default().size);
    }
}
