//! 2D physics: solid tilemaps and colliders without a rigid body.

use automata_physics::{
    CharacterController, CharacterVelocity, Collider2D, PhysicsWorld2D, RigidBody,
};
use automata_scene::{Scene, Tilemap, Transform};
use glam::Vec3;

fn step(world: &mut PhysicsWorld2D, scene: &mut Scene, frames: usize) {
    for _ in 0..frames {
        world.sync_from_scene(scene.world_mut());
        world.move_characters(scene.world_mut(), 1.0 / 60.0);
        world.step(scene.world_mut(), 1.0 / 60.0);
    }
}

fn character(scene: &mut Scene, at: Vec3) -> automata_scene::Entity {
    scene.spawn((
        Transform::from_position(at),
        RigidBody::kinematic_position(),
        Collider2D::cuboid(0.8, 0.9),
        CharacterController::new(),
        CharacterVelocity::new(Vec3::new(0.0, -6.0, 0.0)),
    ))
}

#[test]
fn characters_stand_on_painted_tiles_and_walls_stop_them() {
    let mut scene = Scene::new();
    // A floor with a wall at the right: the grid starts at (-2, 0).
    let map = Tilemap::from_rows(&["..........#", "..........#", "###########"], &[('#', 1)]);
    scene.spawn((Transform::from_position(Vec3::new(-2.0, 0.0, 0.0)), map));
    let player = character(&mut scene, Vec3::new(0.0, 4.0, 0.0));
    let mut world = PhysicsWorld2D::new();
    step(&mut world, &mut scene, 90);
    let y = scene
        .world()
        .get::<&Transform>(player)
        .expect("t")
        .position
        .y;
    // The floor's top is y = 1; the box is 0.9 tall.
    assert!((y - 1.45).abs() < 0.1, "stands on the floor: {y}");
    assert!(world.grounded(player));

    scene
        .world_mut()
        .insert_one(player, CharacterVelocity::new(Vec3::new(6.0, -6.0, 0.0)))
        .expect("insert");
    step(&mut world, &mut scene, 120);
    let x = scene
        .world()
        .get::<&Transform>(player)
        .expect("t")
        .position
        .x;
    // The wall's left face is x = -2 + 10 = 8; half the box is 0.4.
    assert!((x - 7.6).abs() < 0.1, "stopped by the wall: {x}");
}

#[test]
fn painting_a_tile_rebuilds_the_colliders_and_moved_characters_follow() {
    let mut scene = Scene::new();
    let map_entity = scene.spawn((
        Transform::default(),
        Tilemap::from_rows(&["...", "#.."], &[('#', 1)]),
    ));
    let player = character(&mut scene, Vec3::new(1.5, 4.0, 0.0));
    let mut world = PhysicsWorld2D::new();
    step(&mut world, &mut scene, 90);
    let fell = scene
        .world()
        .get::<&Transform>(player)
        .expect("t")
        .position
        .y;
    assert!(fell < -1.0, "no tile under it yet: {fell}");

    // Teleport it back up (scripts respawn characters by moving them).
    scene
        .world_mut()
        .get::<&mut Transform>(player)
        .expect("t")
        .position = Vec3::new(1.5, 4.0, 0.0);
    scene
        .world_mut()
        .get::<&mut Tilemap>(map_entity)
        .expect("map")
        .set(1, 1, 1);
    step(&mut world, &mut scene, 90);
    let y = scene
        .world()
        .get::<&Transform>(player)
        .expect("t")
        .position
        .y;
    assert!((y - 2.45).abs() < 0.1, "lands on the new tile: {y}");
}

#[test]
fn a_collider_without_a_body_is_fixed_ground() {
    let mut scene = Scene::new();
    scene.spawn((Transform::default(), Collider2D::cuboid(10.0, 1.0)));
    let player = character(&mut scene, Vec3::new(0.0, 3.0, 0.0));
    let mut world = PhysicsWorld2D::new();
    step(&mut world, &mut scene, 90);
    let y = scene
        .world()
        .get::<&Transform>(player)
        .expect("t")
        .position
        .y;
    assert!((y - 0.95).abs() < 0.1, "{y}");
}
