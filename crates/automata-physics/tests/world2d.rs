//! Integration tests for the 2D physics world.

use automata_physics::prelude::*;
use automata_scene::{SceneWorld, Transform};
use glam::Vec3;

const DT: f32 = 1.0 / 60.0;

fn step_n(physics: &mut PhysicsWorld2D, scene: &mut SceneWorld, n: usize) -> PhysicsTick {
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
fn dynamic_body_falls_in_2d() {
    let mut scene = SceneWorld::new();
    let ball = scene.spawn((
        Transform::from_xy(0.0, 10.0),
        RigidBody::dynamic(),
        Collider2D::ball(0.5),
    ));

    let mut physics = PhysicsWorld2D::new();
    step_n(&mut physics, &mut scene, 30);

    assert!(position(&scene, ball).y < 9.0, "ball should fall in 2D");
}

#[test]
fn ball_rests_on_2d_floor() {
    let mut scene = SceneWorld::new();
    let _floor = scene.spawn((
        Transform::from_xy(0.0, 0.0),
        RigidBody::fixed(),
        Collider2D::cuboid(20.0, 1.0),
    ));
    let ball = scene.spawn((
        Transform::from_xy(0.0, 3.0),
        RigidBody::dynamic(),
        Collider2D::ball(0.5),
    ));

    let mut physics = PhysicsWorld2D::new();
    step_n(&mut physics, &mut scene, 120);

    let y = position(&scene, ball).y;
    assert!(y > 0.8 && y < 1.3, "ball should rest on the floor, y = {y}");
}

#[test]
fn collision_events_fire_in_2d() {
    let mut scene = SceneWorld::new();
    let floor = scene.spawn((
        Transform::from_xy(0.0, 0.0),
        RigidBody::fixed(),
        Collider2D::cuboid(20.0, 1.0),
    ));
    let ball = scene.spawn((
        Transform::from_xy(0.0, 1.05),
        RigidBody::dynamic(),
        Collider2D::ball(0.5),
    ));

    let mut physics = PhysicsWorld2D::new();
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
    assert!(saw_start, "expected a CollisionStarted in 2D");
}

#[test]
fn raycast_hits_in_2d() {
    let mut scene = SceneWorld::new();
    let target = scene.spawn((
        Transform::from_xy(5.0, 0.0),
        RigidBody::fixed(),
        Collider2D::cuboid(1.0, 2.0),
    ));

    let mut physics = PhysicsWorld2D::new();
    // One step so the broad phase is ready for queries (the engine always
    // steps at least once before scripts can query).
    physics.sync_from_scene(&mut scene);
    physics.step(&mut scene, DT);

    let hit = physics
        .cast_ray(
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            100.0,
            &RayFilter::everything(),
        )
        .expect("ray should hit");
    assert_eq!(hit.entity, target);
    assert!((hit.distance - 4.5).abs() < 0.01);
}

#[test]
fn character_controller_platformer_movement() {
    let mut scene = SceneWorld::new();
    let _floor = scene.spawn((
        Transform::from_xy(0.0, 0.0),
        RigidBody::fixed(),
        Collider2D::cuboid(40.0, 1.0),
    ));
    let character = scene.spawn((
        Transform::from_xy(0.0, 1.2),
        RigidBody::kinematic_position(),
        Collider2D::capsule(0.35, 0.3),
        CharacterController::new().snap_to_ground(0.4),
        CharacterVelocity::new(Vec3::new(3.0, 0.0, 0.0)),
    ));

    let mut physics = PhysicsWorld2D::new();
    step_n(&mut physics, &mut scene, 90);

    let pos = position(&scene, character);
    assert!(pos.x > 2.0, "character should move +x, x = {}", pos.x);
    // Capsule: half_height 0.35 + radius 0.3 = 0.65 above feet; floor top 0.5.
    assert!(
        (1.0..=1.35).contains(&pos.y),
        "character should stand on the floor, y = {}",
        pos.y
    );
}

#[test]
fn shape_cast_reports_impact() {
    let mut scene = SceneWorld::new();
    let _wall = scene.spawn((
        Transform::from_xy(3.0, 0.0),
        RigidBody::fixed(),
        Collider2D::cuboid(1.0, 4.0),
    ));

    let mut physics = PhysicsWorld2D::new();
    // One step so the broad phase is ready for queries (the engine always
    // steps at least once before scripts can query).
    physics.sync_from_scene(&mut scene);
    physics.step(&mut scene, DT);

    let hit = physics
        .cast_shape(
            automata_physics::Shape2D::Ball { radius: 0.5 },
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            100.0,
            &RayFilter::everything(),
        )
        .expect("shape cast should hit the wall");
    // Ball centre travels until surface touches wall at x = 2.5 → toi = 2.0.
    assert!((hit.distance - 2.0).abs() < 0.1, "hit at {}", hit.distance);
}

fn joint_state(scene: &SceneWorld, entity: automata_scene::Entity) -> JointState {
    *scene.get::<&JointState>(entity).unwrap()
}

#[test]
fn a_2d_pendulum_swings_on_a_pin_and_keeps_its_length() {
    let mut scene = SceneWorld::new();
    scene.spawn((
        automata_scene::Name::new("Pin"),
        Transform::from_xy(0.0, 5.0),
    ));
    let bob = scene.spawn((
        Transform::from_xy(2.0, 5.0),
        RigidBody::dynamic(),
        Collider2D::ball(0.2),
        Joint::hinge(Vec3::Z)
            .connected_to("Pin")
            .anchor(Vec3::new(-2.0, 0.0, 0.0)),
    ));
    let mut physics = PhysicsWorld2D::new();
    let mut lowest = f32::MAX;
    for _ in 0..90 {
        step_n(&mut physics, &mut scene, 1);
        let p = position(&scene, bob);
        lowest = lowest.min(p.y);
        let arm = p.truncate().distance(glam::Vec2::new(0.0, 5.0));
        assert!((arm - 2.0).abs() < 0.05, "arm length {arm}");
    }
    assert!(lowest < 3.1, "it never swung down ({lowest})");
    assert!(joint_state(&scene, bob).attached);
    assert!(joint_state(&scene, bob).value.abs() > 0.1);
}

#[test]
fn a_2d_slider_motor_runs_to_its_limit() {
    let mut scene = SceneWorld::new();
    let cart = scene.spawn((
        Transform::from_xy(0.0, 1.0),
        RigidBody::dynamic(),
        Collider2D::cuboid(0.5, 0.5),
        Joint::slider(Vec3::X)
            .limits(-1.0, 1.5)
            .motor(JointMotor::speed(3.0)),
    ));
    let mut physics = PhysicsWorld2D::new();
    step_n(&mut physics, &mut scene, 120);
    let p = position(&scene, cart);
    assert!((p.x - 1.5).abs() < 0.03 && (p.y - 1.0).abs() < 0.03, "{p}");
}

#[test]
fn a_2d_rope_catches_a_falling_load_and_snaps_when_overloaded() {
    let mut scene = SceneWorld::new();
    let load = scene.spawn((
        Transform::from_xy(0.0, 4.0),
        RigidBody::dynamic(),
        Collider2D::cuboid(1.0, 1.0),
        Joint::rope(2.0).connected_anchor(Vec3::new(0.0, 5.0, 0.0)),
    ));
    let mut physics = PhysicsWorld2D::new();
    step_n(&mut physics, &mut scene, 180);
    assert!((position(&scene, load).y - 3.0).abs() < 0.08);
    // 1 kg hanging pulls ~9.8 N; set the break force under that.
    scene.get::<&mut Joint>(load).unwrap().break_force = 5.0;
    let mut snapped = Vec::new();
    for _ in 0..10 {
        physics.sync_from_scene(&mut scene);
        snapped.extend(physics.step(&mut scene, DT).broken_joints);
    }
    assert_eq!(snapped, vec![load]);
    assert!(joint_state(&scene, load).broken);
}
