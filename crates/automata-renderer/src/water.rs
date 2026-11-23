//! Water surfaces: plain data a scene stores, plus the GPU-side uniform
//! and grid the water shader draws.
//!
//! A [`Water`] is a rectangle on the entity's local XZ plane, `size` wide,
//! animated by Gerstner waves. Every preset draws it; the
//! [`GraphicsQuality::Beautiful`](crate::GraphicsQuality::Beautiful)
//! preset adds refraction, depth absorption and shoreline foam.
//!
//! On top of the waves, a [`RippleMap`] (simulated by physics around the
//! camera) adds the rings and wakes things leave in the water.

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
    /// Density of the water for buoyancy, in the same units as collider
    /// density. Bodies less dense than this float; denser ones sink. The
    /// default 2.0 floats default bodies (density 1) half under.
    pub density: f32,
    /// How quickly the water slows bodies moving through it (fraction of
    /// velocity lost per second when fully submerged).
    pub drag: f32,
    /// The surface carries on to the horizon (a sea or ocean), beyond
    /// `size`. Waves and buoyancy still cover only `size`; past it the
    /// water carries on as a sheet that fades into the fog.
    pub endless: bool,
    /// How strongly sunlight through the waves dances on what's under the
    /// water (caustics), 0..=2 (0 = none). Medium graphics and up.
    pub caustics: f32,
    /// Wind over the water, 0..=1: from 0.5 up it blows spray off the
    /// wave crests (more and further the stronger it is). The scene's
    /// wind ([`Environment::wind`](crate::Environment::wind)) blows here
    /// too: spray follows whichever is stronger.
    pub wind: f32,
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
            density: 2.0,
            drag: 1.0,
            endless: false,
            caustics: 1.0,
            wind: 0.0,
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
            endless: true,
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
    /// The simulated ripple field: xy its world XZ corner, z 1 / its width,
    /// w 1 when there is one (see [`RippleMap`]).
    pub ripple: [f32; 4],
}

/// Per-wave constants for one draw (see [`WaterUniform`]): wave vectors
/// and phases, and amplitudes with the vertex grid's level of detail.
fn wave_constants(w: &Water, model: Mat4, time: f32) -> ([[f32; 4]; WAVES], [[f32; 4]; WAVES]) {
    let size = glam::Vec2::from(w.size).max(glam::Vec2::splat(0.01));
    let (scale, _, _) = model.to_scale_rotation_translation();
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
    (wave_k, wave_amp)
}

impl WaterUniform {
    pub fn new(draw: &WaterDraw, refraction: bool, time: f32) -> Self {
        let w = &draw.water;
        let size = glam::Vec2::from(w.size).max(glam::Vec2::splat(0.01));
        // The unit grid scaled to the surface's size, then the entity.
        let model = draw.model * Mat4::from_scale(glam::Vec3::new(size.x, 1.0, size.y));
        let (wave_k, wave_amp) = wave_constants(w, draw.model, time);
        let [dr, dg, db] = w.deep_color;
        let [sr, sg, sb] = w.shallow_color;
        Self {
            model: model.to_cols_array(),
            deep: [dr, dg, db, w.clarity.max(0.01)],
            shallow: [sr, sg, sb, w.foam.clamp(0.0, 1.0)],
            params: [
                w.wave_height.max(0.0),
                f32::from(u8::from(refraction)),
                time,
                0.0,
            ],
            wave_k,
            wave_amp,
            ripple: [0.0; 4],
        }
    }

    /// Draws `ripples` on this surface (none: the field is off).
    pub fn with_ripples(mut self, ripples: Option<&RippleMap>) -> Self {
        self.ripple = ripples.map_or([0.0; 4], |r| {
            let (corner, width) = r.extent();
            [corner.x, corner.y, 1.0 / width.max(1e-3), 1.0]
        });
        self
    }
}

/// Waters whose caustics the lit shaders draw at once (the nearest).
pub(crate) const MAX_CAUSTIC_WATERS: usize = 4;

