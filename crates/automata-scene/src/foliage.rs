//! Grass and foliage: fields of blades and scattered models, painted onto
//! the ground.
//!
//! A [`Grass`] component grows instanced blades (drawn by the renderer,
//! swaying in the [`automata_renderer::Environment`] wind and bending away
//! from whatever pushes through them). A [`Scatter`] component draws its
//! entity's own model (rocks, flowers, bushes) many times instead of once.
//!
//! Both cover the same kind of ground: an optional rectangle (`area`,
//! centred on the entity) plus brush strokes ([`Dab`]s, in the entity's
//! space) that paint more in or, with a negative strength, erase. The
//! strokes are rasterised into half-metre cells, each with a coverage and
//! the plane of the surface it was painted on, and every cell grows its
//! own blades from its own seed, so painting one spot leaves the rest of
//! the field exactly as it was.
//!
//! What's grown is cached in the component (keyed by everything that
//! shapes it, including where the entity is), so a field is built once,
//! not every frame.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use automata_renderer::{DrawList, GrassBlade, GrassChunk, GrassField, GrassLook, MeshData};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use hecs::Entity;
use serde::{Deserialize, Serialize};

use crate::{GlobalTransform, MeshVisual, SceneWorld};

/// The size of a paint cell (metres): the grain strokes paint at.
pub const PAINT_CELL: f32 = 0.5;
/// The size of a grass chunk (metres): the grain the renderer culls and
/// thins blades at.
const CHUNK_SIZE: f32 = 8.0;
/// The most blades one [`Grass`] grows.
pub const MAX_BLADES: usize = 1_000_000;
/// The most copies one [`Scatter`] places.
pub const MAX_SCATTERED: usize = 20_000;
/// The largest `area` side (metres); bigger is clamped.
pub const MAX_AREA: f32 = 400.0;
/// Surfaces steeper than this (the normal's up component) grow nothing.
const STEEPEST: f32 = 0.3;

/// One dab of a paint brush: a soft disc on a surface, in its entity's
/// space. Positive `strength` paints in (up to that much cover), negative
/// erases (that much of what's there).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Dab {
    /// The disc's centre.
    pub position: [f32; 3],
    /// The surface's normal there (the disc lies in its plane).
    #[serde(default = "up")]
    pub normal: [f32; 3],
    /// The disc's radius (metres); it fades out over its outer half.
    pub radius: f32,
    /// -1..1: how much it paints in (or, negative, erases).
    #[serde(default = "one")]
    pub strength: f32,
}

fn up() -> [f32; 3] {
    [0.0, 1.0, 0.0]
}

fn one() -> f32 {
    1.0
}

impl Dab {
    /// A dab painting full cover at `position` on a surface facing
    /// `normal`.
    #[must_use]
    pub fn paint(position: Vec3, normal: Vec3, radius: f32) -> Self {
        Self {
            position: position.to_array(),
            normal: normal.to_array(),
            radius,
            strength: 1.0,
        }
    }

    /// A dab erasing everything at `position`.
    #[must_use]
    pub fn erase(position: Vec3, radius: f32) -> Self {
        Self {
            position: position.to_array(),
            normal: up(),
            radius,
            strength: -1.0,
        }
    }
}

/// A painted cell: how covered it is and the surface it was painted on.
#[derive(Debug, Clone, Copy)]
struct Cell {
    coverage: f32,
    point: Vec3,
    normal: Vec3,
}

impl Cell {
    const FLAT: Self = Self {
        coverage: 1.0,
        point: Vec3::ZERO,
        normal: Vec3::Y,
    };

    /// The height of its surface above `(x, z)`, or `None` where that's
    /// too steep to grow on.
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let n = self.normal;
        (n.y >= STEEPEST)
            .then(|| self.point.y - (n.x * (x - self.point.x) + n.z * (z - self.point.z)) / n.y)
    }
}

