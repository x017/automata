//! Grass: fields of thin blades that sway in the wind and bend away from
//! whatever walks through them.
//!
//! The scene builds each field once (blade placement is CPU work, see
//! `automata_scene::Grass`) and hands it over every frame as a
//! [`GrassField`]: chunks of [`GrassBlade`]s, a few metres across, each in
//! a random order. The renderer draws the chunks the camera sees with the
//! shared mesh pipeline, so grass gets the sun, shadows, point lights,
//! fog, ambient occlusion and every style preset like any other mesh. Its
//! own vertex stage (`vs_grass` in mesh.wgsl) bends each blade by the wind
//! ([`crate::Environment::wind`]) and by up to [`MAX_GRASS_PUSHERS`]
//! [`GrassPusher`]s, and thins the field out with distance: a chunk draws
//! only the first part of its (shuffled) blades, which is an even sample
//! of them, the rest shrinking away smoothly while the remaining ones
//! widen to fill in. Grass doesn't cast shadows.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};

use crate::mesh::{MeshData, Vertex};

/// The most [`GrassPusher`]s a frame bends grass with (the nearest to the
/// camera win). Must match `MAX_PUSHERS` in mesh.wgsl.
pub const MAX_GRASS_PUSHERS: usize = 8;

/// One blade, as the GPU sees it (the same layout as any instanced mesh
/// copy).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GrassBlade {
    /// World matrix columns of the unit blade ([`blade_mesh`]): its root's
    /// place, which way it faces, its lean, and its width (X) and height
    /// (Y) in metres.
    pub model: [[f32; 4]; 4],
    /// `x`: 1. `y`: its colour variant (0..1). `z`: its rank in its chunk
    /// (0..1, the order it thins out in). `w`: the field's draw distance
    /// (metres, 0 = always drawn at full density).
    pub params: [f32; 4],
}

impl GrassBlade {
    /// A blade rooted at `root` (world space), facing `yaw` radians around
    /// `up`, `width` × `height` metres, of colour variant `tint` (0..1),
    /// `rank` in its chunk (0..1), in a field drawn to `draw_distance`.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // one blade's every property
    pub fn new(
        root: Vec3,
        up: Vec3,
        yaw: f32,
        width: f32,
        height: f32,
        tint: f32,
        rank: f32,
        draw_distance: f32,
    ) -> Self {
        let up = up.try_normalize().unwrap_or(Vec3::Y);
        let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);
        let model =
            Mat4::from_scale_rotation_translation(Vec3::new(width, height, width), rotation, root);
        Self {
            model: model.to_cols_array_2d(),
            params: [
                1.0,
                tint.clamp(0.0, 1.0),
                rank.clamp(0.0, 1.0),
                draw_distance.max(0.0),
            ],
        }
    }

    /// Where it's rooted (world space).
    #[must_use]
    pub fn root(&self) -> Vec3 {
        Vec3::new(self.model[3][0], self.model[3][1], self.model[3][2])
    }
}

/// A few metres of a field: its bounds and its blades, shuffled so that
/// any first part of them is an even sample (the order distance thins
/// them in).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GrassChunk {
    /// Bounding sphere centre (world space).
    pub center: [f32; 3],
    /// Bounding sphere radius (metres), tips swaying included.
    pub radius: f32,
    /// The blades, in random order.
    pub blades: Vec<GrassBlade>,
}

impl GrassChunk {
    /// A chunk of `blades` (shuffle them first), its bounds fitted to
    /// them with `reach` metres to spare for blade height and sway.
    #[must_use]
    pub fn new(blades: Vec<GrassBlade>, reach: f32) -> Self {
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for blade in &blades {
            let root = blade.root();
            min = min.min(root);
            max = max.max(root);
        }
        if blades.is_empty() {
            (min, max) = (Vec3::ZERO, Vec3::ZERO);
        }
        let center = (min + max) * 0.5;
        Self {
            center: center.to_array(),
            radius: (max - min).length() * 0.5 + reach.max(0.0),
            blades,
        }
    }
}

