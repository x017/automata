//! Physics steps inside the engine's headless loop.

use automata_core::{EngineConfig, WindowMode};
use automata_physics::prelude::*;
use automata_scene::Transform;
use glam::Vec3;

#[test]
fn ball_falls_and_lands_inside_headless_loop() {
    let config = EngineConfig {
        window_mode: WindowMode::Headless,
        target_fps: None,
        ..EngineConfig::default()
    };
    let mut app = automata_core::Application::new(config);

    let floor = app.world_mut().scene_mut().spawn((
        Transform::from_position(Vec3::ZERO),
        RigidBody::fixed(),
        Collider3D::cuboid(20.0, 1.0, 20.0),
    ));
    let ball = app.world_mut().scene_mut().spawn((
        Transform::from_position(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.5),
    ));
    let _ = floor;

    app.run(|ctx| {
        if ctx.time().frame_count() >= 120 {
            ctx.request_quit();
        }
    })
    .unwrap();

    let y = app
        .world()
        .scene()
        .world()
        .get::<&Transform>(ball)
        .unwrap()
        .position
        .y;
    assert!(y > 0.8 && y < 1.3, "ball should rest on the floor, y = {y}");
}
