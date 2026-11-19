//! Integration tests for the 3D physics world.

use automata_physics::prelude::*;
use automata_scene::{SceneWorld, Transform};
use glam::Vec3;

const DT: f32 = 1.0 / 60.0;

fn step_n(physics: &mut PhysicsWorld3D, scene: &mut SceneWorld, n: usize) -> PhysicsTick {
    let mut last = PhysicsTick::default();
    for _ in 0..n {
        physics.sync_from_scene(scene);
        physics.move_characters(scene, DT);
        last = physics.step(scene, DT);
    }
    last
}

fn position(scene: &SceneWorld, entity: automata_scene::Entity) -> Vec3 {
    scene.get::<&Transform>(entity).unwrap().position
}

#[test]
fn dynamic_body_falls_under_gravity() {
    let mut scene = SceneWorld::new();
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 10.0, 0.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.5),
    ));

    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 30);

    let y = position(&scene, ball).y;
    assert!(y < 9.0, "ball should have fallen, y = {y}");
}

#[test]
fn ball_lands_on_floor_and_stays() {
    let mut scene = SceneWorld::new();
    let _floor = scene.spawn((
        Transform::from_position(Vec3::ZERO),
        RigidBody::fixed(),
        Collider3D::cuboid(20.0, 1.0, 20.0),
    ));
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.5),
    ));

    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 120);

    let y = position(&scene, ball).y;
    // Ball radius 0.5 resting on floor top (y = 0.5): centre at ~1.0.
    assert!(y > 0.8 && y < 1.3, "ball should rest on the floor, y = {y}");
}

#[test]
fn collision_events_fire_on_contact() {
    let mut scene = SceneWorld::new();
    let floor = scene.spawn((
        Transform::from_position(Vec3::ZERO),
        RigidBody::fixed(),
        Collider3D::cuboid(20.0, 1.0, 20.0),
    ));
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 1.05, 0.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.5),
    ));

    let mut physics = PhysicsWorld3D::new();
    let mut saw_start = false;
    for _ in 0..30 {
        physics.sync_from_scene(&mut scene);
        let tick = physics.step(&mut scene, DT);
        for event in tick.collisions {
            if let PhysicsEvent::CollisionStarted { entity_a, entity_b } = event {
                let pair = [entity_a, entity_b];
                if pair.contains(&ball) && pair.contains(&floor) {
                    saw_start = true;
                }
            }
        }
        if saw_start {
            break;
        }
    }
    assert!(saw_start, "expected a CollisionStarted for ball and floor");
}

#[test]
fn sensor_generates_events_without_response() {
    let mut scene = SceneWorld::new();
    let trigger = scene.spawn((
        Transform::from_position(Vec3::ZERO),
        RigidBody::fixed(),
        Collider3D::cuboid(2.0, 2.0, 2.0).sensor(true),
    ));
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 0.5, 0.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.5),
    ));

    let mut physics = PhysicsWorld3D::new();
    let mut saw_start = false;
    for _ in 0..30 {
        physics.sync_from_scene(&mut scene);
        let tick = physics.step(&mut scene, DT);
        for event in tick.collisions {
            if let PhysicsEvent::CollisionStarted { entity_a, entity_b } = event {
                let pair = [entity_a, entity_b];
                if pair.contains(&ball) && pair.contains(&trigger) {
                    saw_start = true;
                }
            }
        }
    }
    assert!(saw_start, "sensor should generate CollisionStarted");
    // Sensor gives no contact response: the ball passes through (y < 0).
    assert!(position(&scene, ball).y < 0.5);
}

#[test]
fn raycast_hits_closest_collider() {
    let mut scene = SceneWorld::new();
    let target = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 0.0, -5.0)),
        RigidBody::fixed(),
        Collider3D::cuboid(1.0, 1.0, 1.0),
    ));

    let mut physics = PhysicsWorld3D::new();
    // One step so the broad phase is ready for queries.
    physics.sync_from_scene(&mut scene);
    physics.step(&mut scene, DT);

    let hit = physics
        .cast_ray(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -1.0),
            100.0,
            &RayFilter::everything(),
        )
        .expect("ray should hit the cube");
    assert_eq!(hit.entity, target);
    assert!((hit.distance - 4.5).abs() < 0.01, "hit at {}", hit.distance);
    assert!((hit.normal.z - 1.0).abs() < 0.01);

    // Excluding the target entity yields no hit.
    let miss = physics.cast_ray(
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, -1.0),
        100.0,
        &RayFilter::everything().excluding(target),
    );
    assert!(miss.is_none());
}

#[test]
fn impulse_changes_velocity() {
    let mut scene = SceneWorld::new();
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 100.0, 0.0)),
        RigidBody::dynamic().gravity_scale(0.0),
        Collider3D::ball(0.5),
    ));

    let mut physics = PhysicsWorld3D::new();
    physics.sync_from_scene(&mut scene);
    physics.apply_impulse(ball, Vec3::new(5.0, 0.0, 0.0));
    step_n(&mut physics, &mut scene, 10);

    assert!(position(&scene, ball).x > 0.3, "impulse should push +x");
    assert!(physics.velocity(ball).x > 1.0);
}

#[test]
fn kinematic_position_body_pushes_dynamic() {
    let mut scene = SceneWorld::new();
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 5.0, 0.0)),
        RigidBody::dynamic().gravity_scale(0.0),
        Collider3D::ball(0.5),
    ));
    // Platform sweeps toward the ball.
    let platform = scene.spawn((
        Transform::from_position(Vec3::new(-5.0, 5.0, 0.0)),
        RigidBody::kinematic_position(),
        Collider3D::cuboid(1.0, 1.0, 1.0),
    ));

    let mut physics = PhysicsWorld3D::new();
    for _ in 0..120 {
        // Move the platform toward +x before syncing.
        if let Ok(mut t) = scene.get::<&mut Transform>(platform) {
            t.position.x += 4.0 * DT;
        }
        physics.sync_from_scene(&mut scene);
        physics.step(&mut scene, DT);
    }

    assert!(
        position(&scene, ball).x > 0.5,
        "kinematic platform should push the ball, x = {}",
        position(&scene, ball).x
    );
}

#[test]
fn character_controller_walks_and_stays_grounded() {
    let mut scene = SceneWorld::new();
    let _floor = scene.spawn((
        Transform::from_position(Vec3::ZERO),
        RigidBody::fixed(),
        Collider3D::cuboid(40.0, 1.0, 40.0),
    ));
    let character = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 1.5, 0.0)),
        RigidBody::kinematic_position(),
        Collider3D::capsule(0.5, 0.4),
        CharacterController::new().snap_to_ground(0.5),
        CharacterVelocity::new(Vec3::new(2.0, 0.0, 0.0)),
    ));

    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 90);

    let pos = position(&scene, character);
    assert!(pos.x > 1.5, "character should walk +x, x = {}", pos.x);
    // Capsule half_height 0.5 + radius 0.4 → feet at y - 0.9; floor top 0.5.
    assert!(
        (1.3..=1.6).contains(&pos.y),
        "character should stand on the floor, y = {}",
        pos.y
    );
}

#[test]
fn despawned_entity_leaves_physics_world() {
    let mut scene = SceneWorld::new();
    let ball = scene.spawn((
        Transform::from_position(Vec3::new(0.0, 10.0, 0.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.5),
    ));

    let mut physics = PhysicsWorld3D::new();
    physics.sync_from_scene(&mut scene);
    assert_eq!(physics.body_count(), 1);

    scene.despawn(ball).unwrap();
    physics.sync_from_scene(&mut scene);
    assert_eq!(physics.body_count(), 0);
}