/// How a field's blades look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrassLook {
    /// Colour at the root (linear RGB).
    pub base_color: [f32; 3],
    /// Colour at the tip (linear RGB).
    pub tip_color: [f32; 3],
    /// How much blades differ from each other in shade and hue (0..1).
    pub variation: f32,
}

impl Default for GrassLook {
    fn default() -> Self {
        Self {
            base_color: [0.07, 0.2, 0.03],
            tip_color: [0.42, 0.62, 0.16],
            variation: 0.3,
        }
    }
}

impl GrassLook {
    /// A key that's equal for equal looks (the renderer's material cache).
    #[must_use]
    pub fn key(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        for v in self
            .base_color
            .iter()
            .chain(&self.tip_color)
            .chain(std::iter::once(&self.variation))
        {
            h = (h ^ u64::from(v.to_bits())).wrapping_mul(0x0100_0000_01b3);
        }
        h
    }
}

/// One field for a frame: its chunks (shared, built once by the scene)
/// and its look.
#[derive(Debug, Clone, Default)]
pub struct GrassField {
    /// The field's chunks.
    pub chunks: Vec<Arc<GrassChunk>>,
    /// The blades' colours.
    pub look: GrassLook,
    /// Beyond this camera distance (metres) no blade is drawn; from a fifth
    /// of it the field thins out. 0 draws every blade at any distance.
    pub draw_distance: f32,
}

impl GrassField {
    /// How many blades the field has.
    #[must_use]
    pub fn blade_count(&self) -> usize {
        self.chunks.iter().map(|c| c.blades.len()).sum()
    }
}

/// Something grass bends away from: a character's feet, a rolling ball.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrassPusher {
    /// The bottom of it (world space): where it meets the ground.
    pub position: Vec3,
    /// How far around it blades bend (metres).
    pub radius: f32,
}

/// The fraction of a field's blades drawn at `distance` metres from the
/// camera: all of them up to a fifth of `draw_distance`, then fewer, down
/// to none at it. Must match `grass_keep` in mesh.wgsl.
#[must_use]
pub fn keep_at(distance: f32, draw_distance: f32) -> f32 {
    if draw_distance <= 0.0 {
        return 1.0;
    }
    let (start, end) = (0.2 * draw_distance, draw_distance);
    let t = ((distance - start) / (end - start)).clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    1.0 - s
}