/// What the lit shaders know about the water (`underwater`, `water_eye`,
/// `water_rects` and `water_levels` in common.wgsl's light uniform): the
/// water around the camera, if it's in or at the surface of one, and the
/// surfaces that throw caustics on what's under them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WaterView {
    /// rgb: the colour of the water around the camera, a: how quickly it
    /// swallows light (per metre; 0 = the camera isn't in any water).
    pub underwater: [f32; 4],
    /// x: the surface height above the camera, y: the near plane's half
    /// height (m, the waterline's wobble), z: waters with caustics.
    pub eye: [f32; 4],
    /// Per caustic water: world XZ min (xy) and max (zw).
    pub rects: [[f32; 4]; MAX_CAUSTIC_WATERS],
    /// Per caustic water: rest height, caustic strength, clarity (m).
    pub levels: [[f32; 4]; MAX_CAUSTIC_WATERS],
}

impl Default for WaterView {
    fn default() -> Self {
        Self {
            underwater: [0.0; 4],
            eye: [0.0; 4],
            rects: [[0.0; 4]; MAX_CAUSTIC_WATERS],
            levels: [[0.0; 4]; MAX_CAUSTIC_WATERS],
        }
    }
}

/// How quickly water of `clarity` metres dims the view through it, per
/// metre (green light; red goes ~2x faster, blue a little slower).
#[must_use]
pub(crate) fn underwater_extinction(clarity: f32) -> f32 {
    0.35 / clarity.max(0.05)
}

impl WaterView {
    /// The view of `waters` at `time` from `camera`. `caustics` off (Low
    /// graphics) leaves the caustic list empty.
    pub fn new(waters: &[WaterDraw], camera: &crate::Camera3D, time: f32, caustics: bool) -> Self {
        let mut view = Self::default();
        let eye = camera.position;
        let near_half = camera.near.max(1e-4) * (camera.fov_y_degrees.to_radians() * 0.5).tan();
        // The water the camera is in: of those over (or under) it, the one
        // whose surface is the nearest above the lens (or just below it,
        // where the waterline crosses the view).
        let mut best: Option<(f32, &WaterDraw)> = None;
        for w in waters {
            let surface = WaterSurface::new(&w.water, w.model, time);
            let height = if surface.contains(eye.x, eye.z) {
                surface.height_unchecked(eye.x, eye.z)
            } else if w.water.endless {
                surface.level()
            } else {
                continue;
            };
            // Above the surface by more than the lens: dry.
            if eye.y > height + near_half * 2.0 {
                continue;
            }
            if best.is_none_or(|(h, _)| height < h) {
                best = Some((height, w));
            }
        }
        if let Some((height, w)) = best {
            let [dr, dg, db] = w.water.deep_color;
            let [sr, sg, sb] = w.water.shallow_color;
            // Between the deep and shallow colours: blue-green, lit from
            // above in the shader.
            let mix = |d: f32, s: f32| d + (s - d) * 0.45;
            view.underwater = [
                mix(dr, sr),
                mix(dg, sg),
                mix(db, sb),
                underwater_extinction(w.water.clarity),
            ];
            view.eye[0] = height;
        }
        view.eye[1] = near_half;
        if caustics {
            // The nearest waters with caustics, nearest first.
            let mut lit: Vec<(f32, [f32; 4], [f32; 4])> = waters
                .iter()
                .filter(|w| w.water.caustics > 0.0)
                .map(|w| {
                    let surface = WaterSurface::new(&w.water, w.model, time);
                    let (min, max) = surface.bounds();
                    let closest = glam::Vec2::new(eye.x, eye.z).clamp(min, max);
                    let distance = closest.distance(glam::Vec2::new(eye.x, eye.z));
                    (
                        distance,
                        [min.x, min.y, max.x, max.y],
                        [
                            surface.level(),
                            w.water.caustics.clamp(0.0, 2.0),
                            w.water.clarity.max(0.05),
                            0.0,
                        ],
                    )
                })
                .collect();
            lit.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (i, (_, rect, level)) in lit.into_iter().take(MAX_CAUSTIC_WATERS).enumerate() {
                view.rects[i] = rect;
                view.levels[i] = level;
                view.eye[2] = (i + 1) as f32;
            }
        }
        view
    }

