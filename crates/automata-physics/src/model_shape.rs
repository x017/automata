//! Collision shapes built from model files: triangle meshes, convex hulls,
//! and the rule that picks between them and a plain box.

use automata_assets::Model;
use glam::Vec3;
use rapier3d as r3;

use crate::components::{BodyType, Shape3D};

/// How much of its bounding box a model must fill to get a box collider
/// instead of a mesh or hull (see [`auto_model_shape`]).
pub const BOX_FILL_THRESHOLD: f32 = 0.9;

/// A model's collision shape: the convex hull of its vertices, or its
/// triangles as a mesh. `None` when the model has nothing to build from.
pub(crate) fn build_model_shape(
    model: &Model,
    scale: Vec3,
    hull: bool,
) -> Option<r3::geometry::SharedShape> {
    let (points, tris) = model.triangles(scale);
    if hull {
        r3::geometry::SharedShape::convex_hull(&points)
    } else {
        if tris.is_empty() {
            return None;
        }
        // Merging duplicate vertices lets contacts slide across shared
        // edges instead of catching on internal ones.
        let flags = r3::geometry::TriMeshFlags::MERGE_DUPLICATE_VERTICES
            | r3::geometry::TriMeshFlags::FIX_INTERNAL_EDGES;
        r3::geometry::SharedShape::trimesh_with_flags(points, tris, flags).ok()
    }
}

/// How much of its bounding box a model fills, from `0.0` to `1.0`: the
/// smaller of its enclosed volume and its convex hull's volume, over the
/// box's volume. A crate scores about 1; an arch, a ramp or a tree scores
/// well below. Flat models (one side under 1% of the longest) count as
/// full, since a thin box fits them.
#[must_use]
pub fn box_fill(model: &Model) -> f32 {
    let size = model.size();
    let longest = size.max_element();
    if longest <= f32::EPSILON || size.min_element() < longest * 0.01 {
        return 1.0;
    }
    let box_volume = size.x * size.y * size.z;
    let (points, tris) = model.triangles(Vec3::ONE);
    // Divergence theorem over the triangles, measured from the box centre
    // so open or flipped parts skew it as little as possible.
    let c = model.center();
    let enclosed = tris
        .iter()
        .map(|t| {
            let [a, b, d] = t.map(|i| points[i as usize] - c);
            a.dot(b.cross(d)) / 6.0
        })
        .sum::<f32>()
        .abs();
    let hull = r3::geometry::SharedShape::convex_hull(&points)
        .map_or(box_volume, |s| s.mass_properties(1.0).mass());
    (enclosed.min(hull) / box_volume).clamp(0.0, 1.0)
}

/// The collider shape Add Collider / Fit Collider should give an entity
/// showing `model`, or `None` when a box fits it well enough (or the file
/// doesn't load), so the caller fits a box to the bounds.
///
/// Models that fill under [`BOX_FILL_THRESHOLD`] of their bounds get their
/// triangles ([`Shape3D::Mesh`]) when static (no body, or a static one)
/// and their convex hull when the body moves.
#[must_use]
pub fn auto_model_shape(model: &str, body: Option<BodyType>) -> Option<Shape3D> {
    let loaded = automata_assets::load(model).ok()?;
    if box_fill(&loaded) >= BOX_FILL_THRESHOLD {
        return None;
    }
    let model = model.replace('\\', "/");
    Some(match body {
        None | Some(BodyType::Static) => Shape3D::Mesh { model },
        Some(_) => Shape3D::ConvexHull { model },
    })
}

/// What's wrong with a collider shape on a body of type `body` (`None` for
/// no body), for the Inspector and export checks. `None` when it's fine.
#[must_use]
pub fn collider_problem(shape: &Shape3D, body: Option<BodyType>) -> Option<&'static str> {
    match (shape, body) {
        (Shape3D::Mesh { .. }, Some(BodyType::Dynamic)) => Some(
            "A Mesh collider can't be on a Dynamic body (it plays as its Convex Hull). \
             Use Convex Hull, or a Static or Kinematic body.",
        ),
        (Shape3D::Mesh { model } | Shape3D::ConvexHull { model }, _) if model.trim().is_empty() => {
            Some("The collider has no model to take its shape from.")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_assets::ModelPart;
    use automata_renderer::{MeshData, Vertex};

    fn model(points: &[[f32; 3]], tris: &[[u32; 3]]) -> Model {
        let vertices = points
            .iter()
            .map(|&p| Vertex::new(p, [0.0, 1.0, 0.0], [0.0, 0.0]))
            .collect();
        let indices = tris.iter().flatten().copied().collect();
        Model::new(
            "test.glb".into(),
            vec![ModelPart {
                name: "part".into(),
                mesh: MeshData::new(vertices, indices),
                color: [1.0; 4],
                texture: None,
                pixelated: false,
                skin: None,
                surface: automata_assets::PartSurface::default(),
                lods: Vec::new(),
            }],
        )
    }

    /// A closed, outward-wound unit cube.
    fn cube() -> Model {
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let t = [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [3, 6, 2],
            [3, 7, 6],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
        ];
        model(&p, &t)
    }

    /// A closed wedge (ramp): half its bounding box.
    fn wedge() -> Model {
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ];
        let t = [
            [0, 2, 1],
            [3, 4, 5],
            [0, 1, 4],
            [0, 4, 3],
            [1, 2, 5],
            [1, 5, 4],
            [0, 3, 5],
            [0, 5, 2],
        ];
        model(&p, &t)
    }

    #[test]
    fn boxes_fill_their_bounds_and_ramps_do_not() {
        assert!((box_fill(&cube()) - 1.0).abs() < 1e-4);
        assert!((box_fill(&wedge()) - 0.5).abs() < 1e-4);
        // Flat geometry boxes well whatever its triangles say.
        let quad = model(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]],
            &[[0, 1, 2]],
        );
        assert!((box_fill(&quad) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn built_shapes_match_the_request() {
        let ramp = wedge();
        let mesh = build_model_shape(&ramp, Vec3::splat(2.0), false).unwrap();
        let tri = mesh.as_trimesh().unwrap();
        assert_eq!(tri.indices().len(), 8);
        assert!(tri.vertices().iter().any(|v| (v.y - 2.0).abs() < 1e-6));
        let hull = build_model_shape(&ramp, Vec3::ONE, true).unwrap();
        assert!(hull.as_convex_polyhedron().is_some());
        assert!((hull.mass_properties(1.0).mass() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn mesh_on_a_dynamic_body_is_a_problem() {
        let mesh = Shape3D::Mesh {
            model: "m.glb".into(),
        };
        assert!(collider_problem(&mesh, Some(BodyType::Dynamic)).is_some());
        assert!(collider_problem(&mesh, None).is_none());
        assert!(collider_problem(&mesh, Some(BodyType::KinematicPosition)).is_none());
        let hull = Shape3D::ConvexHull {
            model: "m.glb".into(),
        };
        assert!(collider_problem(&hull, Some(BodyType::Dynamic)).is_none());
        let empty = Shape3D::ConvexHull { model: " ".into() };
        assert!(collider_problem(&empty, None).is_some());
    }
}