#[allow(clippy::cast_possible_truncation)] // clamped to the area
fn cell_of(v: f32) -> i32 {
    (v / PAINT_CELL).floor() as i32
}

#[allow(clippy::cast_precision_loss)]
fn cell_center(ix: i32, iz: i32) -> Vec2 {
    Vec2::new(
        (ix as f32 + 0.5) * PAINT_CELL,
        (iz as f32 + 0.5) * PAINT_CELL,
    )
}

/// Calls `f` with every covered cell of `area` and `strokes`, in a fixed
/// order.
fn for_each_cell(area: [f32; 2], strokes: &[Dab], mut visit: impl FnMut((i32, i32), Cell)) {
    let half = Vec2::new(
        area[0].clamp(0.0, MAX_AREA) * 0.5,
        area[1].clamp(0.0, MAX_AREA) * 0.5,
    );
    let in_area = |ix, iz| {
        let c = cell_center(ix, iz);
        half.x > 0.0 && half.y > 0.0 && c.x.abs() <= half.x && c.y.abs() <= half.y
    };
    let mut painted: HashMap<(i32, i32), Cell> = HashMap::new();
    for dab in strokes {
        let radius = dab.radius.clamp(0.01, MAX_AREA * 0.5);
        let centre = Vec3::from(dab.position);
        if !centre.is_finite() {
            continue;
        }
        let normal = Vec3::from(dab.normal).try_normalize().unwrap_or(Vec3::Y);
        let strength = dab.strength.clamp(-1.0, 1.0);
        for ix in cell_of(centre.x - radius)..=cell_of(centre.x + radius) {
            for iz in cell_of(centre.z - radius)..=cell_of(centre.z + radius) {
                let away = cell_center(ix, iz).distance(Vec2::new(centre.x, centre.z));
                if away > radius {
                    continue;
                }
                let falloff = 1.0 - smoothstep(0.5 * radius, radius, away);
                let cell = painted.entry((ix, iz)).or_insert_with(|| {
                    if in_area(ix, iz) {
                        Cell::FLAT
                    } else {
                        Cell {
                            coverage: 0.0,
                            point: centre,
                            normal,
                        }
                    }
                });
                if strength >= 0.0 {
                    if falloff * strength > cell.coverage {
                        *cell = Cell {
                            coverage: falloff * strength,
                            point: centre,
                            normal,
                        };
                    }
                } else {
                    cell.coverage *= 1.0 - falloff * -strength;
                }
            }
        }
    }
    if half.x > 0.0 && half.y > 0.0 {
        for ix in cell_of(-half.x)..=cell_of(half.x) {
            for iz in cell_of(-half.y)..=cell_of(half.y) {
                if in_area(ix, iz) && !painted.contains_key(&(ix, iz)) {
                    visit((ix, iz), Cell::FLAT);
                }
            }
        }
    }
    let mut painted: Vec<_> = painted
        .into_iter()
        .filter(|(_, c)| c.coverage > 0.01)
        .collect();
    painted.sort_unstable_by_key(|(k, _)| *k);
    for (key, cell) in painted {
        visit(key, cell);
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A small deterministic random source (xorshift64*).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Splitmix the seed so neighbouring cells don't start alike.
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Self((z ^ (z >> 31)) | 1)
    }

    fn cell(seed: u32, (ix, iz): (i32, i32)) -> Self {
        #[allow(clippy::cast_sign_loss)]
        let (x, z) = (u64::from(ix as u32), u64::from(iz as u32));
        Self::new(u64::from(seed) << 40 ^ x << 20 ^ z ^ z << 52)
    }

    /// A number in [0, 1).
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        #[allow(clippy::cast_precision_loss)] // 24 bits
        let v = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32;
        v / 16_777_216.0
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let j = ((self.next() * (i + 1) as f32) as usize).min(i);
            items.swap(i, j);
        }
    }
}

