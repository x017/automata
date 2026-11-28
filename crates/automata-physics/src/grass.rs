//! What pushes grass aside: characters and moving bodies.

use automata_renderer::{DrawList, GrassPusher};
use automata_scene::{GlobalTransform, SceneWorld};
use glam::Vec3;

use crate::{BodyType, CharacterController, Collider3D, RigidBody, Shape3D};

/// Adds a [`GrassPusher`] to `draw` for every character and moving
/// (non-static) body: a circle at the bottom of its collider as wide as
/// the collider, so grass bends away from where it stands. (The renderer
/// uses the ones nearest the camera.)
pub fn record_grass_pushers(world: &SceneWorld, draw: &mut DrawList) {
    if draw.grass_fields().is_empty() {
        return;
    }
    let mut query = world.query::<(
        &GlobalTransform,
        Option<&CharacterController>,
        Option<&RigidBody>,
        Option<&Collider3D>,
    )>();
    for (global, character, body, collider) in &mut query {
        let moves = body.is_some_and(|b| b.enabled && b.body_type != BodyType::Static);
        if character.is_none() && !moves {
            continue;
        }
        let (scale, rotation, translation) = global.matrix().to_scale_rotation_translation();
        let (center, radius, half_height) = match collider {
            Some(c) if !c.enabled || c.sensor => continue,
            Some(c) => {
                let s = scale.abs();
                let (radius, half_height) = match &c.shape {
                    Shape3D::Ball { radius } => {
                        (radius * s.max_element(), radius * s.max_element())
                    }
                    Shape3D::Cuboid { extents } => (
                        (extents.x * s.x).max(extents.z * s.z) * 0.5,
                        extents.y * s.y * 0.5,
                    ),
                    Shape3D::Capsule {
                        half_height,
                        radius,
                    } => (
                        radius * s.x.max(s.z),
                        half_height * s.y + radius * s.x.max(s.z),
                    ),
                    // Model shapes: about a metre across, around the origin.
                    Shape3D::Mesh { .. } | Shape3D::ConvexHull { .. } => (0.5, 0.5),
                };
                (
                    translation + rotation * (Vec3::from(c.offset) * scale),
                    radius,
                    half_height,
                )
            }
            None => (translation, 0.4, 0.0),
        };
        draw.push_grass(GrassPusher {
            position: center - Vec3::Y * half_height,
            radius: radius.max(0.2),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_scene::{Grass, Scene, Transform};

    #[test]
    fn characters_and_moving_bodies_push_grass_but_walls_do_not() {
        let mut scene = Scene::new();
        scene.spawn((Transform::default(), Grass::area(4.0, 4.0)));
        scene.spawn((
            Transform::from_position(Vec3::new(1.0, 1.0, 0.0)),
            CharacterController::new(),
            Collider3D::capsule(0.5, 0.3),
        ));
        scene.spawn((
            Transform::from_position(Vec3::new(-1.0, 0.5, 0.0)),
            RigidBody::dynamic(),
            Collider3D::ball(0.5),
        ));
        scene.spawn((
            Transform::from_position(Vec3::new(0.0, 0.5, 2.0)),
            RigidBody::fixed(),
            Collider3D::cuboid(1.0, 1.0, 1.0),
        ));
        automata_scene::propagate_transforms(scene.world_mut());
        let mut draw = DrawList::new();
        automata_scene::record_draw_list(scene.world(), &mut draw);
        record_grass_pushers(scene.world(), &mut draw);
        let pushers = draw.grass_pushers();
        assert_eq!(pushers.len(), 2);
        // The capsule's feet: 1.0 - (0.5 + 0.3).
        let feet = pushers.iter().find(|p| p.position.x > 0.0).unwrap();
        assert!((feet.position.y - 0.2).abs() < 1e-5, "{}", feet.position);
        assert!((feet.radius - 0.3).abs() < 1e-5);
        let ball = pushers.iter().find(|p| p.position.x < 0.0).unwrap();
        assert!(ball.position.y.abs() < 1e-5);
        // No grass, nothing to push.
        let mut empty = DrawList::new();
        record_grass_pushers(scene.world(), &mut empty);
        assert_eq!(empty.grass_pushers().len(), 0);
    }
}
