//! Mesh geometry: vertex data, CPU-side mesh building, GPU mesh handles.

use std::sync::Arc;

use glam::Vec3;

/// A 3D vertex: position, normal, and texture coordinates.
///
/// The memory layout matches the mesh pipeline's vertex buffer layout.
#[repr(C)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    bytemuck::Pod,
    bytemuck::Zeroable,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Vertex {
    /// Position in model space.
    pub position: [f32; 3],
    /// Surface normal (normalized).
    pub normal: [f32; 3],
    /// Texture coordinates.
    pub uv: [f32; 2],
}

/// How a skinned vertex follows its skeleton: up to four joints (indices
/// into the draw's joint palette) and their weights, which sum to 1.
///
/// A skinned mesh carries one per vertex beside its [`Vertex`] data; see
/// [`crate::Renderer::create_skinned_mesh`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinVertex {
    /// Joint indices, each below [`crate::MAX_JOINTS`].
    pub joints: [u16; 4],
    /// Joint weights.
    pub weights: [f32; 4],
}

impl Vertex {
    /// Creates a vertex.
    #[must_use]
    pub const fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Self {
        Self {
            position,
            normal,
            uv,
        }
    }
}

/// CPU-side mesh geometry, ready to upload via
/// [`Renderer::create_mesh`](crate::Renderer::create_mesh).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshData {
    /// Vertex data.
    pub vertices: Vec<Vertex>,
    /// Triangle indices (counter-clockwise front faces).
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Creates mesh data from vertices and indices.
    #[must_use]
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self { vertices, indices }
    }

    /// A quad centered at the origin, lying in the XY plane facing +Z.
    #[must_use]
    pub fn quad(size: f32) -> Self {
        let h = size / 2.0;
        let n = [0.0, 0.0, 1.0];
        Self::new(
            vec![
                Vertex::new([-h, -h, 0.0], n, [0.0, 1.0]),
                Vertex::new([h, -h, 0.0], n, [1.0, 1.0]),
                Vertex::new([h, h, 0.0], n, [1.0, 0.0]),
                Vertex::new([-h, h, 0.0], n, [0.0, 0.0]),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    /// A cube centered at the origin with per-face normals and UVs.
    ///
    /// # Panics
    ///
    /// Panics if the vertex count exceeds `u32::MAX` (impossible for this
    /// fixed 24-vertex mesh).
    #[must_use]
    pub fn cube(size: f32) -> Self {
        let h = size / 2.0;
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);

        // Each face: right × up == normal, giving counter-clockwise
        // outward-facing triangles.
        let faces: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y, Vec3::Z),         // front (+Z)
            (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z), // back (-Z)
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),     // right (+X)
            (Vec3::Z, Vec3::Y, Vec3::NEG_X),     // left (-X)
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),     // top (+Y)
            (Vec3::X, Vec3::Z, Vec3::NEG_Y),     // bottom (-Y)
        ];

        for (right, up, normal) in faces {
            let base = u32::try_from(vertices.len()).expect("cube has 24 vertices");
            let center = normal * h;
            for (ru, uu, uv) in [
                (-1.0, -1.0, [0.0, 1.0]),
                (1.0, -1.0, [1.0, 1.0]),
                (1.0, 1.0, [1.0, 0.0]),
                (-1.0, 1.0, [0.0, 0.0]),
            ] {
                let p = center + right * (ru * h) + up * (uu * h);
                vertices.push(Vertex::new(p.into(), normal.into(), uv));
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        Self::new(vertices, indices)
    }

    /// A UV sphere `size` across (diameter), centered at the origin: 48
    /// segments around, 24 rings top to bottom, smooth normals, U around the
    /// equator and V from the top pole down.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // small loop counters
    pub fn sphere(size: f32) -> Self {
        const SEGMENTS: u32 = 48;
        const RINGS: u32 = 24;
        let r = size / 2.0;
        let mut vertices = Vec::with_capacity(((SEGMENTS + 1) * (RINGS + 1)) as usize);
        for ring in 0..=RINGS {
            let v = ring as f32 / RINGS as f32;
            let (sin_t, cos_t) = (v * std::f32::consts::PI).sin_cos();
            for seg in 0..=SEGMENTS {
                let u = seg as f32 / SEGMENTS as f32;
                let (sin_p, cos_p) = (u * std::f32::consts::TAU).sin_cos();
                // u = 0 faces +Z and runs counter-clockwise seen from above.
                let n = Vec3::new(sin_t * sin_p, cos_t, sin_t * cos_p);
                vertices.push(Vertex::new((n * r).into(), n.into(), [u, v]));
            }
        }
        let row = SEGMENTS + 1;
        let mut indices = Vec::with_capacity((SEGMENTS * RINGS * 6) as usize);
        for ring in 0..RINGS {
            for seg in 0..SEGMENTS {
                let a = ring * row + seg;
                let b = a + row;
                // Counter-clockwise from outside; the pole rows' degenerate
                // triangles are skipped.
                if ring != 0 {
                    indices.extend_from_slice(&[a, b, a + 1]);
                }
                if ring != RINGS - 1 {
                    indices.extend_from_slice(&[a + 1, b, b + 1]);
                }
            }
        }
        Self::new(vertices, indices)
    }
}