/// Calls `f(position, normal, coverage, rng)` for `density` points per
/// square metre over the painted ground (thinner where it's lightly
/// painted), up to `max` points, in the entity's space.
fn scatter_points(
    area: [f32; 2],
    strokes: &[Dab],
    density: f32,
    seed: u32,
    max: usize,
    mut f: impl FnMut(Vec3, Vec3, f32, &mut Rng),
) {
    let per_cell = density.max(0.0) * PAINT_CELL * PAINT_CELL;
    let mut placed = 0;
    for_each_cell(area, strokes, |key, cell| {
        if placed >= max {
            return;
        }
        let mut rng = Rng::cell(seed, key);
        let expected = per_cell * cell.coverage;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (expected + rng.next()).floor() as usize;
        let origin = cell_center(key.0, key.1) - Vec2::splat(PAINT_CELL * 0.5);
        for _ in 0..n.min(max - placed) {
            let (x, z) = (
                origin.x + rng.next() * PAINT_CELL,
                origin.y + rng.next() * PAINT_CELL,
            );
            if let Some(y) = cell.height_at(x, z) {
                f(Vec3::new(x, y, z), cell.normal, cell.coverage, &mut rng);
                placed += 1;
            }
        }
    });
}

/// A component's grown result, cached by a key of everything that shaped
/// it. Clones share it; it never compares or saves.
#[derive(Clone, Default)]
pub struct Grown<T>(Arc<Mutex<Slot<T>>>);

/// What [`Grown`] holds: the key it grew for and what grew.
type Slot<T> = Option<(u64, Arc<T>)>;

impl<T> Grown<T> {
    fn get_or(&self, key: u64, grow: impl FnOnce() -> T) -> Arc<T> {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*slot {
            Some((k, grown)) if *k == key => Arc::clone(grown),
            _ => {
                let grown = Arc::new(grow());
                *slot = Some((key, Arc::clone(&grown)));
                grown
            }
        }
    }
}

impl<T> std::fmt::Debug for Grown<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Grown")
    }
}

impl<T> PartialEq for Grown<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Hashes floats by their bits.
#[derive(Default)]
struct KeyHasher(std::collections::hash_map::DefaultHasher);

impl KeyHasher {
    fn floats(&mut self, values: &[f32]) -> &mut Self {
        for v in values {
            v.to_bits().hash(&mut self.0);
        }
        self
    }

    fn strokes(&mut self, area: [f32; 2], strokes: &[Dab]) -> &mut Self {
        self.floats(&area);
        strokes.len().hash(&mut self.0);
        for d in strokes {
            self.floats(&d.position)
                .floats(&d.normal)
                .floats(&[d.radius, d.strength]);
        }
        self
    }

    fn finish(&self) -> u64 {
        self.0.finish()
    }
}

/// A field of grass: a scene component (saved with the scene).
///
/// Blades grow `density` per square metre over the painted ground, `height`
/// metres tall (up to `height_variation` shorter, and shorter where the
/// paint is light), coloured from `base_color` at the root to `tip_color`
/// at the tip with `color_variation` between blades. Past `draw_distance`
/// metres from the camera they're gone, thinning out over the way there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grass {
    /// Blades per square metre where fully painted.
    pub density: f32,
    /// The tallest blades (metres).
    pub height: f32,
    /// 0..1: how much shorter some blades are.
    pub height_variation: f32,
    /// Blade width at the root (metres).
    pub width: f32,
    /// Colour at the root (linear RGB 0..1).
    pub base_color: [f32; 3],
    /// Colour at the tip.
    pub tip_color: [f32; 3],
    /// 0..1: how much blades' colours differ.
    pub color_variation: f32,
    /// Metres from the camera it's drawn to (0 = any distance).
    pub draw_distance: f32,
    /// Picks which random field grows.
    pub seed: u32,
    /// A filled rectangle centred on the entity, its size along the
    /// entity's X and Z (metres); zero for none.
    pub area: [f32; 2],
    /// Brush strokes, in the entity's space, in the order painted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Dab>,
    /// What it grew last (a cache; leave it to `..Default::default()`).
    #[doc(hidden)]
    #[serde(skip)]
    pub grown: Grown<Vec<Arc<GrassChunk>>>,
}