/// How many of `count` blades a chunk whose nearest point is `distance`
/// metres from the camera draws (a few to spare: the shader shrinks the
/// ones just past the cut away smoothly).
#[must_use]
pub fn drawn_blades(count: usize, distance: f32, draw_distance: f32) -> usize {
    let keep = keep_at(distance.max(0.0), draw_distance);
    if keep >= 1.0 {
        return count;
    }
    if keep <= 0.0 {
        return 0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let n = ((keep + 0.1).min(1.0) * count as f32).ceil() as usize;
    n.min(count)
}

/// The palette texture's size: colour variants across, root to tip down.
pub(crate) const PALETTE_SIZE: (u32, u32) = (8, 16);

/// The blade colours as an sRGB RGBA8 texture: [`PALETTE_SIZE`], one column
/// per colour variant (darker and yellower to lusher), root at the top row
/// and tip at the bottom.
#[must_use]
pub(crate) fn palette(look: &GrassLook) -> Vec<u8> {
    let (cols, rows) = PALETTE_SIZE;
    let variation = look.variation.clamp(0.0, 1.0);
    let mut rgba = Vec::with_capacity((cols * rows * 4) as usize);
    for row in 0..rows {
        #[allow(clippy::cast_precision_loss)]
        let up = row as f32 / (rows - 1) as f32;
        // A soft curve: the root's colour stays dark for the first third.
        let up = up * up * (3.0 - 2.0 * up);
        for col in 0..cols {
            #[allow(clippy::cast_precision_loss)]
            let variant = col as f32 / (cols - 1) as f32 * 2.0 - 1.0;
            let mut c = [0.0; 3];
            for (k, value) in c.iter_mut().enumerate() {
                *value = look.base_color[k] + (look.tip_color[k] - look.base_color[k]) * up;
            }
            // Variants: brightness, plus a shift between dry (more red,
            // less blue) and lush.
            let bright = 1.0 + 0.45 * variation * variant;
            let dry = -0.35 * variation * variant.min(0.0);
            c[0] = c[0] * bright + dry * c[1] * 0.6;
            c[1] *= bright;
            c[2] *= bright * (1.0 - dry);
            for value in c {
                rgba.push(to_srgb8(value));
            }
            rgba.push(255);
        }
    }
    rgba
}

fn to_srgb8(linear: f32) -> u8 {
    let l = linear.clamp(0.0, 1.0);
    let s = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = (s * 255.0).round() as u8;
    byte
}

/// Chunks whose nearest point is closer to the camera than this (metres)
/// draw [`blade_mesh`]'s five triangles a blade; further ones just one.
pub const FULL_BLADE_DISTANCE: f32 = 12.0;

/// The unit blade: 1 wide at the root and 1 tall, tapering to a point,
/// curving a little toward +Z. One-sided: grass draws without back-face
/// culling, so both sides show (5 triangles a blade, not 10). UV `x` runs
/// across it and `y` from root (near 0) to tip (near 1). Its normals lean
/// mostly up, so a field shades like a soft surface instead of flickering
/// sheets.
#[must_use]
pub fn blade_mesh() -> MeshData {
    // (height, half width) of each row, root to just below the tip.
    const ROWS: [(f32, f32); 3] = [(0.0, 0.5), (0.4, 0.4), (0.75, 0.24)];
    let mut vertices = Vec::new();
    for (h, half) in ROWS {
        for x in [-half, half] {
            vertices.push(blade_vertex(x, h));
        }
    }
    vertices.push(blade_vertex(0.0, 1.0));
    let mut indices = Vec::new();
    for row in 0..2 {
        let (a, b) = (row * 2, row * 2 + 1);
        let (c, d) = (a + 2, b + 2);
        indices.extend([a, b, d, a, d, c]);
    }
    indices.extend([4, 5, 6]);
    MeshData::new(vertices, indices)
}

/// One vertex of a baked chunk (see [`bake`]): a blade's vertex with
/// everything the vertex shader needs, so a chunk draws without instancing
/// (thousands of seven-vertex instances waste most of a GPU's vertex
/// waves, and software rasterizers pay per instance).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(crate) struct GrassVertex {
    /// `xyz`: the unbent vertex's offset from the root (world space).
    /// `w`: how far up the blade it is (0 root, 1 tip).
    pub offset_h: [f32; 4],
    /// `xyz`: the blade's root (world space). `w`: its colour variant.
    pub root_variant: [f32; 4],
    /// `xyz`: the blade's up axis, as long as it's tall. `w`: its rank.
    pub up_rank: [f32; 4],
    /// `xyz`: the vertex normal (world space, unnormalized). `w`: the
    /// field's draw distance.
    pub normal_reach: [f32; 4],
}

/// Bytes per [`GrassVertex`].
pub(crate) const GRASS_VERTEX_SIZE: u64 = std::mem::size_of::<GrassVertex>() as u64;

const GRASS_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32x4
];

/// The vertex buffer layout of a baked chunk (the grass pipelines' only
/// buffer).
pub(crate) fn grass_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: GRASS_VERTEX_SIZE,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &GRASS_VERTEX_ATTRIBUTES,
    }
}

/// Vertices per blade in a baked chunk ([`blade_mesh`]'s).
pub(crate) const BLADE_VERTICES: u32 = 7;