/// A mesh uploaded to the GPU.
///
/// Cheap to clone: the underlying buffers are reference-counted.
#[derive(Debug, Clone)]
pub struct Mesh {
    pub(crate) inner: Arc<MeshInner>,
}

impl Mesh {
    /// The number of indices in the mesh's index buffer.
    #[must_use]
    pub fn index_count(&self) -> u32 {
        self.inner.index_count
    }

    /// Whether the mesh has joints and weights (draw it with
    /// [`crate::DrawList::skinned_mesh`]).
    #[must_use]
    pub fn is_skinned(&self) -> bool {
        self.inner.skin_buffer.is_some()
    }
}

/// GPU-resident mesh data.
#[derive(Debug)]
pub(crate) struct MeshInner {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    /// Joints and weights per vertex ([`SkinVertex`]), for skinned meshes.
    pub skin_buffer: Option<wgpu::Buffer>,
    /// Radius of the sphere around the origin containing all vertices;
    /// used to fit the shadow map around the scene.
    pub bounding_radius: f32,
    /// The vertices' bounding box `(min, max)` in model space (the bind
    /// pose for skinned meshes): where a glowing mesh's light sits.
    pub bounds: (glam::Vec3, glam::Vec3),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spheres_face_outward_with_unit_normals() {
        let m = MeshData::sphere(2.0);
        for v in &m.vertices {
            let p = Vec3::from(v.position);
            assert!((p.length() - 1.0).abs() < 1e-5);
            assert!((Vec3::from(v.normal) - p).length() < 1e-5);
        }
        for t in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(m.vertices[t[i] as usize].position));
            let face = (b - a).cross(c - a);
            assert!(face.length() > 1e-8, "no degenerate triangles");
            assert!(face.dot(a + b + c) > 0.0, "counter-clockwise from outside");
        }
    }

    #[test]
    fn quad_has_two_triangles_facing_forward() {
        let quad = MeshData::quad(2.0);
        assert_eq!(quad.vertices.len(), 4);
        assert_eq!(quad.indices.len(), 6);
        assert!(quad
            .vertices
            .iter()
            .all(|v| (v.normal[2] - 1.0).abs() < f32::EPSILON
                && v.normal[0].abs() < f32::EPSILON
                && v.normal[1].abs() < f32::EPSILON));
    }

    #[test]
    fn cube_geometry_counts() {
        let cube = MeshData::cube(1.0);
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.indices.len(), 36);
    }

    #[test]
    fn cube_triangles_face_outward() {
        let cube = MeshData::cube(2.0);
        for triangle in cube.indices.chunks_exact(3) {
            let [a, b, c] = [
                Vec3::from(cube.vertices[triangle[0] as usize].position),
                Vec3::from(cube.vertices[triangle[1] as usize].position),
                Vec3::from(cube.vertices[triangle[2] as usize].position),
            ];
            let winding_normal = (b - a).cross(c - a);
            let face_normal = Vec3::from(cube.vertices[triangle[0] as usize].normal);
            assert!(
                winding_normal.dot(face_normal) > 0.0,
                "triangle {triangle:?} faces inward"
            );
        }
    }

    #[test]
    fn cube_indices_are_in_bounds() {
        let cube = MeshData::cube(1.0);
        let count = u32::try_from(cube.vertices.len()).unwrap();
        assert!(cube.indices.iter().all(|&i| i < count));
    }
}
