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

#[test]
fn a_crate_dropped_in_sends_rings_outward() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    let mut physics = PhysicsWorld3D::new();
    physics.set_ripple_view(Vec3::new(0.0, 10.0, 10.0), Vec3::ZERO);
    step_n(&mut physics, &mut scene, 5);
    assert!(physics.ripples().is_none(), "undisturbed water");

    crate_at(&mut scene, Vec3::new(1.0, 2.0, 1.0), 1.0);
    // It falls 1.5 m (about half a second), hits, and the ring spreads.
    let mut reached = Vec::new();
    for _ in 0..4 {
        step_n(&mut physics, &mut scene, 30);
        // How far out from the crate the water is moving, along +X.
        let edge = (1..60)
            .rev()
            .map(|i| i as f32 * 0.25)
            .find(|d| physics.ripple_height(1.0 + d, 1.0).abs() > 2e-3)
            .unwrap_or(0.0);
        reached.push(edge);
    }
    assert!(reached[3] > reached[1] + 1.0, "rings spread: {reached:?}");
    let map = physics.ripples().expect("rippling water to draw");
    // The field sits around the view, which looks at the splash.
    assert!(map.center.distance(glam::Vec2::new(0.0, 10.0)) < 20.0);
}

#[test]
fn a_moving_body_leaves_a_wake_and_a_still_one_does_not() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    // Settle a floating crate, then the water calms.
    let boat = crate_at(&mut scene, Vec3::new(-6.0, 0.0, 0.0), 1.0);
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 60 * 40);
    assert!(
        physics.ripples().is_none(),
        "a crate at rest makes no ripples"
    );

    // Push it along +X at 4 m/s (faster than ripples spread).
    for _ in 0..90 {
        physics.set_velocity(boat, Vec3::new(4.0, 0.0, 0.0));
        step_n(&mut physics, &mut scene, 1);
    }
    let x = position(&scene, boat).x;
    // Behind it the water is churned up; well ahead it's still flat.
    let behind: f32 = (-8..=8)
        .map(|i| physics.ripple_height(x - 3.0, i as f32 * 0.25).abs())
        .sum();
    let ahead: f32 = (-8..=8)
        .map(|i| physics.ripple_height(x + 3.0, i as f32 * 0.25).abs())
        .sum();
    assert!(
        behind > 0.05 && ahead < behind * 0.1,
        "behind {behind}, ahead {ahead}"
    );
}

#[test]
fn scripts_can_splash_the_water() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    let mut physics = PhysicsWorld3D::new();
    physics.splash(Vec3::new(2.0, 0.0, 2.0), 1.0);
    step_n(&mut physics, &mut scene, 1);
    assert!(physics.ripple_height(2.0, 2.0) < -0.1);
    assert!(physics.ripples().is_some());
    // Without water there is nothing to splash.
    let mut dry = SceneWorld::new();
    let mut physics = PhysicsWorld3D::new();
    physics.splash(Vec3::ZERO, 1.0);
    step_n(&mut physics, &mut dry, 1);
    assert!(physics.ripples().is_none());
}

/// Steps until a body splashes (or `n` steps pass): the splashes, and the
/// particle bursts in the scene afterwards.
fn step_until_splash(
    physics: &mut PhysicsWorld3D,
    scene: &mut SceneWorld,
    n: usize,
) -> Vec<WaterSplash> {
    for _ in 0..n {
        physics.sync_from_scene(scene);
        physics.move_characters(scene, DT);
        let tick = physics.step(scene, DT);
        if !tick.splashes.is_empty() {
            return tick.splashes;
        }
    }
    Vec::new()
}

fn bursts(scene: &SceneWorld) -> Vec<automata_scene::ParticleEmitter> {
    scene
        .query::<&automata_scene::ParticleEmitter>()
        .iter()
        .cloned()
        .collect()
}