/// A chunk's blades as one vertex buffer's worth of [`GrassVertex`]es,
/// [`BLADE_VERTICES`] a blade, in the chunk's order: drawing the first `n`
/// blades' indices ([`blade_indices`]) draws its first `n` blades.
pub(crate) fn bake(chunk: &GrassChunk) -> Vec<GrassVertex> {
    let mesh = blade_mesh();
    let mut out = Vec::with_capacity(chunk.blades.len() * mesh.vertices.len());
    for blade in &chunk.blades {
        let m = Mat4::from_cols_array_2d(&blade.model);
        let root = blade.root();
        let up = m.y_axis.truncate();
        let [_, variant, rank, reach] = blade.params;
        for v in &mesh.vertices {
            let offset = m.transform_vector3(Vec3::from(v.position));
            let normal = m.transform_vector3(Vec3::from(v.normal));
            out.push(GrassVertex {
                offset_h: [offset.x, offset.y, offset.z, v.position[1]],
                root_variant: [root.x, root.y, root.z, variant],
                up_rank: [up.x, up.y, up.z, rank],
                normal_reach: [normal.x, normal.y, normal.z, reach],
            });
        }
    }
    out
}

/// Indices drawing `blades` baked blades: [`blade_mesh`]'s five triangles
/// a blade, or with `far` one (root corners to tip) for blades too far
/// off for the difference to show.
pub(crate) fn blade_indices(blades: u32, far: bool) -> Vec<u32> {
    let pattern: Vec<u32> = if far {
        vec![0, 1, 6]
    } else {
        blade_mesh().indices
    };
    (0..blades)
        .flat_map(|b| pattern.iter().map(move |i| b * BLADE_VERTICES + i))
        .collect()
}

/// Indices per blade drawn up close and far off (see [`blade_indices`]).
pub(crate) const BLADE_INDICES: (u32, u32) = (15, 3);

