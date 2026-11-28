//! Terrain: a heightmap ground, sculpted and painted with brushes.
//!
//! A [`Terrain`] component is a square grid of heights centred on its
//! entity (`size` metres a side, `resolution` samples a side) and a splat
//! map: per texel, how much of each of the four [`TerrainLayer`]s shows
//! (grass, rock, sand and dirt to begin with). Both are saved inside the
//! scene file, as base64 PNGs (16-bit for the heights), so a terrain
//! copies, pastes, undoes and plays like any other component.
//!
//! The brushes ([`Terrain::sculpt`] with a [`Sculpt`] tool, and
//! [`Terrain::paint`]) work in the entity's space. [`Terrain::auto_paint`]
//! paints by height and slope: sand along the water, rock on cliffs.
//!
//! For drawing, the grid is cut into 32 × 32 quad chunks; each becomes an
//! ordinary mesh (so it casts shadows, gets culled and lit like any other),
//! with a material whose shader blends the four layers by the splat map.
//! Chunks are cached on the GPU by their content, so a brush stroke only
//! re-uploads the chunks it touched. Physics gets a heightfield collider
//! (in `automata-physics`), and grass and scattered models stand on it.

// Grid maths: i/j index samples along X/Z, x/z are positions, u/v and
// w/h image coordinates, as usual.
#![allow(clippy::many_single_char_names)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use automata_renderer::{DrawList, Material, Mesh, MeshData, Renderer, Texture, Vertex};
use glam::{Mat3, Mat4, Vec2, Vec3};
use hecs::Entity;
use serde::{Deserialize, Serialize};

use crate::{GlobalTransform, SceneWorld};

/// How many layers a terrain blends.
pub const TERRAIN_LAYERS: usize = 4;
/// The fewest height samples a side.
pub const MIN_TERRAIN_RESOLUTION: u32 = 3;
/// The most height samples a side.
pub const MAX_TERRAIN_RESOLUTION: u32 = 1025;
/// The largest terrain side (metres).
pub const MAX_TERRAIN_SIZE: f32 = 4096.0;
/// The largest splat map side (texels).
const MAX_SPLAT: u32 = 2048;
/// Quads per chunk side.
const CHUNK: u32 = 32;
/// Atlas cell side (pixels) and the wrapped gutter round each layer.
const CELL: u32 = 512;
const GUTTER: u32 = 32;

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// One of a terrain's four ground materials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerrainLayer {
    /// Its colour (linear RGB 0..1), multiplied with the texture.
    pub color: [f32; 3],
    /// An image file to tile across the ground (relative to the asset
    /// root); empty for a plain colour with a little grain.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub texture: String,
    /// How wide one repeat of the texture is (metres).
    pub tile: f32,
}

impl Default for TerrainLayer {
    fn default() -> Self {
        Self {
            color: [0.5, 0.5, 0.5],
            texture: String::new(),
            tile: 4.0,
        }
    }
}

impl TerrainLayer {
    /// A plain-coloured layer repeating every `tile` metres.
    #[must_use]
    pub fn color(color: [f32; 3], tile: f32) -> Self {
        Self {
            color,
            texture: String::new(),
            tile,
        }
    }
}

/// The four layers a new terrain starts with: grass, rock, sand, dirt.
#[must_use]
pub fn default_layers() -> [TerrainLayer; TERRAIN_LAYERS] {
    [
        TerrainLayer::color([0.20, 0.42, 0.12], 3.0),
        TerrainLayer::color([0.36, 0.34, 0.31], 5.0),
        TerrainLayer::color([0.78, 0.68, 0.46], 3.0),
        TerrainLayer::color([0.34, 0.24, 0.15], 3.0),
    ]
}

/// Which layer is which in [`default_layers`] (and [`Terrain::auto_paint`]).
pub mod layer {
    /// Grass: the default everywhere.
    pub const GRASS: usize = 0;
    /// Rock: cliffs.
    pub const ROCK: usize = 1;
    /// Sand: beaches.
    pub const SAND: usize = 2;
    /// Dirt: slopes between grass and rock.
    pub const DIRT: usize = 3;
}

/// A sculpting tool (see [`Terrain::sculpt`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sculpt {
    /// Pushes the ground up by `amount` metres at the centre.
    Raise,
    /// Pushes it down.
    Lower,
    /// Evens out bumps: `amount` (0..1) of the way to the neighbours'
    /// average at the centre.
    Smooth,
    /// Moves the ground `amount` (0..1) of the way to this height (in the
    /// entity's space).
    Flatten(f32),
    /// Roughens it with noise up to `amount` metres.
    Noise,
}

/// A heightmap ground: a scene component (saved with the scene).
///
/// The grid is `size` × `size` metres in the entity's X and Z, centred on
/// it, with `resolution` × `resolution` height samples (metres, in the
/// entity's Y). The splat map says how much of each layer shows where.
#[derive(Clone, Serialize, Deserialize)]
#[serde(from = "TerrainFile", into = "TerrainFile")]
pub struct Terrain {
    /// The side of the square (metres).
    pub size: f32,
    /// The four ground layers, blended by the splat map.
    pub layers: [TerrainLayer; TERRAIN_LAYERS],
    /// Surface roughness for lighting (0 shiny .. 1 matte).
    pub roughness: f32,
    /// Whether it's solid (a heightfield collider in physics).
    pub collide: bool,
    resolution: u32,
    heights: Arc<Vec<f32>>,
    splat: Arc<Vec<[u8; 4]>>,
    revision: u64,
    splat_revision: u64,
    /// GPU meshes and material (a cache, shared by clones).
    gpu: GpuCache,
}

impl std::fmt::Debug for Terrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terrain")
            .field("size", &self.size)
            .field("resolution", &self.resolution)
            .field("layers", &self.layers)
            .field("roughness", &self.roughness)
            .field("collide", &self.collide)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Terrain {
    fn eq(&self, other: &Self) -> bool {
        self.size.to_bits() == other.size.to_bits()
            && self.resolution == other.resolution
            && self.layers == other.layers
            && self.roughness.to_bits() == other.roughness.to_bits()
            && self.collide == other.collide
            && (Arc::ptr_eq(&self.heights, &other.heights) || self.heights == other.heights)
            && (Arc::ptr_eq(&self.splat, &other.splat) || self.splat == other.splat)
    }
}

impl Default for Terrain {
    fn default() -> Self {
        Self::new(64.0, 129)
    }
}

fn splat_side(resolution: u32) -> u32 {
    ((resolution - 1) * 2).clamp(2, MAX_SPLAT)
}