    /// Whether the camera is in some water (at least partly: at the
    /// waterline, part of the view is under).
    #[must_use]
    pub fn camera_in_water(&self) -> bool {
        self.underwater[3] > 0.0
    }
}

/// A water surface at one moment, for the CPU (physics, scripts): the same
/// Gerstner waves the GPU draws, so floating things ride the visible swell.
///
/// Built from a [`Water`], its entity's world transform and the time the
/// frame is drawn at; cheap to build (a few dozen flops) and to sample.
#[derive(Debug, Clone, Copy)]
pub struct WaterSurface {
    /// World → the surface's unit square (`|x|, |z| <= 0.5` inside).
    to_unit: Mat4,
    /// World height of the water at rest.
    level: f32,
    /// Highest a crest can reach above `level`.
    amplitude: f32,
    /// World XZ bounds of the rectangle (min, max), for quick rejection.
    bounds: [glam::Vec2; 2],
    /// Per wave: direction × k (xy), phase (z), k (w).
    wave_k: [[f32; 4]; WAVES],
    /// Per wave: amplitude and steepness × amplitude, as drawn by the grid.
    wave_amp: [[f32; 2]; WAVES],
    /// The same waves as lanes, for sampling all four at once: direction
    /// × k (x, z), phase, amplitude, and horizontal pull (x, z).
    lanes: WaveLanes,
    /// Buoyancy density (see [`Water::density`]).
    pub density: f32,
    /// Drag (see [`Water::drag`]).
    pub drag: f32,
}

/// [`WaterSurface`]'s waves, one per SIMD lane.
#[derive(Debug, Clone, Copy)]
struct WaveLanes {
    kx: glam::Vec4,
    kz: glam::Vec4,
    phase: glam::Vec4,
    amp: glam::Vec4,
    pull_x: glam::Vec4,
    pull_z: glam::Vec4,
}

impl WaveLanes {
    fn new(wave_k: &[[f32; 4]; WAVES], wave_amp: &[[f32; 2]; WAVES]) -> Self {
        let lane = |f: &dyn Fn(usize) -> f32| glam::Vec4::new(f(0), f(1), f(2), f(3));
        let pull = |i: usize| {
            if wave_k[i][3] > 0.0 {
                wave_amp[i][1] / wave_k[i][3]
            } else {
                0.0
            }
        };
        Self {
            kx: lane(&|i| wave_k[i][0]),
            kz: lane(&|i| wave_k[i][1]),
            phase: lane(&|i| wave_k[i][2]),
            amp: lane(&|i| wave_amp[i][0]),
            pull_x: lane(&|i| wave_k[i][0] * pull(i)),
            pull_z: lane(&|i| wave_k[i][1] * pull(i)),
        }
    }
}