impl Default for Grass {
    fn default() -> Self {
        let look = GrassLook::default();
        Self {
            density: 60.0,
            height: 0.6,
            height_variation: 0.45,
            width: 0.07,
            base_color: look.base_color,
            tip_color: look.tip_color,
            color_variation: look.variation,
            draw_distance: 60.0,
            seed: 0,
            area: [0.0, 0.0],
            strokes: Vec::new(),
            grown: Grown::default(),
        }
    }
}

impl Grass {
    /// A `width` × `depth` metre field centred on the entity.
    #[must_use]
    pub fn area(width: f32, depth: f32) -> Self {
        Self {
            area: [width, depth],
            ..Self::default()
        }
    }

    /// How it looks, for the renderer.
    #[must_use]
    pub fn look(&self) -> GrassLook {
        GrassLook {
            base_color: self.base_color,
            tip_color: self.tip_color,
            variation: self.color_variation,
        }
    }

    /// Its blades for an entity at `model`, in chunks (grown on first use
    /// and whenever anything that shapes them changes).
    #[must_use]
    pub fn chunks(&self, model: Mat4) -> Arc<Vec<Arc<GrassChunk>>> {
        let key = KeyHasher::default()
            .floats(&model.to_cols_array())
            .floats(&[
                self.density,
                self.height,
                self.height_variation,
                self.width,
                self.draw_distance,
            ])
            .floats(&[f32::from_bits(self.seed)])
            .strokes(self.area, &self.strokes)
            .finish();
        self.grown.get_or(key, || self.grow(model))
    }

    /// How many blades it has at `model`.
    #[must_use]
    pub fn blade_count(&self, model: Mat4) -> usize {
        self.chunks(model).iter().map(|c| c.blades.len()).sum()
    }

    /// Grows its blades for an entity at `model`, chunked and shuffled.
    fn grow(&self, model: Mat4) -> Vec<Arc<GrassChunk>> {
        /// One blade before it's ranked in its chunk.
        struct Sprout {
            root: Vec3,
            up: Vec3,
            yaw: f32,
            width: f32,
            height: f32,
            tint: f32,
        }
        let normals = Mat3::from_mat4(model).inverse().transpose();
        let mut chunks: HashMap<(i32, i32), Vec<Sprout>> = HashMap::new();
        let (height, variation, width) = (
            self.height.max(0.01),
            self.height_variation.clamp(0.0, 1.0),
            self.width.max(0.002),
        );
        scatter_points(
            self.area,
            &self.strokes,
            self.density,
            self.seed,
            MAX_BLADES,
            |local, normal, coverage, rng| {
                let root = model.transform_point3(local);
                let ground = (normals * normal).try_normalize().unwrap_or(Vec3::Y);
                #[allow(clippy::cast_possible_truncation)]
                let key = (
                    (root.x / CHUNK_SIZE).floor() as i32,
                    (root.z / CHUNK_SIZE).floor() as i32,
                );
                chunks.entry(key).or_default().push(Sprout {
                    root,
                    // Half way between the ground's normal and straight up.
                    up: (ground + Vec3::Y).normalize_or(Vec3::Y),
                    yaw: rng.next() * std::f32::consts::TAU,
                    width: width * (0.75 + 0.5 * rng.next()),
                    height: height * (1.0 - variation * rng.next()) * (0.4 + 0.6 * coverage),
                    tint: rng.next(),
                });
            },
        );
        let mut keys: Vec<_> = chunks.keys().copied().collect();
        keys.sort_unstable();
        keys.into_iter()
            .filter_map(|key| {
                let mut sprouts = chunks.remove(&key)?;
                // A random order, so drawing the first so many thins the
                // chunk evenly.
                Rng::cell(self.seed ^ 0x5eed, key).shuffle(&mut sprouts);
                #[allow(clippy::cast_precision_loss)]
                let n = sprouts.len() as f32;
                let blades = sprouts
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        #[allow(clippy::cast_precision_loss)]
                        let rank = i as f32 / n;
                        GrassBlade::new(
                            s.root,
                            s.up,
                            s.yaw,
                            s.width,
                            s.height,
                            s.tint,
                            rank,
                            self.draw_distance,
                        )
                    })
                    .collect();
                Some(Arc::new(GrassChunk::new(blades, height * 1.2 + 0.3)))
            })
            .collect()
    }
}