fn clamp_resolution(resolution: u32) -> u32 {
    resolution.clamp(MIN_TERRAIN_RESOLUTION, MAX_TERRAIN_RESOLUTION)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A brush's weight at `d` metres from its centre: 1 in the middle,
/// easing smoothly to 0 at `radius`.
fn falloff(d: f32, radius: f32) -> f32 {
    if d >= radius {
        return 0.0;
    }
    let t = 1.0 - (d / radius) * (d / radius);
    t * t
}

impl Terrain {
    /// A flat terrain `size` metres a side with `resolution` height
    /// samples a side (clamped to 3..=1025), all grass.
    #[must_use]
    pub fn new(size: f32, resolution: u32) -> Self {
        let resolution = clamp_resolution(resolution);
        let side = splat_side(resolution);
        Self {
            size: size.clamp(1.0, MAX_TERRAIN_SIZE),
            layers: default_layers(),
            roughness: 0.9,
            collide: true,
            resolution,
            heights: Arc::new(vec![0.0; (resolution * resolution) as usize]),
            splat: Arc::new(vec![[255, 0, 0, 0]; (side * side) as usize]),
            revision: next_revision(),
            splat_revision: next_revision(),
            gpu: GpuCache::default(),
        }
    }

    /// Height samples a side.
    #[must_use]
    pub fn resolution(&self) -> u32 {
        self.resolution
    }

    /// The distance between neighbouring samples (metres).
    #[must_use]
    pub fn spacing(&self) -> f32 {
        self.size / (self.resolution - 1) as f32
    }

    /// The heights, row by row along Z, each row along X (metres).
    #[must_use]
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// The splat map's side (texels).
    #[must_use]
    pub fn splat_resolution(&self) -> u32 {
        splat_side(self.resolution)
    }

    /// The splat map, row by row along Z: each texel's four layer weights
    /// (0..255, summing to 255).
    #[must_use]
    pub fn splat(&self) -> &[[u8; 4]] {
        &self.splat
    }

    /// Changes whenever the heights change (equal for clones).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Changes whenever the splat map changes.
    #[must_use]
    pub fn splat_revision(&self) -> u64 {
        self.splat_revision
    }

    /// Replaces every height (`resolution²` of them, row by row along Z).
    ///
    /// # Errors
    ///
    /// Returns a message when the count is wrong or a height isn't finite.
    pub fn set_heights(&mut self, heights: Vec<f32>) -> Result<(), String> {
        let want = (self.resolution * self.resolution) as usize;
        if heights.len() != want {
            return Err(format!(
                "terrain: {} heights for a {r}×{r} grid (needs {want})",
                heights.len(),
                r = self.resolution
            ));
        }
        if heights.iter().any(|h| !h.is_finite()) {
            return Err("terrain: heights must be finite numbers".into());
        }
        self.heights = Arc::new(heights);
        self.revision = next_revision();
        Ok(())
    }

    /// Replaces the splat map (`splat_resolution²` texels); each texel's
    /// weights are rescaled to sum to 255.
    ///
    /// # Errors
    ///
    /// Returns a message when the count is wrong.
    pub fn set_splat(&mut self, splat: Vec<[u8; 4]>) -> Result<(), String> {
        let side = self.splat_resolution();
        if splat.len() != (side * side) as usize {
            return Err(format!(
                "terrain: {} splat texels for a {side}×{side} map",
                splat.len()
            ));
        }
        self.splat = Arc::new(
            splat
                .into_iter()
                .map(|w| normalize_weights(w.map(f32::from)))
                .collect(),
        );
        self.splat_revision = next_revision();
        Ok(())
    }

    /// Sets every height from `f(x, z)` (the entity's space).
    pub fn set_heights_with(&mut self, mut f: impl FnMut(f32, f32) -> f32) {
        let (r, half, step) = (self.resolution, self.size * 0.5, self.spacing());
        let heights = (0..r * r)
            .map(|k| {
                let (i, j) = (k % r, k / r);
                let h = f(-half + i as f32 * step, -half + j as f32 * step);
                if h.is_finite() {
                    h
                } else {
                    0.0
                }
            })
            .collect();
        self.heights = Arc::new(heights);
        self.revision = next_revision();
    }

    /// Changes the number of samples a side, resampling the heights and
    /// the splat map so the ground keeps its shape.
    pub fn set_resolution(&mut self, resolution: u32) {
        let resolution = clamp_resolution(resolution);
        if resolution == self.resolution {
            return;
        }
        let old = self.clone();
        let (half, step) = (self.size * 0.5, self.size / (resolution - 1) as f32);
        self.resolution = resolution;
        self.heights = Arc::new(
            (0..resolution * resolution)
                .map(|k| {
                    let (i, j) = (k % resolution, k / resolution);
                    old.height_at(-half + i as f32 * step, -half + j as f32 * step)
                })
                .collect(),
        );
        let side = splat_side(resolution);
        let old_side = old.splat_resolution();
        self.splat = Arc::new(
            (0..side * side)
                .map(|k| {
                    let (i, j) = (k % side, k / side);
                    let u = (i as f32 + 0.5) / side as f32;
                    let v = (j as f32 + 0.5) / side as f32;
                    let oi = ((u * old_side as f32) as u32).min(old_side - 1);
                    let oj = ((v * old_side as f32) as u32).min(old_side - 1);
                    old.splat[(oj * old_side + oi) as usize]
                })
                .collect(),
        );
        self.revision = next_revision();
        self.splat_revision = next_revision();
    }

    /// The sample at column `i` (X) and row `j` (Z), clamped to the grid.
    #[must_use]
    pub fn sample(&self, i: i64, j: i64) -> f32 {
        let last = i64::from(self.resolution) - 1;
        let (i, j) = (i.clamp(0, last), j.clamp(0, last));
        self.heights[(j * (last + 1) + i) as usize]
    }

    /// Grid coordinates (fractional samples) of a point in the entity's
    /// space.
    fn grid(&self, x: f32, z: f32) -> (f32, f32) {
        let half = self.size * 0.5;
        let step = self.spacing();
        ((x + half) / step, (z + half) / step)
    }

    /// Whether `(x, z)` (the entity's space) is over the terrain.
    #[must_use]
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let half = self.size * 0.5;
        x.abs() <= half && z.abs() <= half
    }

    /// The ground's height at `(x, z)` in the entity's space, on the same
    /// triangles physics and drawing use. Outside the square, the edge's.
    #[must_use]
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let last = (self.resolution - 1) as f32;
        let (gx, gz) = self.grid(x, z);
        let (gx, gz) = (gx.clamp(0.0, last), gz.clamp(0.0, last));
        let (i, j) = (gx.floor().min(last - 1.0), gz.floor().min(last - 1.0));
        let (fx, fz) = (gx - i, gz - j);
        let (i, j) = (i as i64, j as i64);
        let h00 = self.sample(i, j);
        let h10 = self.sample(i + 1, j);
        let h01 = self.sample(i, j + 1);
        let h11 = self.sample(i + 1, j + 1);
        // Each cell is split along its (0,0)-(1,1) diagonal, as parry's
        // heightfield is.
        if fz >= fx {
            h00 + fz * (h01 - h00) + fx * (h11 - h01)
        } else {
            h00 + fx * (h10 - h00) + fz * (h11 - h10)
        }
    }

    /// The smoothed surface normal at `(x, z)` (the entity's space).
    #[must_use]
    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let e = self.spacing();
        let dx = self.height_at(x + e, z) - self.height_at(x - e, z);
        let dz = self.height_at(x, z + e) - self.height_at(x, z - e);
        Vec3::new(-dx, 2.0 * e, -dz).normalize_or(Vec3::Y)
    }

    /// The lowest and highest height.
    #[must_use]
    pub fn height_range(&self) -> (f32, f32) {
        self.heights
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &h| {
                (lo.min(h), hi.max(h))
            })
    }

    /// Where a ray (the entity's space) first meets the ground from above:
    /// the distance along `dir` (in `dir`'s units) and the normal.
    #[must_use]
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<(f32, Vec3)> {
        let len = dir.length();
        if len <= 1e-9 || !origin.is_finite() || !dir.is_finite() {
            return None;
        }
        let d = dir / len;
        let max = max_distance * len;
        let (lo, hi) = self.height_range();
        let half = self.size * 0.5;
        let lo_b = Vec3::new(-half, lo - 0.01, -half);
        let hi_b = Vec3::new(half, hi + 0.01, half);
        // Slab test against the bounds.
        let (mut t0, mut t1) = (0.0_f32, max);
        for a in 0..3 {
            if d[a].abs() < 1e-9 {
                if origin[a] < lo_b[a] || origin[a] > hi_b[a] {
                    return None;
                }
            } else {
                let (mut ta, mut tb) = ((lo_b[a] - origin[a]) / d[a], (hi_b[a] - origin[a]) / d[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
        }
        if t0 > t1 {
            return None;
        }
        let above = |t: f32| {
            let p = origin + d * t;
            p.y - self.height_at(p.x, p.z)
        };
        if above(t0) < 0.0 {
            return None; // starts under the ground
        }
        let step = (self.spacing() * 0.4).max(1e-3);
        let mut a = t0;
        while a < t1 {
            let b = (a + step).min(t1);
            if above(b) <= 0.0 {
                let (mut lo_t, mut hi_t) = (a, b);
                for _ in 0..24 {
                    let mid = f32::midpoint(lo_t, hi_t);
                    if above(mid) > 0.0 {
                        lo_t = mid;
                    } else {
                        hi_t = mid;
                    }
                }
                let p = origin + d * hi_t;
                return Some((hi_t / len, self.normal_at(p.x, p.z)));
            }
            a = b;
        }
        None
    }

    /// Calls `f(index, x, z, weight)` for every height sample within
    /// `radius` of `centre` (the entity's X and Z).
    fn for_samples(&self, centre: Vec2, radius: f32, mut f: impl FnMut(usize, f32, f32, f32)) {
        let (gx, gz) = self.grid(centre.x, centre.y);
        let reach = radius / self.spacing();
        let last = i64::from(self.resolution) - 1;
        let (i0, i1) = (
            ((gx - reach).floor() as i64).max(0),
            ((gx + reach).ceil() as i64).min(last),
        );
        let (j0, j1) = (
            ((gz - reach).floor() as i64).max(0),
            ((gz + reach).ceil() as i64).min(last),
        );
        let (half, step) = (self.size * 0.5, self.spacing());
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (x, z) = (-half + i as f32 * step, -half + j as f32 * step);
                let w = falloff(Vec2::new(x, z).distance(centre), radius);
                if w > 0.0 {
                    f((j * (last + 1) + i) as usize, x, z, w);
                }
            }
        }
    }

    /// Sculpts with `tool` round `centre` (the entity's X and Z), `radius`
    /// metres wide; see [`Sculpt`] for what `amount` means. Returns whether
    /// anything changed.
    pub fn sculpt(&mut self, tool: Sculpt, centre: Vec2, radius: f32, amount: f32) -> bool {
        if !centre.is_finite()
            || radius.is_nan()
            || radius <= 0.0
            || !amount.is_finite()
            || amount == 0.0
        {
            return false;
        }
        let r = i64::from(self.resolution);
        let mut touched = Vec::new();
        self.for_samples(centre, radius, |k, x, z, w| touched.push((k, x, z, w)));
        if touched.is_empty() {
            return false;
        }
        let old = Arc::clone(&self.heights);
        let heights = Arc::make_mut(&mut self.heights);
        let sample = |k: usize, di: i64, dj: i64| {
            let (i, j) = (k as i64 % r + di, k as i64 / r + dj);
            old[(j.clamp(0, r - 1) * r + i.clamp(0, r - 1)) as usize]
        };
        for (k, x, z, w) in touched {
            let h = old[k];
            heights[k] = match tool {
                Sculpt::Raise => h + amount * w,
                Sculpt::Lower => h - amount * w,
                Sculpt::Smooth => {
                    let mut sum = 0.0;
                    for (di, dj) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        sum += sample(k, di, dj);
                    }
                    let avg = (sum + h) / 9.0;
                    h + (avg - h) * (amount.clamp(0.0, 1.0) * w)
                }
                Sculpt::Flatten(target) => h + (target - h) * (amount.clamp(0.0, 1.0) * w),
                Sculpt::Noise => h + fbm(x * 0.25, z * 0.25) * amount * w,
            };
        }
        self.revision = next_revision();
        true
    }

    /// Paints `layer` (0..4) round `centre` (the entity's X and Z), `radius`
    /// metres wide, `strength` (0..1) of the way to fully covering at the
    /// centre. Returns whether anything changed.
    pub fn paint(&mut self, layer: usize, centre: Vec2, radius: f32, strength: f32) -> bool {
        if layer >= TERRAIN_LAYERS || !centre.is_finite() || radius.is_nan() || radius <= 0.0 {
            return false;
        }
        let strength = strength.clamp(0.0, 1.0);
        if strength == 0.0 {
            return false;
        }
        let side = self.splat_resolution();
        let texel = self.size / side as f32;
        let half = self.size * 0.5;
        let to_texel = |v: f32| (v + half) / texel - 0.5;
        let reach = radius / texel;
        let (cx, cz) = (to_texel(centre.x), to_texel(centre.y));
        let last = i64::from(side) - 1;
        let (i0, i1) = (
            ((cx - reach).floor() as i64).max(0),
            ((cx + reach).ceil() as i64).min(last),
        );
        let (j0, j1) = (
            ((cz - reach).floor() as i64).max(0),
            ((cz + reach).ceil() as i64).min(last),
        );
        if i0 > i1 || j0 > j1 {
            return false;
        }
        let splat = Arc::make_mut(&mut self.splat);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (x, z) = (
                    -half + (i as f32 + 0.5) * texel,
                    -half + (j as f32 + 0.5) * texel,
                );
                let w = falloff(Vec2::new(x, z).distance(centre), radius) * strength;
                if w <= 0.0 {
                    continue;
                }
                let k = (j * (last + 1) + i) as usize;
                let mut f = splat[k].map(f32::from);
                let target = f[layer] + (255.0 - f[layer]) * w;
                let others: f32 = f
                    .iter()
                    .enumerate()
                    .filter(|(l, _)| *l != layer)
                    .map(|(_, v)| v)
                    .sum();
                let left = 255.0 - target;
                for (l, v) in f.iter_mut().enumerate() {
                    if l == layer {
                        *v = target;
                    } else if others > 0.0 {
                        *v *= left / others;
                    }
                }
                splat[k] = normalize_weights(f);
            }
        }
        self.splat_revision = next_revision();
        true
    }

    /// Repaints everything by height and slope with the
    /// [`default_layers`] roles: sand up to `beach` metres above
    /// `water_level` (the entity's space), rock on slopes steeper than
    /// `cliff` degrees, dirt on the slopes just below that, grass
    /// elsewhere.
    pub fn auto_paint(&mut self, water_level: f32, beach: f32, cliff: f32) {
        let side = self.splat_resolution();
        let texel = self.size / side as f32;
        let half = self.size * 0.5;
        let beach = beach.max(0.0);
        let splat = (0..side * side)
            .map(|k| {
                let (i, j) = (k % side, k / side);
                let (x, z) = (
                    -half + (i as f32 + 0.5) * texel,
                    -half + (j as f32 + 0.5) * texel,
                );
                let h = self.height_at(x, z);
                let slope = self.normal_at(x, z).y.clamp(-1.0, 1.0).acos().to_degrees();
                let rock = smoothstep(cliff - 6.0, cliff + 4.0, slope);
                let dirt = smoothstep(cliff - 18.0, cliff - 6.0, slope) * (1.0 - rock) * 0.8;
                let sand = (1.0
                    - smoothstep(water_level + beach * 0.6, water_level + beach + 0.2, h))
                    * (1.0 - rock);
                let mut w = [0.0_f32; TERRAIN_LAYERS];
                w[layer::ROCK] = rock;
                w[layer::SAND] = sand;
                w[layer::DIRT] = dirt * (1.0 - sand);
                w[layer::GRASS] = (1.0 - rock - sand - w[layer::DIRT]).max(0.0);
                normalize_weights(w.map(|v| v * 255.0))
            })
            .collect();
        self.splat = Arc::new(splat);
        self.splat_revision = next_revision();
    }

    /// The layer weights (0..1, summing to 1) at `(x, z)` in the entity's
    /// space (the nearest splat texel).
    #[must_use]
    pub fn weights_at(&self, x: f32, z: f32) -> [f32; TERRAIN_LAYERS] {
        let side = self.splat_resolution();
        let half = self.size * 0.5;
        let u = ((x + half) / self.size).clamp(0.0, 1.0);
        let v = ((z + half) / self.size).clamp(0.0, 1.0);
        let i = ((u * side as f32) as u32).min(side - 1);
        let j = ((v * side as f32) as u32).min(side - 1);
        self.splat[(j * side + i) as usize].map(|w| f32::from(w) / 255.0)
    }

    /// The heights as a 16-bit greyscale PNG: black is the lowest height,
    /// white the highest. Returns the PNG and that `(lowest, highest)`.
    #[must_use]
    pub fn heightmap_png(&self) -> (Vec<u8>, f32, f32) {
        let (lo, hi) = self.height_range();
        (
            encode_heights(&self.heights, self.resolution, lo, hi),
            lo,
            hi,
        )
    }

    /// Replaces the heights from a greyscale image (any size; it's
    /// resampled to the grid): black is `lowest`, white `highest` metres.
    ///
    /// # Errors
    ///
    /// Returns a message when the bytes aren't a readable image.
    pub fn load_heightmap(
        &mut self,
        image: &[u8],
        lowest: f32,
        highest: f32,
    ) -> Result<(), String> {
        let img = image::load_from_memory(image)
            .map_err(|e| format!("terrain: couldn't read the heightmap: {e}"))?
            .into_luma16();
        let (w, h) = img.dimensions();
        if w < 2 || h < 2 {
            return Err("terrain: the heightmap is too small".into());
        }
        let r = self.resolution;
        let heights = (0..r * r)
            .map(|k| {
                let (i, j) = (k % r, k / r);
                let x = i as f32 / (r - 1) as f32 * (w - 1) as f32;
                let y = j as f32 / (r - 1) as f32 * (h - 1) as f32;
                let (x0, y0) = (x.floor() as u32, y.floor() as u32);
                let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                let (fx, fy) = (x - x0 as f32, y - y0 as f32);
                let p = |a: u32, b: u32| f32::from(img.get_pixel(a, b).0[0]) / 65535.0;
                let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
                let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
                let t = top + (bottom - top) * fy;
                lowest + (highest - lowest) * t
            })
            .collect();
        self.heights = Arc::new(heights);
        self.revision = next_revision();
        Ok(())
    }

    // --- drawing -----------------------------------------------------

    /// Chunk `(cx, cz)`'s quads: `[i0, i1) × [j0, j1)` in samples.
    fn chunk_range(&self, cx: u32, cz: u32) -> (u32, u32, u32, u32) {
        let last = self.resolution - 1;
        (
            cx * CHUNK,
            ((cx + 1) * CHUNK).min(last),
            cz * CHUNK,
            ((cz + 1) * CHUNK).min(last),
        )
    }

    fn chunk_count(&self) -> u32 {
        (self.resolution - 1).div_ceil(CHUNK)
    }

    /// A hash of everything chunk `(cx, cz)`'s mesh depends on: its
    /// samples plus a one-sample border (for the normals), and the size.
    fn chunk_hash(&self, cx: u32, cz: u32) -> u64 {
        let (i0, i1, j0, j1) = self.chunk_range(cx, cz);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.size.to_bits().hash(&mut hasher);
        self.resolution.hash(&mut hasher);
        for j in i64::from(j0) - 1..=i64::from(j1) + 1 {
            for i in i64::from(i0) - 1..=i64::from(i1) + 1 {
                self.sample(i, j).to_bits().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Chunk `(cx, cz)` as mesh data in the entity's space: positions,
    /// smooth normals, and UVs across the whole terrain (0..1).
    #[must_use]
    pub fn chunk_mesh(&self, cx: u32, cz: u32) -> MeshData {
        let (i0, i1, j0, j1) = self.chunk_range(cx, cz);
        let (half, step) = (self.size * 0.5, self.spacing());
        let last = (self.resolution - 1) as f32;
        let w = i1 - i0 + 1;
        let mut vertices = Vec::with_capacity((w * (j1 - j0 + 1)) as usize);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (ii, jj) = (i64::from(i), i64::from(j));
                let h = self.sample(ii, jj);
                let dx = self.sample(ii + 1, jj) - self.sample(ii - 1, jj);
                let dz = self.sample(ii, jj + 1) - self.sample(ii, jj - 1);
                let n = Vec3::new(-dx, 2.0 * step, -dz).normalize_or(Vec3::Y);
                vertices.push(Vertex::new(
                    [-half + i as f32 * step, h, -half + j as f32 * step],
                    n.to_array(),
                    [i as f32 / last, j as f32 / last],
                ));
            }
        }
        let mut indices = Vec::with_capacity(((i1 - i0) * (j1 - j0) * 6) as usize);
        for j in 0..j1 - j0 {
            for i in 0..i1 - i0 {
                let a = j * w + i; // (x0, z0)
                let b = (j + 1) * w + i; // (x0, z1)
                let c = (j + 1) * w + i + 1; // (x1, z1)
                let d = j * w + i + 1; // (x1, z0)
                indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
        MeshData::new(vertices, indices)
    }

    /// The GPU chunks and material, built (or brought up to date) with
    /// `renderer`: only chunks whose samples changed are uploaded again.
    pub fn prepare_gpu(&self, renderer: &Renderer) {
        let mut cache = self.gpu.0.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.revision != self.revision || cache.size != self.size.to_bits() {
            let n = self.chunk_count();
            let mut chunks = HashMap::new();
            for cz in 0..n {
                for cx in 0..n {
                    let hash = self.chunk_hash(cx, cz);
                    let mesh = match cache.chunks.remove(&(cx, cz)) {
                        Some((h, mesh)) if h == hash => mesh,
                        _ => renderer.create_mesh(&self.chunk_mesh(cx, cz)),
                    };
                    chunks.insert((cx, cz), (hash, mesh));
                }
            }
            cache.chunks = chunks;
            cache.revision = self.revision;
            cache.size = self.size.to_bits();
        }
        let atlas_key = layers_key(&self.layers);
        if cache.atlas.as_ref().is_none_or(|(k, _)| *k != atlas_key) {
            let (w, h, rgba) = build_atlas(&self.layers);
            cache.atlas = Some((
                atlas_key,
                renderer.create_texture_rgba_mipmapped(w, h, &rgba, false),
            ));
            cache.material = None;
        }
        let material_key = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            atlas_key.hash(&mut hasher);
            self.splat_revision.hash(&mut hasher);
            self.size.to_bits().hash(&mut hasher);
            self.roughness.to_bits().hash(&mut hasher);
            for l in &self.layers {
                l.tile.to_bits().hash(&mut hasher);
            }
            hasher.finish()
        };
        if cache
            .material
            .as_ref()
            .is_none_or(|(k, _)| *k != material_key)
        {
            let side = self.splat_resolution();
            let rgba: Vec<u8> = self.splat.iter().flatten().copied().collect();
            let splat = renderer.create_data_texture(side, side, &rgba);
            let atlas = cache.atlas.as_ref().map(|(_, t)| t.clone());
            let Some(atlas) = atlas else { return };
            let material = renderer.create_terrain_material(&automata_renderer::TerrainMaterial {
                layers: atlas,
                splat,
                splat_texels: side,
                size: self.size,
                tiles: self.layers.clone().map(|l| l.tile.max(0.05)),
                roughness: self.roughness.clamp(0.04, 1.0),
            });
            cache.material = Some((material_key, material));
        }
    }

    /// The prepared chunks and material (empty until [`Self::prepare_gpu`]).
    fn gpu_parts(&self) -> Option<(Vec<Mesh>, Material)> {
        let cache = self.gpu.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (_, material) = cache.material.as_ref()?;
        if cache.revision != self.revision {
            return None;
        }
        let mut keys: Vec<_> = cache.chunks.keys().copied().collect();
        keys.sort_unstable();
        Some((
            keys.iter().map(|k| cache.chunks[k].1.clone()).collect(),
            material.clone(),
        ))
    }
}

/// Weights (any scale) as bytes summing to exactly 255.
fn normalize_weights(w: [f32; TERRAIN_LAYERS]) -> [u8; 4] {
    let w = w.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let sum: f32 = w.iter().sum();
    if sum <= 1e-6 {
        return [255, 0, 0, 0];
    }
    let mut out = w.map(|v| (v / sum * 255.0).round().clamp(0.0, 255.0) as u8);
    let total: i32 = out.iter().map(|&v| i32::from(v)).sum();
    // Rounding can leave it a little off 255: fix the biggest.
    let big = (0..TERRAIN_LAYERS).max_by_key(|&i| out[i]).unwrap_or(0);
    out[big] = (i32::from(out[big]) + 255 - total).clamp(0, 255) as u8;
    out
}

/// Smooth value noise in -1..1.
fn value_noise(x: f32, z: f32) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let h = |i: f32, j: f32| hash2(i as i32, j as i32);
    let a = h(x0, z0) + (h(x0 + 1.0, z0) - h(x0, z0)) * ux;
    let b = h(x0, z0 + 1.0) + (h(x0 + 1.0, z0 + 1.0) - h(x0, z0 + 1.0)) * ux;
    (a + (b - a) * uz) * 2.0 - 1.0
}