impl WaterSurface {
    /// The surface of `water` on an entity at `model` (its world
    /// transform), at `time` seconds.
    #[must_use]
    pub fn new(water: &Water, model: Mat4, time: f32) -> Self {
        let size = glam::Vec2::from(water.size).max(glam::Vec2::splat(0.01));
        let unit = model * Mat4::from_scale(glam::Vec3::new(size.x, 1.0, size.y));
        let (wave_k, amps) = wave_constants(water, model, time);
        let wave_amp = amps.map(|a| [a[0], a[1]]);
        let amplitude = wave_amp.iter().map(|a| a[0]).sum();
        let corners = [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)]
            .map(|(x, z)| unit.transform_point3(glam::Vec3::new(x, 0.0, z)));
        let mut min = glam::Vec2::splat(f32::MAX);
        let mut max = glam::Vec2::splat(f32::MIN);
        for c in corners {
            min = min.min(glam::Vec2::new(c.x, c.z));
            max = max.max(glam::Vec2::new(c.x, c.z));
        }
        Self {
            to_unit: unit.inverse(),
            level: model.w_axis.y,
            amplitude,
            bounds: [min, max],
            wave_k,
            wave_amp,
            lanes: WaveLanes::new(&wave_k, &wave_amp),
            density: water.density.max(0.0),
            drag: water.drag.max(0.0),
        }
    }

    /// The world height of the water at rest.
    #[must_use]
    pub fn level(&self) -> f32 {
        self.level
    }

    /// The highest any crest reaches (world height).
    #[must_use]
    pub fn max_height(&self) -> f32 {
        self.level + self.amplitude
    }

    /// The rectangle's world XZ bounds, `(min, max)`.
    #[must_use]
    pub fn bounds(&self) -> (glam::Vec2, glam::Vec2) {
        (self.bounds[0], self.bounds[1])
    }

    /// Whether the world point `(x, z)` is over the water's rectangle.
    #[inline]
    #[must_use]
    pub fn contains(&self, x: f32, z: f32) -> bool {
        if x < self.bounds[0].x
            || z < self.bounds[0].y
            || x > self.bounds[1].x
            || z > self.bounds[1].y
        {
            return false;
        }
        let p = self
            .to_unit
            .transform_point3(glam::Vec3::new(x, self.level, z));
        p.x.abs() <= 0.5 && p.z.abs() <= 0.5
    }

    /// Horizontal displacement and height of the waves at rest point `p`
    /// (all four waves at once).
    fn displace(&self, p: glam::Vec2) -> (glam::Vec2, f32) {
        let w = &self.lanes;
        let (s, c) = sin_cos4(w.kx * p.x + w.kz * p.y - w.phase);
        (
            glam::Vec2::new(w.pull_x.dot(c), w.pull_z.dot(c)),
            w.amp.dot(s),
        )
    }

    /// The water's surface height at world `(x, z)`, or `None` outside its
    /// rectangle.
    ///
    /// Gerstner waves move water sideways as well as up, so the surface
    /// above `(x, z)` comes from a nearby rest point; one fixed-point step
    /// finds it (within millimetres for steep waves).
    #[inline]
    #[must_use]
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.contains(x, z).then(|| self.height_unchecked(x, z))
    }

    /// Length of [`pack`](Self::pack)'s array.
    pub const PACKED_LEN: usize = 14 + 6 * WAVES;

    /// The surface as plain numbers, for a script runner in another
    /// process to sample (Oberon's bridge): the world→unit rows that give
    /// unit x and z (8), level, max crest height, bounds min x, min z,
    /// max x, max z, then per wave `kx, kz, phase, k, amplitude, q·amp`.
    /// Height at `(x, z)`: `offset = Σ (kx, kz)·qa·cos(kx·x + kz·z − phase)/k`,
    /// then `level + Σ amp·sin(...)` at `(x, z) − offset`.
    #[must_use]
    pub fn pack(&self) -> [f32; Self::PACKED_LEN] {
        let mut out = [0.0; Self::PACKED_LEN];
        let rows = self.to_unit.transpose();
        out[0..4].copy_from_slice(&rows.x_axis.to_array());
        out[4..8].copy_from_slice(&rows.z_axis.to_array());
        out[8] = self.level;
        out[9] = self.max_height();
        out[10..12].copy_from_slice(&self.bounds[0].to_array());
        out[12..14].copy_from_slice(&self.bounds[1].to_array());
        for (i, (k, a)) in self.wave_k.iter().zip(&self.wave_amp).enumerate() {
            let w = 14 + 6 * i;
            out[w..w + 4].copy_from_slice(k);
            out[w + 4..w + 6].copy_from_slice(a);
        }
        out
    }

    /// [`height_at`](Self::height_at) without the rectangle test.
    #[inline]
    #[must_use]
    pub fn height_unchecked(&self, x: f32, z: f32) -> f32 {
        if self.amplitude == 0.0 {
            return self.level;
        }
        let p = glam::Vec2::new(x, z);
        let (offset, _) = self.displace(p);
        let (_, height) = self.displace(p - offset);
        self.level + height
    }
}

