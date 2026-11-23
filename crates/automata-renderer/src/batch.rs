//! Culling and instancing: which meshes a pass draws, and in how few draw
//! calls.
//!
//! Every frame each pass (the camera's, and each sun shadow cascade) gets
//! a list of [`Batch`]es. A mesh outside the pass's view isn't in it.
//! Meshes past their draw distance aren't in any. The rest are grouped by
//! geometry (and, for passes that shade, material): a group draws as one
//! instanced call, with each copy's model matrix in a per-instance vertex
//! buffer ([`Instance`]). Content keys ([`crate::mesh::content_key`],
//! [`crate::material::content_key`]) decide the groups, so ten thousand
//! trees from one model, or cubes made one by one, draw as one call per
//! part. Skinned meshes draw one by one with their joint palettes.
//!
//! The instance data goes in a vertex buffer bound at an offset (not
//! `first_instance`), so it works on GLES and WebGL too.

use std::collections::HashMap;

use glam::{Mat4, Vec4};

use crate::draw::MeshDraw;
use crate::mesh::Mesh;
use crate::shadow::Sphere;

/// One copy of a mesh: its model matrix and how much of it shows.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Instance {
    /// Model matrix columns.
    pub model: [[f32; 4]; 4],
    /// `x`: the fade (1 solid, toward 0 dithered away); the rest unused.
    pub params: [f32; 4],
}

impl Instance {
    fn new(model: &Mat4, fade: f32) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            params: [fade, 0.0, 0.0, 0.0],
        }
    }
}

/// Bytes per [`Instance`].
pub(crate) const INSTANCE_SIZE: u64 = std::mem::size_of::<Instance>() as u64;

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    5 => Float32x4,
    6 => Float32x4,
    7 => Float32x4,
    8 => Float32x4,
    9 => Float32x4
];

/// The per-instance vertex buffer layout (slot 1 of plain mesh pipelines).
pub(crate) fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: INSTANCE_SIZE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &INSTANCE_ATTRIBUTES,
    }
}

/// How a batch draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BatchKind {
    /// Instanced, solid.
    Plain,
    /// Instanced, some copies dithering away (the fading pipeline).
    Faded,
    /// One skinned mesh with its joint palette.
    Skinned,
    /// Grass blades (`grass.rs`): instanced, bent by `vs_grass`. The
    /// batch's `draw` indexes the frame's grass materials, not its meshes.
    Grass,
}

/// One draw call.
#[derive(Debug, Clone)]
pub(crate) struct Batch {
    pub kind: BatchKind,
    /// The draw whose material (and for skinned ones, bind group) it uses.
    pub draw: usize,
    /// The mesh it draws: the draw's own, or one of its simpler levels.
    pub mesh: Mesh,
    /// Its first instance in the frame's instance buffer, and how many.
    pub first: u32,
    pub count: u32,
    /// See-through: drawn after the solid ones, and not in the depth
    /// prepass.
    pub transparent: bool,
}

/// What the camera sees of one draw this frame: which level of detail, and
/// how faded. `None` (in [`choose`]) is past its draw distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Seen {
    pub level: usize,
    pub fade: f32,
}

/// The level and fade of `draw`, whose bounds are `sphere`, seen from
/// `eye`; `None` past its draw distance.
pub(crate) fn choose(
    draw: &MeshDraw,
    sphere: Sphere,
    eye: glam::Vec3,
    lod_scale: f32,
) -> Option<Seen> {
    let Some(lod) = &draw.lod else {
        return Some(Seen {
            level: 0,
            fade: 1.0,
        });
    };
    let distance = sphere.center.distance(eye);
    let fade = lod.fade_at(distance);
    if fade <= 0.0 {
        return None;
    }
    // Skinned meshes have no simpler levels (they'd need their own skins).
    let level = if draw.joints.is_some() {
        0
    } else {
        lod.level_at(distance, lod_scale)
    };
    Some(Seen { level, fade })
}

/// The mesh `draw` draws at `level`.
pub(crate) fn mesh_at(draw: &MeshDraw, level: usize) -> &Mesh {
    match (level, &draw.lod) {
        (0, _) | (_, None) => &draw.mesh,
        (k, Some(lod)) => lod.levels.get(k - 1).map_or(&draw.mesh, |(mesh, _)| mesh),
    }
}

/// A view volume as six inward-facing planes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// The frustum of a view-projection matrix (wgpu's 0..1 depth).
    pub(crate) fn new(view_proj: &Mat4) -> Self {
        let (r0, r1, r2, r3) = (
            view_proj.row(0),
            view_proj.row(1),
            view_proj.row(2),
            view_proj.row(3),
        );
        let planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|p| {
            let len = p.truncate().length();
            if len > 0.0 {
                p / len
            } else {
                p
            }
        });
        Self { planes }
    }

    /// Whether any of `sphere` is inside.
    pub(crate) fn sees(&self, sphere: Sphere) -> bool {
        let c = sphere.center.extend(1.0);
        self.planes.iter().all(|p| p.dot(c) >= -sphere.radius)
    }
}