#[test]
fn hitting_the_water_hard_throws_droplets_sized_by_speed() {
    let drop = |height: f32| {
        let mut scene = SceneWorld::new();
        calm_pool(&mut scene);
        crate_at(&mut scene, Vec3::new(1.0, height, 2.0), 1.0);
        let mut physics = PhysicsWorld3D::new();
        let splashes = step_until_splash(&mut physics, &mut scene, 240);
        (splashes, bursts(&scene))
    };
    let (low, low_bursts) = drop(1.2);
    let (high, high_bursts) = drop(6.0);
    assert_eq!(low.len(), 1, "{low:?}");
    assert_eq!(high.len(), 1, "{high:?}");
    // On the surface, where it went in.
    let s = high[0];
    assert!(
        s.position.y.abs() < 1e-3 && (s.position.x - 1.0).abs() < 0.1,
        "{s:?}"
    );
    assert!((s.position.z - 2.0).abs() < 0.1, "{s:?}");
    assert!(high[0].speed > low[0].speed + 3.0, "{low:?} {high:?}");
    // One burst each; the harder hit throws more, faster.
    assert_eq!((low_bursts.len(), high_bursts.len()), (1, 1));
    let (a, b) = (&low_bursts[0], &high_bursts[0]);
    assert!(b.burst > a.burst * 2, "{} vs {}", a.burst, b.burst);
    assert!(b.speed[1] > a.speed[1], "{:?} vs {:?}", a.speed, b.speed);
    assert!(b.one_shot && a.one_shot);
}

#[test]
fn easing_into_the_water_makes_no_splash() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    // Starts just above the surface: in at well under the splash speed.
    crate_at(&mut scene, Vec3::new(0.0, 0.52, 0.0), 1.0);
    let mut physics = PhysicsWorld3D::new();
    assert_eq!(step_until_splash(&mut physics, &mut scene, 120).len(), 0);
    assert_eq!(bursts(&scene).len(), 0);
}

#[test]
fn a_character_jumping_in_splashes() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    scene.spawn((
        Transform::from_position(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody::kinematic_position(),
        Collider3D::capsule(0.5, 0.3),
        CharacterController::new(),
        CharacterVelocity::new(Vec3::new(0.0, -6.0, 0.0)),
    ));
    let mut physics = PhysicsWorld3D::new();
    let splashes = step_until_splash(&mut physics, &mut scene, 120);
    assert_eq!(splashes.len(), 1, "{splashes:?}");
    assert!(splashes[0].speed > 4.0, "{splashes:?}");
    assert_eq!(bursts(&scene).len(), 1);
}

#[test]
fn water_particles_can_be_turned_off() {
    let mut scene = SceneWorld::new();
    calm_pool(&mut scene);
    crate_at(&mut scene, Vec3::new(0.0, 4.0, 0.0), 1.0);
    let mut physics = PhysicsWorld3D::new();
    physics.set_water_particles(false);
    assert_eq!(step_until_splash(&mut physics, &mut scene, 240).len(), 1);
    assert_eq!(bursts(&scene).len(), 0);
}

#[test]
fn strong_wind_blows_spray_off_the_crests() {
    let spray = |wind: f32| {
        let mut scene = SceneWorld::new();
        scene.spawn((
            Transform::from_position(Vec3::ZERO),
            Water {
                size: [80.0, 80.0],
                wave_height: 0.6,
                wave_length: 10.0,
                wind,
                ..Water::default()
            },
        ));
        let mut physics = PhysicsWorld3D::new();
        let mut seen = 0;
        let mut on_crests = true;
        for _ in 0..120 {
            physics.sync_from_scene(&mut scene);
            physics.step(&mut scene, DT);
            for (t, e) in &mut scene.query::<(&Transform, &automata_scene::ParticleEmitter)>() {
                if e.burst > 0 {
                    // Up on a crest: above the rest height.
                    on_crests &= t.position.y > 0.1;
                }
            }
            seen = seen.max(bursts(&scene).len());
        }
        (seen, on_crests)
    };
    assert_eq!(spray(0.0).0, 0, "calm air: no spray");
    assert_eq!(spray(0.45).0, 0, "a breeze: no spray");
    let (strong, on_crests) = spray(1.0);
    assert!(strong >= 5, "a gale throws spray, saw {strong}");
    assert!(on_crests);
}