/// `sin_cos` of four angles to about 1e-5, without branches or libm (the
/// CPU water sampler's hot loop): reduce to ±π, evaluate the half angle's
/// Taylor series (±π/2), then double it.
#[inline]
fn sin_cos4(x: glam::Vec4) -> (glam::Vec4, glam::Vec4) {
    use std::f32::consts::{FRAC_1_PI, TAU};
    // Round to whole turns by adding and removing 1.5·2²³ (two adds; a
    // vector `round` is four libm calls without SSE4.1). Exact while
    // |turns| < 2²², i.e. wave phases below ~26 million radians.
    const MAGIC: f32 = 12_582_912.0;
    let turns = (x * (0.5 * FRAC_1_PI) + MAGIC) - MAGIC;
    let h = (x - turns * TAU) * 0.5;
    let h2 = h * h;
    let s = h
        * (1.0
            + h2 * (-1.0 / 6.0
                + h2 * (1.0 / 120.0 + h2 * (-1.0 / 5040.0 + h2 * (1.0 / 362_880.0)))));
    let c = 1.0
        + h2 * (-0.5
            + h2 * (1.0 / 24.0
                + h2 * (-1.0 / 720.0 + h2 * (1.0 / 40320.0 - h2 * (1.0 / 3_628_800.0)))));
    (2.0 * s * c, c * c - s * s)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Texels per side of the simulated ripple field (see [`RippleMap`]).
pub const RIPPLE_FIELD_SIZE: u32 = 256;

/// The simulated ripples around the camera at one moment, ready for the
/// GPU: rings spreading where things hit the water, wakes behind what
/// moves through it, and the foam they whip up. Physics simulates them
/// (`automata_physics`); the draw list carries a snapshot
/// ([`crate::DrawList::set_ripples`]) and the renderer uploads it only
/// when it changed.
///
/// The field is a square of `size`² texels, `cell` metres each, centred
/// on `center` (world XZ). Each texel holds, as half floats, the ripple
/// height (m), its slope along X and Z, and foam (0..1).
#[derive(Debug, Clone)]
pub struct RippleMap {
    /// World XZ of the field's centre.
    pub center: glam::Vec2,
    /// Metres per texel.
    pub cell: f32,
    /// Texels per side.
    pub size: u32,
    /// `size`² RGBA half floats, rows along +Z: height, dh/dx, dh/dz, foam.
    pub texels: std::sync::Arc<[u16]>,
    /// Bumped by the simulation whenever the texels change.
    pub version: u64,
}

impl RippleMap {
    /// Packs a simulated field: `height` (m) and `foam` (0..1), `size`²
    /// values each in rows along +Z, slopes taken by central differences.
    ///
    /// # Panics
    /// If `height` or `foam` doesn't hold `size`² values.
    #[must_use]
    pub fn new(
        center: glam::Vec2,
        cell: f32,
        size: u32,
        height: &[f32],
        foam: &[f32],
        version: u64,
    ) -> Self {
        let n = size as usize;
        assert!(height.len() == n * n && foam.len() == n * n, "field size");
        let inv = 0.5 / cell.max(1e-6);
        let mut texels = Vec::with_capacity(n * n * 4);
        for z in 0..n {
            let row = z * n;
            let up = if z > 0 { row - n } else { row };
            let down = if z + 1 < n { row + n } else { row };
            for x in 0..n {
                let left = row + x.saturating_sub(1);
                let right = row + (x + 1).min(n - 1);
                let dx = (height[right] - height[left]) * inv;
                let dz = (height[down + x] - height[up + x]) * inv;
                texels.extend_from_slice(&[
                    f16_bits(height[row + x].clamp(-16.0, 16.0)),
                    f16_bits(dx.clamp(-16.0, 16.0)),
                    f16_bits(dz.clamp(-16.0, 16.0)),
                    f16_bits(foam[row + x].clamp(0.0, 1.0)),
                ]);
            }
        }
        Self {
            center,
            cell,
            size,
            texels: texels.into(),
            version,
        }
    }

    /// World XZ of the field's corner (texel 0's outer corner) and its
    /// width in metres.
    #[must_use]
    pub fn extent(&self) -> (glam::Vec2, f32) {
        #[allow(clippy::cast_precision_loss)] // a few hundred texels
        let width = self.size as f32 * self.cell;
        (self.center - glam::Vec2::splat(width * 0.5), width)
    }
}

/// `v` as IEEE half-float bits, rounded to nearest (inputs are clamped to
/// a small range by the callers; out-of-range values become infinity).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)] // bit fields
pub(crate) fn f16_bits(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    if v.is_nan() {
        return 0x7e00;
    }
    let exp = ((x >> 23) & 0xff) as i32 - 127 + 15;
    let mant = x & 0x007f_ffff;
    if exp >= 31 {
        return sign | 0x7c00;
    }
    if exp <= 0 {
        if exp < -10 {
            return sign;
        }
        // Subnormal: the implicit 1 joins the mantissa, shifted down.
        let m = (mant | 0x0080_0000) >> (14 - exp) as u32;
        let round = ((mant | 0x0080_0000) >> (13 - exp) as u32) & 1;
        return sign | (m + round) as u16;
    }
    let bits = ((exp as u32) << 10) | (mant >> 13);
    let round = (mant >> 12) & 1;
    // A carry out of the mantissa correctly bumps the exponent.
    sign | (bits + round) as u16
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
/// How far the endless skirt reaches, in multiples of the surface size.
const SKIRT_REACH: f32 = 60.0;

/// The unit grid ([`GRID_CELLS`]² quads over `[-0.5, 0.5]` on XZ),
/// followed by an "endless" skirt: a ring of quads from the grid's edge
/// out to `SKIRT_REACH` sharing the edge vertices (no cracks). Draw
/// [`grid_index_count`] indices for a bounded surface, all of them for an
/// endless one.
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
    // The grid's boundary walked once around, and an outer ring pushed out
    // radially to the skirt's square.
    let mut boundary = Vec::with_capacity((4 * n) as usize);
    boundary.extend(0..n); // z = 0 row, +x
    boundary.extend((0..n).map(|z| z * row + n)); // x = n column, +z
    boundary.extend((0..n).map(|x| n * row + (n - x))); // z = n row, -x
    boundary.extend((0..n).map(|z| (n - z) * row)); // x = 0 column, -z
    let outer_start = u32::try_from(vertices.len()).expect("grid fits u32 indices");
    for &b in &boundary {
        let p = vertices[b as usize].position;
        let (x, z) = (p[0] * 2.0 * SKIRT_REACH, p[2] * 2.0 * SKIRT_REACH);
        vertices.push(Vertex::new(
            [x, 0.0, z],
            [0.0, 1.0, 0.0],
            [x + 0.5, z + 0.5],
        ));
    }
    let count = 4 * n;
    for k in 0..count {
        let next = (k + 1) % count;
        let (a, b) = (boundary[k as usize], boundary[next as usize]);
        let (oa, ob) = (outer_start + k, outer_start + next);
        indices.extend_from_slice(&[a, b, ob, a, ob, oa]);
    }
    MeshData::new(vertices, indices)
}

/// Indices of the bounded grid (without the endless skirt).
pub(crate) fn grid_index_count() -> u32 {
    GRID_CELLS * GRID_CELLS * 6
}

#[cfg(test)]
mod tests {
    #[test]
    fn half_floats_round_trip_the_ripple_range() {
        // Exact values, signs, zero, rounding, subnormals and overflow.
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(65504.0), 0x7bff);
        assert_eq!(f16_bits(1e6), 0x7c00);
        assert_eq!(f16_bits(f32::NAN), 0x7e00);
        assert_eq!(f16_bits(2.0_f32.powi(-24)), 1);
        assert_eq!(f16_bits(1e-9), 0);
        // 1 + 2^-11 is halfway between halves: rounds up (to the odd one).
        assert_eq!(f16_bits(1.0 + 2.0_f32.powi(-11)), 0x3c01);
        // Back again, for a value inside the ripple range.
        let bits = f16_bits(0.123);
        let back = f32::from(bits & 0x3ff) / 1024.0 + 1.0;
        let back = back * 2.0_f32.powi(i32::from((bits >> 10) & 0x1f) - 15);
        assert!((back - 0.123).abs() < 1e-4, "{back}");
    }

    #[test]
    fn ripple_maps_pack_heights_slopes_and_foam() {
        let n = 4usize;
        // A ramp rising 0.1 m per texel along +X, foam in one corner.
        let height: Vec<f32> = (0..n * n).map(|i| (i % n) as f32 * 0.1).collect();
        let mut foam = vec![0.0; n * n];
        foam[0] = 0.5;
        let map = RippleMap::new(glam::Vec2::new(10.0, -4.0), 0.5, 4, &height, &foam, 7);
        assert_eq!(map.texels.len(), n * n * 4);
        let (corner, width) = map.extent();
        assert_eq!((corner, width), (glam::Vec2::new(9.0, -5.0), 2.0));
        // Texel (1, 2): height 0.1, slope 0.1 m per 0.5 m along X, flat in Z.
        let t = &map.texels[(2 * n + 1) * 4..(2 * n + 2) * 4];
        assert_eq!(t[0], f16_bits(0.1));
        assert_eq!(t[1], f16_bits(0.2));
        assert_eq!(t[2], 0);
        assert_eq!(map.texels[3], f16_bits(0.5));
        assert_eq!(map.version, 7);
    }

    use super::*;

    #[test]
    fn grid_covers_the_unit_square_facing_up() {
        let grid = grid_mesh();
        assert_eq!(
            grid.vertices.len(),
            ((GRID_CELLS + 1) * (GRID_CELLS + 1) + 4 * GRID_CELLS) as usize
        );
        assert_eq!(
            grid.indices.len(),
            (grid_index_count() + GRID_CELLS * 4 * 6) as usize
        );
        // The skirt's triangles wind like the grid's.
        let normal_y = |t: &[u32]| {
            let [a, b, c] =
                [t[0], t[1], t[2]].map(|i| glam::Vec3::from(grid.vertices[i as usize].position));
            (b - a).cross(c - a).y
        };
        let grid_sign = normal_y(&grid.indices[..3]).signum();
        for t in grid.indices[grid_index_count() as usize..].chunks(3) {
            assert_eq!(normal_y(t).signum(), grid_sign, "{t:?}");
        }
        let corner = |i: u32| glam::Vec3::from(grid.vertices[i as usize].position);
        // First triangle winds counter-clockwise seen from +Y.
        let [v0, v1, v2] = [0, 1, 2].map(|k| corner(grid.indices[k]));
        assert!((v1 - v0).cross(v2 - v0).y > 0.0);
        let grid_vertices = ((GRID_CELLS + 1) * (GRID_CELLS + 1)) as usize;
        let xs: Vec<f32> = grid.vertices[..grid_vertices]
            .iter()
            .map(|v| v.position[0])
            .collect();
        assert_eq!(xs.iter().copied().fold(f32::MAX, f32::min), -0.5);
        assert_eq!(xs.iter().copied().fold(f32::MIN, f32::max), 0.5);
        // The skirt reaches far past the surface.
        let far = grid.vertices[grid_vertices..]
            .iter()
            .map(|v| v.position[0].abs())
            .fold(0.0, f32::max);
        assert!((far - SKIRT_REACH).abs() < 1e-3);
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
    fn cpu_surface_matches_the_drawn_waves() {
        let water = Water {
            size: [40.0, 40.0],
            wave_height: 0.5,
            wave_length: 8.0,
            ..Water::default()
        };
        let model = Mat4::from_translation(glam::Vec3::new(3.0, 1.5, -2.0));
        let t = 2.25;
        let surface = WaterSurface::new(&water, model, t);
        let u = WaterUniform::new(&WaterDraw { water, model }, false, t);
        // The GPU displaces each rest point by the waves; the CPU height at
        // the displaced point must match the displaced height.
        for (x, z) in [(0.0, 0.0), (5.0, -3.0), (-7.5, 9.0), (12.0, 4.0)] {
            let mut offset = glam::Vec3::ZERO;
            for i in 0..WAVES {
                let [kx, kz, phase, k] = u.wave_k[i];
                let [amp, qamp, _, _] = u.wave_amp[i];
                let theta = kx * x + kz * z - phase;
                offset += glam::Vec3::new(
                    qamp * kx / k * theta.cos(),
                    amp * theta.sin(),
                    qamp * kz / k * theta.cos(),
                );
            }
            let (wx, wz) = (x + offset.x, z + offset.z);
            let h = surface.height_at(wx, wz).expect("inside");
            assert!(
                (h - (1.5 + offset.y)).abs() < 0.02,
                "at ({x}, {z}): {h} vs {}",
                1.5 + offset.y
            );
        }
        // Crests reach at most the wave height (less where the grid fades
        // the shortest wave out).
        assert!(surface.max_height() > 1.5 + 0.4 && surface.max_height() <= 2.0 + 1e-4);
    }

    #[test]
    fn cpu_surface_respects_the_rectangle_and_rotation() {
        let water = Water {
            size: [10.0, 2.0],
            wave_height: 0.0,
            ..Water::default()
        };
        let model = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let s = WaterSurface::new(&water, model, 0.0);
        // Rotated 90°: the long side now runs along Z.
        assert_eq!(s.height_at(0.0, 4.5), Some(0.0));
        assert_eq!(s.height_at(4.5, 0.0), None);
        let (min, max) = s.bounds();
        assert!((min.x + 1.0).abs() < 1e-4 && (max.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn partial_scene_data_falls_back_to_defaults() {
        let w: Water = ron::from_str("(wave_height: 0.5)").unwrap();
        assert_eq!(w.wave_height, 0.5);
        assert_eq!(w.size, Water::default().size);
    }

    /// The packed form evaluates to the same heights (it's what Oberon's
    /// script runner samples; `bridge.py` mirrors this loop).
    #[test]
    fn packed_surface_evaluates_like_the_surface() {
        let water = Water {
            size: [30.0, 12.0],
            wave_height: 0.8,
            wave_length: 7.0,
            ..Water::default()
        };
        let model = Mat4::from_rotation_translation(
            glam::Quat::from_rotation_y(0.6),
            glam::Vec3::new(3.0, -1.0, 2.0),
        );
        let surface = WaterSurface::new(&water, model, 4.2);
        let p = surface.pack();
        let height = |x: f32, z: f32| -> Option<f32> {
            let ux = p[0] * x + p[1] * p[8] + p[2] * z + p[3];
            let uz = p[4] * x + p[5] * p[8] + p[6] * z + p[7];
            if ux.abs() > 0.5 || uz.abs() > 0.5 {
                return None;
            }
            let waves = |x: f32, z: f32| {
                let (mut ox, mut oz, mut h) = (0.0, 0.0, 0.0);
                for w in p[14..].chunks(6) {
                    let (s, c) = (w[0] * x + w[1] * z - w[2]).sin_cos();
                    ox += w[0] * w[5] * c / w[3];
                    oz += w[1] * w[5] * c / w[3];
                    h += w[4] * s;
                }
                (ox, oz, h)
            };
            let (ox, oz, _) = waves(x, z);
            Some(p[8] + waves(x - ox, z - oz).2)
        };
        for &(x, z) in &[(3.0, 2.0), (8.0, 1.0), (-4.0, 5.0), (40.0, 0.0), (3.0, 9.0)] {
            match (surface.height_at(x, z), height(x, z)) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-4, "{a} vs {b} at {x},{z}"),
                (None, None) => {}
                other => panic!("inside/outside disagree at {x},{z}: {other:?}"),
            }
        }
        assert!((p[9] - surface.max_height()).abs() < 1e-6);
    }

    #[test]
    fn sin_cos4_matches_libm() {
        let mut x = -300.0_f32;
        while x < 300.0 {
            let v = glam::Vec4::new(x, x + 0.4, -x * 0.7, x * 1.3);
            let (s, c) = super::sin_cos4(v);
            for i in 0..4 {
                let (rs, rc) = v[i].sin_cos();
                assert!(
                    (s[i] - rs).abs() < 4e-5 && (c[i] - rc).abs() < 4e-5,
                    "{}: {} {} vs {rs} {rc}",
                    v[i],
                    s[i],
                    c[i]
                );
            }
            x += 0.0137;
        }
    }
}