/// How a pass groups its draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassKind {
    /// Shades: groups by material too, keeps fades and see-through order.
    Shaded,
    /// Depth only (shadows): groups by geometry alone, draws solid.
    Depth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GroupKey {
    mesh: u64,
    material: u64,
    kind: BatchKind,
    transparent: bool,
}

/// Groups `entries` (draw index and what's seen of it) into batches,
/// appending their instances to `instances` (batch by batch).
pub(crate) fn batch(
    draws: &[MeshDraw],
    entries: impl IntoIterator<Item = (usize, Seen)>,
    pass: PassKind,
    instances: &mut Vec<Instance>,
) -> Vec<Batch> {
    let mut index: HashMap<GroupKey, usize> = HashMap::new();
    let mut groups: Vec<(Batch, Vec<Instance>)> = Vec::new();
    for (i, seen) in entries {
        let draw = &draws[i];
        let shaded = pass == PassKind::Shaded;
        let transparent = shaded && draw.material.inner.params.color[3] < 1.0;
        let mesh = mesh_at(draw, seen.level);
        if draw.joints.is_some() {
            groups.push((
                Batch {
                    kind: BatchKind::Skinned,
                    draw: i,
                    mesh: mesh.clone(),
                    first: 0,
                    count: 1,
                    transparent,
                },
                Vec::new(),
            ));
            continue;
        }
        let kind = if shaded && seen.fade < 1.0 {
            BatchKind::Faded
        } else {
            BatchKind::Plain
        };
        let key = GroupKey {
            mesh: mesh.inner.key,
            material: if shaded { draw.material.inner.key } else { 0 },
            kind,
            transparent,
        };
        let slot = *index.entry(key).or_insert_with(|| {
            groups.push((
                Batch {
                    kind,
                    draw: i,
                    mesh: mesh.clone(),
                    first: 0,
                    count: 0,
                    transparent,
                },
                Vec::new(),
            ));
            groups.len() - 1
        });
        groups[slot].1.push(Instance::new(&draw.model, seen.fade));
    }
    // Solid first; see-through after, in the order they were drawn.
    groups.sort_by_key(|(b, _)| b.transparent);
    groups
        .into_iter()
        .map(|(mut b, copies)| {
            if b.kind != BatchKind::Skinned {
                b.first = u32::try_from(instances.len()).unwrap_or(u32::MAX);
                b.count = u32::try_from(copies.len()).unwrap_or(u32::MAX);
                instances.extend(copies);
            }
            b
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn sphere(x: f32, y: f32, z: f32, radius: f32) -> Sphere {
        Sphere {
            center: Vec3::new(x, y, z),
            radius,
        }
    }

    #[test]
    fn the_frustum_keeps_what_the_camera_can_see() {
        let camera = crate::Camera3D::new(Vec3::ZERO, Vec3::NEG_Z);
        let f = Frustum::new(&camera.view_proj((1600, 900)));
        assert!(f.sees(sphere(0.0, 0.0, -10.0, 0.5)), "straight ahead");
        assert!(!f.sees(sphere(0.0, 0.0, 10.0, 0.5)), "behind");
        assert!(
            !f.sees(sphere(40.0, 0.0, -10.0, 0.5)),
            "far off to the side"
        );
        assert!(
            f.sees(sphere(0.0, 0.0, 2.0, 2.5)),
            "behind but big enough to reach in"
        );
        assert!(
            !f.sees(sphere(0.0, 0.0, -5000.0, 1.0)),
            "past the far plane"
        );
        // Just clear of the right edge, then just touching it (the side
        // plane slants, so the gap across it is the x gap times its cos).
        let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan() * 1600.0 / 900.0;
        let (edge, cos) = (10.0 * tan, tan.atan().cos());
        assert!(!f.sees(sphere(edge + 1.05 / cos, 0.0, -10.0, 1.0)));
        assert!(f.sees(sphere(edge + 0.95 / cos, 0.0, -10.0, 1.0)));
    }

    #[test]
    fn levels_and_fades_follow_the_distance() {
        let lod = crate::MeshLod {
            levels: Vec::new(),
            draw_distance: 100.0,
        };
        assert!((lod.fade_at(50.0) - 1.0).abs() < 1e-6);
        assert!((lod.fade_at(90.0) - 1.0).abs() < 1e-6);
        assert!((lod.fade_at(95.0) - 0.5).abs() < 1e-6);
        assert!(lod.fade_at(100.0).abs() < 1e-6);
        let forever = crate::MeshLod::default();
        assert!((forever.fade_at(1e6) - 1.0).abs() < 1e-6);
    }
}