/// Foliage scatter: a scene component (saved with the scene) that draws
/// its entity's model (its [`MeshVisual`]) at many spots on the painted
/// ground instead of once.
///
/// `density` copies per square metre, each turned at random, scaled between
/// `scale[0]` and `scale[1]`, and tilted `align` of the way from upright to
/// the ground's slope. Copies aren't solid: give rocks you can stand on
/// their own entities and colliders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scatter {
    /// Copies per square metre where fully painted.
    pub density: f32,
    /// The smallest and largest scale.
    pub scale: [f32; 2],
    /// 0 = upright, 1 = following the ground's slope.
    pub align: f32,
    /// Picks where the copies go.
    pub seed: u32,
    /// A filled rectangle centred on the entity (as [`Grass::area`]).
    pub area: [f32; 2],
    /// Brush strokes, in the entity's space (as [`Grass::strokes`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Dab>,
    /// What it grew last (a cache; leave it to `..Default::default()`).
    #[doc(hidden)]
    #[serde(skip)]
    pub grown: Grown<Vec<Mat4>>,
}

impl Default for Scatter {
    fn default() -> Self {
        Self {
            density: 0.2,
            scale: [0.7, 1.3],
            align: 0.3,
            seed: 0,
            area: [0.0, 0.0],
            strokes: Vec::new(),
            grown: Grown::default(),
        }
    }
}

impl Scatter {
    /// Copies over a `width` × `depth` metre rectangle centred on the
    /// entity.
    #[must_use]
    pub fn area(width: f32, depth: f32) -> Self {
        Self {
            area: [width, depth],
            ..Self::default()
        }
    }

    /// Where each copy goes, in the entity's space (multiply by its model
    /// matrix).
    #[must_use]
    pub fn placements(&self) -> Arc<Vec<Mat4>> {
        let key = KeyHasher::default()
            .floats(&[self.density, self.scale[0], self.scale[1], self.align])
            .floats(&[f32::from_bits(self.seed)])
            .strokes(self.area, &self.strokes)
            .finish();
        self.grown.get_or(key, || {
            let mut out = Vec::new();
            let align = self.align.clamp(0.0, 1.0);
            scatter_points(
                self.area,
                &self.strokes,
                self.density,
                self.seed,
                MAX_SCATTERED,
                |at, normal, _, rng| {
                    let tilt = Quat::from_rotation_arc(Vec3::Y, normal);
                    let rotation = Quat::IDENTITY.slerp(tilt, align)
                        * Quat::from_rotation_y(rng.next() * std::f32::consts::TAU);
                    let scale = self.scale[0] + (self.scale[1] - self.scale[0]) * rng.next();
                    out.push(Mat4::from_scale_rotation_translation(
                        Vec3::splat(scale.max(0.01)),
                        rotation,
                        at,
                    ));
                },
            );
            out
        })
    }
}

/// Hands every [`Grass`] field to the renderer.
pub(crate) fn record_grass(world: &SceneWorld, draw: &mut DrawList) {
    for (grass, global) in &mut world.query::<(&Grass, &GlobalTransform)>() {
        let chunks = grass.chunks(global.matrix());
        if chunks.is_empty() {
            continue;
        }
        draw.grass(GrassField {
            chunks: chunks.as_ref().clone(),
            look: grass.look(),
            draw_distance: grass.draw_distance,
        });
    }
}