/// A vertex of the unit blade at `x` across and `h` up.
fn blade_vertex(x: f32, h: f32) -> Vertex {
    Vertex {
        position: [x, h, 0.18 * h * h],
        normal: Vec3::new(0.0, 1.0, 0.3).normalize().to_array(),
        uv: [x + 0.5, 0.03 + 0.94 * h],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blade_is_five_triangles_and_far_away_one() {
        let mesh = blade_mesh();
        assert_eq!(BLADE_INDICES, (15, 3));
        assert_eq!(blade_indices(1, false), mesh.indices);
        // Far off: the root's corners and the tip.
        let far = blade_indices(2, true);
        assert_eq!(far, [0, 1, 6, 7, 8, 13]);
        assert!((mesh.vertices[6].position[1] - 1.0).abs() < 1e-6);
        assert_eq!(mesh.vertices.len(), 7);
        assert_eq!(mesh.indices.len(), 5 * 3);
        let top = mesh
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MIN, f32::max);
        assert!((top - 1.0).abs() < 1e-6);
        // Every normal points mostly up.
        assert!(mesh.vertices.iter().all(|v| v.normal[1] > 0.8));
        // UVs stay clear of the palette's wrapped edge rows.
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.uv[1] > 0.01 && v.uv[1] < 0.99));
    }

    #[test]
    fn every_triangle_faces_the_same_way() {
        let mesh = blade_mesh();
        let p = |i: u32| Vec3::from(mesh.vertices[i as usize].position);
        for indices in [mesh.indices.clone(), blade_indices(1, true)] {
            assert!(indices
                .chunks(3)
                .all(|t| (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).z > 0.0));
        }
    }

    #[test]
    fn baking_places_every_blade_vertex_in_the_world() {
        let blades = vec![
            GrassBlade::new(
                Vec3::new(1.0, 0.0, 2.0),
                Vec3::Y,
                0.0,
                0.1,
                0.5,
                0.25,
                0.0,
                30.0,
            ),
            GrassBlade::new(
                Vec3::new(-3.0, 1.0, 0.0),
                Vec3::Y,
                1.0,
                0.1,
                0.8,
                0.75,
                0.5,
                30.0,
            ),
        ];
        let baked = bake(&GrassChunk::new(blades, 1.0));
        assert_eq!(baked.len(), 2 * BLADE_VERTICES as usize);
        let tip = &baked[6];
        // The tip: 0.5 m up from the root (plus a little curve forward).
        assert!((tip.offset_h[1] - 0.5).abs() < 1e-5 && (tip.offset_h[3] - 1.0).abs() < 1e-6);
        assert_eq!(tip.root_variant, [1.0, 0.0, 2.0, 0.25]);
        assert!((Vec3::from_slice(&tip.up_rank[..3]).length() - 0.5).abs() < 1e-5);
        let second = &baked[7];
        assert_eq!(second.root_variant, [-3.0, 1.0, 0.0, 0.75]);
        assert!((second.up_rank[3] - 0.5).abs() < 1e-6);
        assert!((second.normal_reach[3] - 30.0).abs() < 1e-6);
        assert!(baked.iter().all(|v| v.normal_reach[1] > 0.0));
    }

    #[test]
    fn distance_thins_the_field_smoothly_to_nothing() {
        assert!((keep_at(0.0, 50.0) - 1.0).abs() < 1e-6);
        assert!((keep_at(10.0, 50.0) - 1.0).abs() < 1e-6);
        let mid = keep_at(30.0, 50.0);
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        assert!(keep_at(50.0, 50.0).abs() < 1e-6);
        assert!(keep_at(80.0, 50.0).abs() < 1e-6);
        assert!((keep_at(1e6, 0.0) - 1.0).abs() < 1e-6, "0 = no limit");
        assert_eq!(drawn_blades(1000, 5.0, 50.0), 1000);
        assert_eq!(drawn_blades(1000, 60.0, 50.0), 0);
        let some = drawn_blades(1000, 30.0, 50.0);
        assert!(some > 300 && some < 800, "{some}");
    }

    #[test]
    fn blades_stand_where_and_how_they_are_told() {
        let blade = GrassBlade::new(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::Y,
            0.0,
            0.1,
            0.5,
            0.2,
            0.7,
            40.0,
        );
        assert_eq!(blade.root(), Vec3::new(1.0, 2.0, 3.0));
        let m = Mat4::from_cols_array_2d(&blade.model);
        let tip = m.transform_point3(Vec3::Y);
        assert!((tip - Vec3::new(1.0, 2.5, 3.0)).length() < 1e-5);
        assert_eq!(blade.params, [1.0, 0.2, 0.7, 40.0]);
        // On a slope it leans with the ground.
        let tilted = GrassBlade::new(
            Vec3::ZERO,
            Vec3::new(1.0, 1.0, 0.0),
            0.0,
            0.1,
            1.0,
            0.0,
            0.0,
            0.0,
        );
        let tip = Mat4::from_cols_array_2d(&tilted.model).transform_point3(Vec3::Y);
        assert!(tip.x > 0.6 && tip.y > 0.6);
    }

    #[test]
    fn the_palette_runs_root_to_tip_and_varies_across() {
        let look = GrassLook {
            base_color: [0.0, 0.1, 0.0],
            tip_color: [0.0, 0.8, 0.0],
            variation: 1.0,
        };
        let px = palette(&look);
        let (w, h) = PALETTE_SIZE;
        assert_eq!(px.len(), (w * h * 4) as usize);
        let green = |col: u32, row: u32| px[((row * w + col) * 4 + 1) as usize];
        assert!(green(4, h - 1) > green(4, 0), "tips are lighter");
        assert!(green(w - 1, 8) > green(0, 8), "variants differ");
        let flat = palette(&GrassLook {
            variation: 0.0,
            ..look
        });
        let g = |col: u32| flat[((8 * w + col) * 4 + 1) as usize];
        assert_eq!(g(0), g(w - 1), "no variation, one colour");
    }

    #[test]
    fn chunks_fit_their_blades() {
        let blades = vec![
            GrassBlade::new(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::Y,
                0.0,
                0.1,
                0.5,
                0.0,
                0.0,
                0.0,
            ),
            GrassBlade::new(
                Vec3::new(4.0, 0.0, 0.0),
                Vec3::Y,
                0.0,
                0.1,
                0.5,
                0.0,
                0.5,
                0.0,
            ),
        ];
        let chunk = GrassChunk::new(blades, 1.0);
        assert_eq!(chunk.center, [2.0, 0.0, 0.0]);
        assert!((chunk.radius - 3.0).abs() < 1e-6);
        assert_eq!(GrassChunk::new(Vec::new(), 1.0).radius, 1.0);
    }
}
