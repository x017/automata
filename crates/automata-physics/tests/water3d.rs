//! Buoyancy and drag in `Water` volumes.

use automata_physics::prelude::*;
use automata_renderer::Water;
use automata_scene::{SceneWorld, Transform};
use glam::Vec3;

const DT: f32 = 1.0 / 60.0;

fn step_n(physics: &mut PhysicsWorld3D, scene: &mut SceneWorld, n: usize) {
    for _ in 0..n {
        physics.sync_from_scene(scene);
        physics.step(scene, DT);
    }
}

fn position(scene: &SceneWorld, entity: automata_scene::Entity) -> Vec3 {
    scene.get::<&Transform>(entity).unwrap().position
}

/// A still 20 × 20 m pool with its surface at y = 0.
fn calm_pool(scene: &mut SceneWorld) {
    scene.spawn((
        Transform::from_position(Vec3::ZERO),
        Water {
            size: [20.0, 20.0],
            wave_height: 0.0,
            ..Water::default()
        },
    ));
}

fn crate_at(scene: &mut SceneWorld, at: Vec3, density: f32) -> automata_scene::Entity {
    scene.spawn((
        Transform::from_position(at),
        RigidBody::dynamic(),
        Collider3D::cuboid(1.0, 1.0, 1.0).density(density),
    ))
}

#[test]
fn a_light_crate_floats_at_its_density_line() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    // Water density 2, crate density 1: half under, centre at the surface.
    let light = crate_at(&mut scene, Vec3::new(0.0, 3.0, 0.0), 1.0);
    // Density 1.5: three quarters under, centre 0.25 m below.
    let heavier = crate_at(&mut scene, Vec3::new(4.0, 3.0, 0.0), 1.5);

    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 600);

    let y = position(&scene, light).y;
    assert!(
        y.abs() < 0.05,
        "light crate should float centred on the surface, y = {y}"
    );
    let y = position(&scene, heavier).y;
    assert!(
        (y + 0.25).abs() < 0.05,
        "heavier crate should ride lower, y = {y}"
    );
    assert!(physics
        .water_height(0.0, 0.0)
        .is_some_and(|h| h.abs() < 1e-4));
}

#[test]
fn a_dense_crate_sinks_but_slower_than_it_falls() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    let rock = crate_at(&mut scene, Vec3::new(0.0, -0.5, 0.0), 6.0);
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 60);

    // Free fall would cover 4.9 m in a second; buoyancy (1/3 of the weight)
    // and drag hold it to well under that.
    let sunk = -0.5 - position(&scene, rock).y;
    assert!(sunk > 0.5, "rock should sink, sank {sunk}");
    assert!(sunk < 3.0, "water should slow the rock, sank {sunk}");
}

#[test]
fn no_buoyancy_outside_the_water_rectangle() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    let outside = crate_at(&mut scene, Vec3::new(15.0, -0.5, 0.0), 1.0);
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 30);
    let y = position(&scene, outside).y;
    // Free fall for half a second: about 1.2 m.
    assert!(
        y < -1.5,
        "crate beside the pool should fall freely, y = {y}"
    );
    assert!(physics.water_height(15.0, 0.0).is_none());
}

#[test]
fn waves_lift_and_drop_a_floating_buoy() {
    let mut scene = SceneWorld::new();
    scene.spawn((
        Transform::from_position(Vec3::ZERO),
        Water {
            size: [40.0, 40.0],
            wave_height: 0.6,
            wave_length: 6.0,
            ..Water::default()
        },
    ));
    let buoy = scene.spawn((
        Transform::from_position(Vec3::new(1.0, 0.0, 2.0)),
        RigidBody::dynamic(),
        Collider3D::ball(0.25),
    ));
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 120);
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for _ in 0..240 {
        step_n(&mut physics, &mut scene, 1);
        let y = position(&scene, buoy).y;
        low = low.min(y);
        high = high.max(y);
    }
    assert!(
        high - low > 0.15,
        "buoy should bob on the waves ({low}..{high})"
    );
    assert!(
        low > -1.0 && high < 1.0,
        "buoy should stay at the surface ({low}..{high})"
    );
}

#[test]
fn floating_bodies_sleep_on_calm_water_but_not_on_waves() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    let calm = crate_at(&mut scene, Vec3::new(0.0, 0.0, 0.0), 1.0);
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 600);
    assert!(
        physics.is_sleeping(calm),
        "settled crate on calm water should sleep"
    );

    let mut scene = SceneWorld::new();
    scene.spawn((
        Transform::from_position(Vec3::ZERO),
        Water {
            size: [20.0, 20.0],
            wave_height: 0.3,
            ..Water::default()
        },
    ));
    let rocking = crate_at(&mut scene, Vec3::new(0.0, 0.0, 0.0), 1.0);
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 600);
    assert!(
        !physics.is_sleeping(rocking),
        "a crate on waves must keep riding them"
    );
}