fn hash2(i: i32, j: i32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

/// Three octaves of value noise, in about -1..1.
#[must_use]
pub fn fbm(x: f32, z: f32) -> f32 {
    value_noise(x, z) * 0.57
        + value_noise(x * 2.03 + 17.1, z * 2.03 - 9.3) * 0.29
        + value_noise(x * 4.11 - 3.7, z * 4.11 + 21.9) * 0.14
}

// --- the GPU cache and atlas ------------------------------------------

#[derive(Default)]
struct GpuState {
    revision: u64,
    size: u32,
    chunks: HashMap<(u32, u32), (u64, Mesh)>,
    atlas: Option<(u64, Texture)>,
    material: Option<(u64, Material)>,
}

/// A terrain's GPU meshes and material, shared by clones of it.
#[derive(Clone, Default)]
struct GpuCache(Arc<Mutex<GpuState>>);

fn layers_key(layers: &[TerrainLayer; TERRAIN_LAYERS]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for l in layers {
        for c in l.color {
            c.to_bits().hash(&mut hasher);
        }
        l.texture.hash(&mut hasher);
        // A texture file that changes on disk makes a new atlas.
        if !l.texture.is_empty() {
            std::fs::metadata(automata_assets::resolve(&l.texture))
                .and_then(|m| m.modified())
                .ok()
                .hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Linear 0..1 to an sRGB byte.
fn srgb_byte(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

fn srgb_to_linear(b: u8) -> f32 {
    let s = f32::from(b) / 255.0;
    if s <= 0.040_45 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Tileable value noise over a `period`-cell lattice, 0..1.
fn tiled_noise(x: f32, y: f32, period: i32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |i: f32, j: f32| hash2((i as i32).rem_euclid(period), (j as i32).rem_euclid(period));
    let a = h(x0, y0) + (h(x0 + 1.0, y0) - h(x0, y0)) * ux;
    let b = h(x0, y0 + 1.0) + (h(x0 + 1.0, y0 + 1.0) - h(x0, y0 + 1.0)) * ux;
    a + (b - a) * uy
}

/// One layer's tile (`content`² linear RGB): its texture tinted by its
/// colour, or the colour with a soft grain.
fn layer_tile(layer: &TerrainLayer, index: usize, content: u32) -> Vec<[f32; 3]> {
    if !layer.texture.is_empty() {
        let path = automata_assets::resolve(&layer.texture);
        match image::open(&path) {
            Ok(img) => {
                let img = image::imageops::resize(
                    &img.into_rgba8(),
                    content,
                    content,
                    image::imageops::FilterType::Triangle,
                );
                return img
                    .pixels()
                    .map(|p| {
                        let c = [0, 1, 2].map(|k| srgb_to_linear(p.0[k]));
                        [
                            c[0] * layer.color[0],
                            c[1] * layer.color[1],
                            c[2] * layer.color[2],
                        ]
                    })
                    .collect();
            }
            Err(e) => tracing::warn!("terrain: couldn't load '{}': {e}", layer.texture),
        }
    }
    let seed = index as f32 * 37.0;
    (0..content * content)
        .map(|k| {
            let (x, y) = ((k % content) as f32, (k / content) as f32);
            let s = content as f32;
            let g = tiled_noise(x / s * 8.0 + seed, y / s * 8.0, 8) * 0.55
                + tiled_noise(x / s * 32.0 + seed, y / s * 32.0, 32) * 0.3
                + tiled_noise(x / s * 96.0, y / s * 96.0 + seed, 96) * 0.15;
            let shade = 0.82 + 0.36 * g;
            layer.color.map(|c| c * shade)
        })
        .collect()
}

/// The four layers in a 2 × 2 atlas, each tile wrapped into a gutter so
/// filtering and mipmaps don't bleed between them. Returns `(w, h, rgba)`.
fn build_atlas(layers: &[TerrainLayer; TERRAIN_LAYERS]) -> (u32, u32, Vec<u8>) {
    let side = CELL * 2;
    let content = CELL - 2 * GUTTER;
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    for (index, layer) in layers.iter().enumerate() {
        let tile = layer_tile(layer, index, content);
        let (ox, oy) = ((index as u32 % 2) * CELL, (index as u32 / 2) * CELL);
        for y in 0..CELL {
            for x in 0..CELL {
                let tx = (x + content - GUTTER % content) % content;
                let ty = (y + content - GUTTER % content) % content;
                let c = tile[(ty * content + tx) as usize];
                let o = (((oy + y) * side + ox + x) * 4) as usize;
                rgba[o] = srgb_byte(c[0]);
                rgba[o + 1] = srgb_byte(c[1]);
                rgba[o + 2] = srgb_byte(c[2]);
                rgba[o + 3] = 255;
            }
        }
    }
    (side, side, rgba)
}

/// Brings every terrain's GPU chunks and material up to date.
pub(crate) fn prepare_terrains(world: &SceneWorld, renderer: &Renderer) {
    for terrain in &mut world.query::<&Terrain>() {
        terrain.prepare_gpu(renderer);
    }
}

/// Draws every prepared terrain.
pub(crate) fn record_terrains(world: &SceneWorld, draw: &mut DrawList) {
    for (terrain, global) in &mut world.query::<(&Terrain, &GlobalTransform)>() {
        if let Some((meshes, material)) = terrain.gpu_parts() {
            let model = global.matrix();
            for mesh in &meshes {
                draw.mesh(mesh, model, &material);
            }
        }
    }
}

/// The nearest terrain along a world ray: `(entity, distance, normal)`.
pub(crate) fn raycast_terrains(
    world: &SceneWorld,
    origin: Vec3,
    dir: Vec3,
    max_distance: f32,
    skip: &dyn Fn(Entity) -> bool,
) -> Option<(Entity, f32, Vec3)> {
    let mut best: Option<(Entity, f32, Vec3)> = None;
    for (entity, terrain, global) in &mut world.query::<(Entity, &Terrain, &GlobalTransform)>() {
        if skip(entity) {
            continue;
        }
        let model = global.matrix();
        let det = model.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            continue;
        }
        let inverse = model.inverse();
        let limit = best.map_or(max_distance, |b| b.1);
        if let Some((t, n)) = terrain.raycast(
            inverse.transform_point3(origin),
            inverse.transform_vector3(dir),
            limit,
        ) {
            let normal = (Mat3::from_mat4(inverse).transpose() * n).normalize_or(Vec3::Y);
            best = Some((entity, t, normal));
        }
    }
    best
}

/// The scene's terrains, for standing things on them.
pub(crate) struct TerrainGround {
    terrains: Vec<(Mat4, Mat4, Terrain)>,
}

impl TerrainGround {
    /// No terrain at all.
    pub(crate) fn none() -> Self {
        Self {
            terrains: Vec::new(),
        }
    }

    /// Every terrain in `world`.
    pub(crate) fn of(world: &SceneWorld) -> Self {
        let terrains = world
            .query::<(&Terrain, &GlobalTransform)>()
            .iter()
            .filter_map(|(t, g)| {
                let m = g.matrix();
                let det = m.determinant();
                (det.is_finite() && det.abs() > 1e-12).then(|| (m, m.inverse(), t.clone()))
            })
            .collect();
        Self { terrains }
    }

    /// Whether there are none.
    pub(crate) fn is_empty(&self) -> bool {
        self.terrains.is_empty()
    }

    /// A key that changes whenever the ground does.
    pub(crate) fn key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (m, _, t) in &self.terrains {
            for v in m.to_cols_array() {
                v.to_bits().hash(&mut hasher);
            }
            t.revision.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The ground under world point `p` (straight down or up): its world
    /// height and normal, if `p` is over a terrain.
    pub(crate) fn under(&self, p: Vec3) -> Option<(f32, Vec3)> {
        for (m, inv, t) in &self.terrains {
            let local = inv.transform_point3(p);
            if !t.contains(local.x, local.z) {
                continue;
            }
            let ground = Vec3::new(local.x, t.height_at(local.x, local.z), local.z);
            let world = m.transform_point3(ground);
            let n = (Mat3::from_mat4(*inv).transpose() * t.normal_at(local.x, local.z))
                .normalize_or(Vec3::Y);
            return Some((world.y, n));
        }
        None
    }
}

// --- saving ------------------------------------------------------------

/// The heights as saved: a 16-bit PNG spanning `min..max` (empty when
/// flat at `min`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct HeightsFile {
    min: f32,
    max: f32,
    #[serde(skip_serializing_if = "String::is_empty")]
    png: String,
}

/// How a [`Terrain`] is written in a scene file.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct TerrainFile {
    size: f32,
    resolution: u32,
    layers: [TerrainLayer; TERRAIN_LAYERS],
    roughness: f32,
    collide: bool,
    heights: HeightsFile,
    /// The splat map as an RGBA PNG (empty: all the first layer).
    #[serde(skip_serializing_if = "String::is_empty")]
    splat: String,
}

impl Default for TerrainFile {
    fn default() -> Self {
        Terrain::default().into()
    }
}

fn encode_heights(heights: &[f32], resolution: u32, lo: f32, hi: f32) -> Vec<u8> {
    let span = (hi - lo).max(1e-6);
    let data: Vec<u16> = heights
        .iter()
        .map(|h| (((h - lo) / span).clamp(0.0, 1.0) * 65535.0).round() as u16)
        .collect();
    let mut out = Vec::new();
    if let Some(img) =
        image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(resolution, resolution, data)
    {
        let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
    }
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()
}

impl From<Terrain> for TerrainFile {
    fn from(t: Terrain) -> Self {
        let (lo, hi) = t.height_range();
        let heights = if hi - lo <= 1e-6 {
            HeightsFile {
                min: lo,
                max: lo,
                png: String::new(),
            }
        } else {
            HeightsFile {
                min: lo,
                max: hi,
                png: base64_encode(&encode_heights(&t.heights, t.resolution, lo, hi)),
            }
        };
        let side = t.splat_resolution();
        let splat = if t.splat.iter().all(|w| *w == [255, 0, 0, 0]) {
            String::new()
        } else {
            let rgba: Vec<u8> = t.splat.iter().flatten().copied().collect();
            let mut out = Vec::new();
            if let Some(img) = image::RgbaImage::from_raw(side, side, rgba) {
                let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
            }
            base64_encode(&out)
        };
        Self {
            size: t.size,
            resolution: t.resolution,
            layers: t.layers,
            roughness: t.roughness,
            collide: t.collide,
            heights,
            splat,
        }
    }
}

impl From<TerrainFile> for Terrain {
    fn from(f: TerrainFile) -> Self {
        let mut t = Terrain::new(f.size, f.resolution);
        t.layers = f.layers;
        t.roughness = f.roughness;
        t.collide = f.collide;
        let r = t.resolution;
        let base = if f.heights.min.is_finite() {
            f.heights.min
        } else {
            0.0
        };
        let mut heights = vec![base; (r * r) as usize];
        if !f.heights.png.is_empty() {
            let decoded = base64_decode(&f.heights.png)
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
                .map(image::DynamicImage::into_luma16);
            match decoded {
                Some(img) if img.dimensions() == (r, r) => {
                    let span = f.heights.max - f.heights.min;
                    for (h, p) in heights.iter_mut().zip(img.pixels()) {
                        *h = base + span * f32::from(p.0[0]) / 65535.0;
                    }
                }
                _ => tracing::warn!("terrain: unreadable heights in the scene file; flat instead"),
            }
        }
        t.heights = Arc::new(heights);
        if !f.splat.is_empty() {
            let side = t.splat_resolution();
            let decoded = base64_decode(&f.splat)
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
                .map(image::DynamicImage::into_rgba8);
            match decoded {
                Some(img) if img.dimensions() == (side, side) => {
                    t.splat = Arc::new(img.pixels().map(|p| p.0).collect());
                }
                _ => tracing::warn!(
                    "terrain: unreadable splat map in the scene file; all grass instead"
                ),
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hill() -> Terrain {
        let mut t = Terrain::new(32.0, 33);
        t.set_heights_with(|x, z| 8.0 - 0.02 * (x * x + z * z));
        t
    }

    #[test]
    fn heights_follow_the_triangles_and_stay_on_the_grid() {
        let t = hill();
        assert_eq!(t.resolution(), 33);
        assert!((t.spacing() - 1.0).abs() < 1e-6);
        // On a sample, exactly its height.
        assert!((t.height_at(0.0, 0.0) - 8.0).abs() < 1e-5);
        assert!((t.height_at(4.0, -2.0) - (8.0 - 0.02 * 20.0)).abs() < 1e-5);
        // Between samples, on the plane of the triangle (bounded by its corners).
        let h = t.height_at(0.3, 0.6);
        assert!(h < 8.0 && h > 7.9, "{h}");
        // Off the edge: the edge's height.
        assert!((t.height_at(100.0, 0.0) - t.height_at(16.0, 0.0)).abs() < 1e-5);
        // The top of a dome faces up; its side leans outwards.
        assert!(t.normal_at(0.0, 0.0).y > 0.99);
        assert!(t.normal_at(10.0, 0.0).x > 0.2);
    }

    #[test]
    fn rays_hit_the_ground_from_above() {
        let t = hill();
        let (d, n) = t
            .raycast(Vec3::new(2.0, 50.0, 3.0), Vec3::NEG_Y, 100.0)
            .unwrap();
        let expected = 50.0 - t.height_at(2.0, 3.0);
        assert!((d - expected).abs() < 1e-3, "{d} vs {expected}");
        assert!(n.y > 0.9);
        // A slanted ray, with a scaled direction: the distance is in its units.
        let (d2, _) = t
            .raycast(
                Vec3::new(-20.0, 20.0, 0.0),
                Vec3::new(2.0, -1.0, 0.0),
                100.0,
            )
            .unwrap();
        let p = Vec3::new(-20.0, 20.0, 0.0) + Vec3::new(2.0, -1.0, 0.0) * d2;
        assert!((p.y - t.height_at(p.x, p.z)).abs() < 1e-3);
        // Missing it, and from underneath.
        assert!(t
            .raycast(Vec3::new(40.0, 50.0, 0.0), Vec3::NEG_Y, 100.0)
            .is_none());
        assert!(t
            .raycast(Vec3::new(0.0, -5.0, 0.0), Vec3::Y, 100.0)
            .is_none());
        assert!(t
            .raycast(Vec3::new(0.0, 50.0, 0.0), Vec3::NEG_Y, 10.0)
            .is_none());
    }

    #[test]
    fn brushes_raise_lower_smooth_flatten_and_roughen() {
        let mut t = Terrain::new(16.0, 17);
        let r0 = t.revision();
        assert!(t.sculpt(Sculpt::Raise, Vec2::ZERO, 4.0, 2.0));
        assert_ne!(t.revision(), r0);
        assert!(
            (t.height_at(0.0, 0.0) - 2.0).abs() < 1e-5,
            "full amount at the centre"
        );
        assert!(
            t.height_at(2.0, 0.0) > 0.5 && t.height_at(2.0, 0.0) < 2.0,
            "less further out"
        );
        assert!(
            t.height_at(5.0, 0.0).abs() < 1e-6,
            "nothing past the radius"
        );
        t.sculpt(Sculpt::Lower, Vec2::ZERO, 4.0, 1.0);
        assert!((t.height_at(0.0, 0.0) - 1.0).abs() < 1e-5);
        // Smoothing pulls the peak down toward its neighbours.
        let peak = t.height_at(0.0, 0.0);
        t.sculpt(Sculpt::Smooth, Vec2::ZERO, 3.0, 1.0);
        assert!(t.height_at(0.0, 0.0) < peak);
        // Flatten to 3 m at full strength levels the centre.
        t.sculpt(Sculpt::Flatten(3.0), Vec2::ZERO, 4.0, 1.0);
        assert!((t.height_at(0.0, 0.0) - 3.0).abs() < 1e-5);
        // Noise roughens: neighbouring samples now differ.
        let mut flat = Terrain::new(16.0, 17);
        flat.sculpt(Sculpt::Noise, Vec2::ZERO, 8.0, 1.0);
        let (lo, hi) = flat.height_range();
        assert!(hi - lo > 0.05);
        // Nothing to do: no change.
        assert!(!flat.sculpt(Sculpt::Raise, Vec2::new(100.0, 0.0), 2.0, 1.0));
        assert!(!flat.sculpt(Sculpt::Raise, Vec2::ZERO, 2.0, 0.0));
    }

    #[test]
    fn painting_shifts_weight_between_layers() {
        let mut t = Terrain::new(16.0, 17);
        assert_eq!(t.splat_resolution(), 32);
        assert_eq!(t.weights_at(0.0, 0.0), [1.0, 0.0, 0.0, 0.0]);
        assert!(t.paint(layer::SAND, Vec2::ZERO, 3.0, 1.0));
        let w = t.weights_at(0.1, 0.1);
        assert!(w[layer::SAND] > 0.9, "{w:?}");
        assert!(
            t.weights_at(6.0, 6.0)[layer::GRASS] > 0.99,
            "untouched further out"
        );
        // Half strength over sand: half the rest becomes rock.
        t.paint(layer::ROCK, Vec2::ZERO, 3.0, 0.5);
        let w = t.weights_at(0.1, 0.1);
        assert!(
            (w[layer::ROCK] - 0.5).abs() < 0.05 && w[layer::SAND] > 0.4,
            "{w:?}"
        );
        assert!(t
            .splat()
            .iter()
            .all(|w| w.iter().map(|&v| u32::from(v)).sum::<u32>() == 255));
        assert!(!t.paint(7, Vec2::ZERO, 3.0, 1.0));
    }

    #[test]
    fn auto_paint_puts_sand_by_the_water_and_rock_on_cliffs() {
        let mut t = Terrain::new(64.0, 65);
        // A beach rising gently from the sea, then a cliff, then a plateau.
        t.set_heights_with(|x, _| {
            if x < 10.0 {
                (x + 32.0) * 0.12 - 2.0
            } else if x < 14.0 {
                3.0 + (x - 10.0) * 3.0
            } else {
                15.0
            }
        });
        t.auto_paint(0.0, 1.5, 35.0);
        let at = |x: f32| t.weights_at(x, 0.0);
        assert!(at(-20.0)[layer::SAND] > 0.8, "beach: {:?}", at(-20.0));
        assert!(at(12.0)[layer::ROCK] > 0.8, "cliff: {:?}", at(12.0));
        assert!(at(25.0)[layer::GRASS] > 0.8, "plateau: {:?}", at(25.0));
    }

    #[test]
    fn saved_terrains_come_back_the_same() {
        let mut t = hill();
        t.paint(layer::ROCK, Vec2::new(3.0, 3.0), 4.0, 1.0);
        t.layers[2].texture = "textures/sand.png".into();
        t.collide = false;
        let file = TerrainFile::from(t.clone());
        assert!(!file.heights.png.is_empty() && !file.splat.is_empty());
        let back = Terrain::from(file);
        assert_eq!(back.resolution(), t.resolution());
        assert_eq!(back.layers, t.layers);
        assert!(!back.collide);
        for (a, b) in back.heights().iter().zip(t.heights()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        assert_eq!(back.splat(), t.splat());
        // A flat, unpainted terrain saves no images at all.
        let flat = TerrainFile::from(Terrain::new(10.0, 9));
        assert!(flat.heights.png.is_empty() && flat.splat.is_empty());
        // Broken data loads flat instead of failing the scene.
        let broken = Terrain::from(TerrainFile {
            heights: HeightsFile {
                min: 1.0,
                max: 2.0,
                png: "not base64!".into(),
            },
            ..TerrainFile::default()
        });
        assert!(broken.heights().iter().all(|h| (*h - 1.0).abs() < 1e-6));
    }

    #[test]
    fn heightmaps_round_trip_and_resolution_changes_keep_the_shape() {
        let t = hill();
        let (png, lo, hi) = t.heightmap_png();
        let mut other = Terrain::new(32.0, 65);
        other.load_heightmap(&png, lo, hi).unwrap();
        assert!((other.height_at(5.0, 5.0) - t.height_at(5.0, 5.0)).abs() < 0.05);
        let mut resized = t.clone();
        resized.set_resolution(17);
        assert_eq!(resized.heights().len(), 17 * 17);
        assert_eq!(resized.splat().len(), 32 * 32);
        assert!((resized.height_at(0.0, 0.0) - 8.0).abs() < 1e-4);
        assert!(other.load_heightmap(b"nope", 0.0, 1.0).is_err());
        assert!(other.set_heights(vec![0.0; 3]).is_err());
    }

    #[test]
    fn chunks_tile_the_grid_and_face_up() {
        let t = Terrain::new(64.0, 70);
        assert_eq!(t.chunk_count(), 3);
        let mut quads = 0;
        for cz in 0..3 {
            for cx in 0..3 {
                let m = t.chunk_mesh(cx, cz);
                quads += m.indices.len() / 6;
                // Counter-clockwise from above: the first triangle faces +Y.
                let [a, b, c] = [m.indices[0], m.indices[1], m.indices[2]]
                    .map(|i| Vec3::from(m.vertices[i as usize].position));
                assert!((b - a).cross(c - a).y > 0.0);
            }
        }
        assert_eq!(quads, 69 * 69);
        // Changing one sample changes only the chunks that see it.
        let mut edited = t.clone();
        edited.sculpt(Sculpt::Raise, Vec2::new(-31.0, -31.0), 0.5, 1.0);
        assert_ne!(edited.chunk_hash(0, 0), t.chunk_hash(0, 0));
        assert_eq!(edited.chunk_hash(2, 2), t.chunk_hash(2, 2));
    }

    #[test]
    fn the_atlas_wraps_each_layer_into_its_gutter() {
        let (w, h, rgba) = build_atlas(&default_layers());
        assert_eq!((w, h), (1024, 1024));
        // Grass (top left) is green, sand (bottom left) is light.
        let px = |x: u32, y: u32| {
            let o = ((y * w + x) * 4) as usize;
            [rgba[o], rgba[o + 1], rgba[o + 2]]
        };
        let grass = px(256, 256);
        assert!(grass[1] > grass[0] && grass[1] > grass[2], "{grass:?}");
        let sand = px(256, 768);
        assert!(sand[0] > 150, "{sand:?}");
        // The gutter repeats the far edge of the tile.
        let content = CELL - 2 * GUTTER;
        assert_eq!(px(0, 300), px(content, 300));
    }

    #[test]
    fn grass_scatter_and_surface_rays_follow_the_terrain() {
        use crate::{Grass, Scatter, Scene, Transform};
        let mut scene = Scene::new();
        scene.spawn((Transform::from_position(Vec3::new(0.0, 1.0, 0.0)), hill()));
        scene.spawn((Transform::default(), Grass::area(6.0, 6.0)));
        let scatter = scene.spawn((Transform::default(), Scatter::area(6.0, 6.0)));
        crate::propagate_transforms(scene.world_mut());

        // Blades stand on the hill, not on the plane they were painted on.
        let mut draw = DrawList::new();
        crate::record_draw_list(scene.world(), &mut draw);
        let blades: Vec<[f32; 4]> = draw.grass_fields()[0]
            .chunks
            .iter()
            .flat_map(|c| c.blades.iter().map(|b| b.model[3]))
            .collect();
        assert_ne!(blades.len(), 0);
        let t = hill();
        for root in &blades {
            let want = 1.0 + t.height_at(root[0], root[2]);
            assert!((root[1] - want).abs() < 1e-3, "{root:?} vs {want}");
        }

        // So do scattered copies (in the scatterer's own space).
        let ground = TerrainGround::of(scene.world());
        let world = scene.world();
        let spots = world
            .get::<&Scatter>(scatter)
            .unwrap()
            .placements_on(Mat4::IDENTITY, &ground);
        assert!(!spots.is_empty());
        for spot in spots.iter() {
            let p = spot.w_axis;
            assert!((p.y - 1.0 - t.height_at(p.x, p.z)).abs() < 1e-3);
        }

        // Surface rays (the brushes' and the editor's) land on it.
        let hit = crate::raycast_surfaces(world, Vec3::new(2.0, 50.0, 1.0), -Vec3::Y, 100.0)
            .expect("the terrain is a surface");
        assert!((hit.point.y - 1.0 - t.height_at(2.0, 1.0)).abs() < 1e-3);
        assert!(hit.normal.y > 0.9);

        // Sculpting moves what grows on it.
        let before = ground.key();
        {
            let terrain = scene
                .world_mut()
                .query_mut::<&mut Terrain>()
                .into_iter()
                .next()
                .unwrap();
            assert!(terrain.sculpt(Sculpt::Raise, Vec2::ZERO, 8.0, 2.0));
        }
        assert_ne!(TerrainGround::of(scene.world()).key(), before);
        let mut draw = DrawList::new();
        crate::record_draw_list(scene.world(), &mut draw);
        let raised = draw.grass_fields()[0].chunks[0].blades[0].model[3];
        let world = scene.world();
        let terrain = world.query::<&Terrain>().iter().next().unwrap().clone();
        assert!((raised[1] - 1.0 - terrain.height_at(raised[0], raised[2])).abs() < 1e-3);
    }
}