/// Where a ray met a surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceHit {
    /// The entity whose mesh it hit.
    pub entity: Entity,
    /// The point hit (world space).
    pub point: Vec3,
    /// The surface's normal there, facing the ray.
    pub normal: Vec3,
    /// How far along the ray.
    pub distance: f32,
}

/// The nearest mesh surface (of entities with a [`MeshVisual`], scattered
/// ones aside) along the ray from `origin` in direction `dir`, within
/// `max_distance`: where a brush paints.
#[must_use]
pub fn raycast_surfaces(
    world: &SceneWorld,
    origin: Vec3,
    dir: Vec3,
    max_distance: f32,
) -> Option<SurfaceHit> {
    raycast_surfaces_except(world, origin, dir, max_distance, &|_| false)
}

/// [`raycast_surfaces`], passing through the entities `skip` picks.
pub(crate) fn raycast_surfaces_except(
    world: &SceneWorld,
    origin: Vec3,
    dir: Vec3,
    max_distance: f32,
    skip: &dyn Fn(Entity) -> bool,
) -> Option<SurfaceHit> {
    let dir = dir.try_normalize()?;
    let mut best: Option<SurfaceHit> = None;
    let mut query = world.query::<(Entity, &MeshVisual, &GlobalTransform, Option<&Scatter>)>();
    for (entity, visual, global, scatter) in &mut query {
        if scatter.is_some() || skip(entity) {
            continue;
        }
        let model = global.matrix();
        let det = model.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            continue;
        }
        let inverse = model.inverse();
        let (o, d) = (
            inverse.transform_point3(origin),
            inverse.transform_vector3(dir),
        );
        let meshes: Vec<&MeshData> = match &visual.model {
            Some(m) => m.parts.iter().map(|p| &p.mesh).collect(),
            None => vec![&visual.data],
        };
        let limit = best.map_or(max_distance, |b| b.distance);
        // `d` is the world direction in local space, so `t` is shared.
        if let Some((t, local_normal)) = meshes
            .into_iter()
            .filter_map(|mesh| ray_mesh(o, d, mesh, limit))
            .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            let normal = (Mat3::from_mat4(inverse).transpose() * local_normal)
                .try_normalize()
                .unwrap_or(Vec3::Y);
            best = Some(SurfaceHit {
                entity,
                point: origin + dir * t,
                normal: if normal.dot(dir) > 0.0 {
                    -normal
                } else {
                    normal
                },
                distance: t,
            });
        }
    }
    best
}

/// The nearest triangle of `mesh` the ray meets before `limit`: its
/// distance and (unnormalised, local) normal.
#[allow(clippy::many_single_char_names)] // the usual names in the maths
fn ray_mesh(o: Vec3, d: Vec3, mesh: &MeshData, limit: f32) -> Option<(f32, Vec3)> {
    // Its bounds first.
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in &mesh.vertices {
        let p = Vec3::from(v.position);
        min = min.min(p);
        max = max.max(p);
    }
    let inv = d.recip();
    let (t1, t2) = ((min - o) * inv, (max - o) * inv);
    let (near, far) = (t1.min(t2).max_element(), t1.max(t2).min_element());
    if !(far >= near.max(0.0) && near <= limit) {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    for tri in mesh.indices.chunks_exact(3) {
        let vertex = |i: u32| {
            mesh.vertices
                .get(i as usize)
                .map(|v| Vec3::from(v.position))
        };
        let (Some(a), Some(b), Some(c)) = (vertex(tri[0]), vertex(tri[1]), vertex(tri[2])) else {
            continue;
        };
        // Möller–Trumbore, both sides.
        let (e1, e2) = (b - a, c - a);
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = o - a;
        let u = s.dot(p) / det;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = d.dot(q) / det;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = e2.dot(q) / det;
        if t >= 0.0 && t <= best.map_or(limit, |b| b.0) {
            best = Some((t, e1.cross(e2)));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeshVisual, Scene, Transform};

    #[test]
    fn an_area_grows_its_density_evenly_and_the_same_every_time() {
        let grass = Grass {
            density: 40.0,
            ..Grass::area(10.0, 10.0)
        };
        let n = grass.blade_count(Mat4::IDENTITY);
        assert!((3600..4400).contains(&n), "about 4000 blades, got {n}");
        let again = Grass {
            density: 40.0,
            ..Grass::area(10.0, 10.0)
        };
        let (a, b) = (again.chunks(Mat4::IDENTITY), grass.chunks(Mat4::IDENTITY));
        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].blades, b[0].blades);
        // All on the ground, inside the area, at most `height` tall.
        for blade in a.iter().flat_map(|c| &c.blades) {
            let root = blade.root();
            assert!(root.y.abs() < 1e-5 && root.x.abs() <= 5.0 && root.z.abs() <= 5.0);
            let height = Vec3::from_slice(&blade.model[1][..3]).length();
            assert!(height <= grass.height + 1e-4, "{height}");
        }
        // Chunked by 8 m: a 10 m field straddling the origin spans four.
        assert_eq!(a.len(), 4);
        // Ranks run evenly through each chunk.
        let ranks: Vec<f32> = a[0].blades.iter().map(|b| b.params[2]).collect();
        assert!(ranks.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn the_cache_regrows_only_when_something_changes() {
        let mut grass = Grass::area(4.0, 4.0);
        let first = grass.chunks(Mat4::IDENTITY);
        assert!(Arc::ptr_eq(&first, &grass.chunks(Mat4::IDENTITY)));
        // Clones share it.
        let copy = grass.clone();
        assert!(Arc::ptr_eq(&first, &copy.chunks(Mat4::IDENTITY)));
        let moved = Mat4::from_translation(Vec3::X);
        assert!(!Arc::ptr_eq(&first, &grass.chunks(moved)));
        grass.height = 1.0;
        assert!(!Arc::ptr_eq(&first, &grass.chunks(Mat4::IDENTITY)));
    }

    #[test]
    fn strokes_paint_on_their_surface_and_erase() {
        let slope = Vec3::new(0.0, 1.0, -0.5).normalize();
        let mut grass = Grass {
            strokes: vec![Dab::paint(Vec3::new(20.0, 3.0, 0.0), slope, 2.0)],
            ..Grass::default()
        };
        let painted = grass.blade_count(Mat4::IDENTITY);
        assert!(painted > 200, "a disc of grass ({painted})");
        for blade in grass.chunks(Mat4::IDENTITY).iter().flat_map(|c| &c.blades) {
            let root = blade.root();
            assert!(root.distance(Vec3::new(20.0, 3.0, 0.0)) < 2.6, "{root}");
            // On the slope's plane: y rises with z.
            let expected = 3.0 + (root.z) * 0.5;
            assert!((root.y - expected).abs() < 1e-3, "{root} vs {expected}");
        }
        // Erasing the middle leaves a ring.
        grass
            .strokes
            .push(Dab::erase(Vec3::new(20.0, 3.0, 0.0), 1.0));
        let ring = grass.blade_count(Mat4::IDENTITY);
        assert!(
            ring < painted * 9 / 10 && ring > painted / 4,
            "{ring} of {painted}"
        );
        assert!(grass
            .chunks(Mat4::IDENTITY)
            .iter()
            .flat_map(|c| &c.blades)
            .all(|b| b.root().distance(Vec3::new(20.0, 3.0, 0.0)) > 0.4));
        // Erasing part of an area takes it out; painting elsewhere leaves
        // the rest of the field as it was.
        let mut field = Grass::area(6.0, 6.0);
        let before = field.chunks(Mat4::IDENTITY);
        field.strokes.push(Dab::erase(Vec3::ZERO, 1.5));
        assert!(field.blade_count(Mat4::IDENTITY) < before.iter().map(|c| c.blades.len()).sum());
        let far: Vec<Vec3> = before
            .iter()
            .flat_map(|c| &c.blades)
            .map(GrassBlade::root)
            .filter(|r| r.length() > 2.5)
            .collect();
        let after: Vec<Vec3> = field
            .chunks(Mat4::IDENTITY)
            .iter()
            .flat_map(|c| &c.blades)
            .map(GrassBlade::root)
            .collect();
        assert!(far.iter().all(|r| after.contains(r)));
    }

    #[test]
    fn steep_walls_grow_nothing() {
        let grass = Grass {
            strokes: vec![Dab::paint(Vec3::ZERO, Vec3::X, 2.0)],
            ..Grass::default()
        };
        assert_eq!(grass.blade_count(Mat4::IDENTITY), 0);
    }

    #[test]
    fn scatter_places_copies_on_the_ground_within_its_scale() {
        let scatter = Scatter {
            density: 1.0,
            ..Scatter::area(10.0, 10.0)
        };
        let spots = scatter.placements();
        assert!((80..120).contains(&spots.len()), "{}", spots.len());
        for m in spots.iter() {
            let (scale, _, at) = m.to_scale_rotation_translation();
            assert!(at.y.abs() < 1e-5);
            assert!(scale.x >= 0.7 - 1e-4 && scale.x <= 1.3 + 1e-4);
        }
    }

    #[test]
    fn rays_hit_the_nearest_mesh_and_skip_scattered_ones() {
        let mut scene = Scene::new();
        let ground = scene.spawn((
            Transform::from_position(Vec3::new(0.0, -1.0, 0.0))
                .with_scale(Vec3::new(10.0, 1.0, 10.0)),
            MeshVisual::cube(1.0, [1.0; 4]),
        ));
        let rock = scene.spawn((
            Transform::from_position(Vec3::new(3.0, 0.0, 0.0)),
            MeshVisual::cube(1.0, [1.0; 4]),
            Scatter::default(),
        ));
        crate::propagate_transforms(scene.world_mut());
        let world = scene.world();
        let hit = raycast_surfaces(world, Vec3::new(3.0, 5.0, 0.0), -Vec3::Y, 100.0).unwrap();
        assert_eq!(hit.entity, ground);
        assert_ne!(hit.entity, rock);
        assert!((hit.point.y + 0.5).abs() < 1e-4, "{}", hit.point);
        assert!(hit.normal.y > 0.99);
        assert!(raycast_surfaces(world, Vec3::new(30.0, 5.0, 0.0), -Vec3::Y, 100.0).is_none());
        assert!(raycast_surfaces(world, Vec3::new(3.0, 5.0, 0.0), -Vec3::Y, 2.0).is_none());
    }

    #[test]
    fn grass_records_one_field_per_component() {
        let mut scene = Scene::new();
        scene.spawn((Transform::default(), Grass::area(4.0, 4.0)));
        scene.spawn((
            Transform::from_position(Vec3::new(50.0, 0.0, 0.0)),
            Grass::area(2.0, 2.0),
        ));
        crate::propagate_transforms(scene.world_mut());
        let mut draw = DrawList::new();
        crate::record_draw_list(scene.world(), &mut draw);
        assert_eq!(draw.grass_fields().len(), 2);
        // Query order isn't spawn order once transforms are propagated, so
        // tell the fields apart by where they grew.
        let far = |field: &GrassField| field.chunks.iter().all(|c| c.center[0] > 48.0);
        let near = |field: &GrassField| field.chunks.iter().all(|c| c.center[0] < 3.0);
        let fields = draw.grass_fields();
        assert_eq!(fields.iter().filter(|f| far(f)).count(), 1);
        assert_eq!(fields.iter().filter(|f| near(f)).count(), 1);
    }
}
