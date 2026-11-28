//! Joints: hinges, balls, sliders, ropes, springs and welds between
//! bodies (or to the world), with limits, motors and break forces.

use automata_physics::prelude::*;
use automata_scene::{Entity, Name, SceneWorld, Transform};
use glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;

fn step_n(physics: &mut PhysicsWorld3D, scene: &mut SceneWorld, n: usize) -> Vec<Entity> {
    let mut broken = Vec::new();
    for _ in 0..n {
        physics.sync_from_scene(scene);
        broken.extend(physics.step(scene, DT).broken_joints);
    }
    broken
}

fn position(scene: &SceneWorld, entity: Entity) -> Vec3 {
    scene.get::<&Transform>(entity).unwrap().position
}

fn rotation(scene: &SceneWorld, entity: Entity) -> Quat {
    scene.get::<&Transform>(entity).unwrap().rotation
}

fn state(scene: &SceneWorld, entity: Entity) -> JointState {
    *scene.get::<&JointState>(entity).unwrap()
}

fn block(scene: &mut SceneWorld, name: &str, at: Vec3, size: Vec3) -> Entity {
    scene.spawn((
        Name::new(name),
        Transform::from_position(at),
        RigidBody::dynamic(),
        Collider3D::cuboid(size.x, size.y, size.z),
    ))
}

fn post(scene: &mut SceneWorld, name: &str, at: Vec3) -> Entity {
    scene.spawn((
        Name::new(name),
        Transform::from_position(at),
        Collider3D::cuboid(0.2, 0.2, 0.2),
    ))
}

#[test]
fn a_ball_joint_pendulum_keeps_its_length_and_swings() {
    let mut scene = SceneWorld::new();
    // Pivot at the origin, bob 2 m out to the side: it swings down.
    let bob = block(
        &mut scene,
        "Bob",
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::splat(0.3),
    );
    scene
        .insert_one(bob, Joint::ball().anchor(Vec3::new(-2.0, 0.0, 0.0)))
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    let mut lowest = f32::MAX;
    for _ in 0..90 {
        step_n(&mut physics, &mut scene, 1);
        let p = position(&scene, bob);
        lowest = lowest.min(p.y);
        assert!((p.length() - 2.0).abs() < 0.05, "length drifted: {p}");
    }
    assert!(
        lowest < -1.9,
        "it never swung through the bottom ({lowest})"
    );
    assert_eq!(physics.joint_count(), 1);
    assert!(state(&scene, bob).attached);
}

#[test]
fn a_hinged_door_turns_on_its_edge_and_stops_at_its_limits() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Frame", Vec3::new(0.0, 1.0, 0.0));
    // A 1 m wide door whose left edge is on the frame.
    let door = block(
        &mut scene,
        "Door",
        Vec3::new(0.5, 1.0, 0.0),
        Vec3::new(1.0, 2.0, 0.1),
    );
    let limit = 1.0;
    scene
        .insert_one(
            door,
            Joint::hinge(Vec3::Y)
                .connected_to("Frame")
                .anchor(Vec3::new(-0.5, 0.0, 0.0))
                .limits(-limit, limit),
        )
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 1);
    // Shove the free edge sideways (the door weighs 0.2 kg at density 1).
    physics.apply_impulse_at(door, Vec3::new(0.0, 0.0, -0.4), Vec3::new(1.0, 1.0, 0.0));
    let mut widest = 0.0f32;
    for _ in 0..120 {
        step_n(&mut physics, &mut scene, 1);
        let angle = state(&scene, door).value;
        widest = widest.max(angle.abs());
        // The hinge edge stays put; the door doesn't fall.
        let edge = position(&scene, door) + rotation(&scene, door) * Vec3::new(-0.5, 0.0, 0.0);
        assert!(
            edge.distance(Vec3::new(0.0, 1.0, 0.0)) < 0.05,
            "edge moved to {edge}"
        );
    }
    assert!(widest > 0.8, "the shove barely moved it ({widest})");
    assert!(widest < limit + 0.08, "it swung past its limit ({widest})");
}

#[test]
fn a_hinge_motor_spins_and_a_target_holds_an_angle() {
    let mut scene = SceneWorld::new();
    let wheel = block(&mut scene, "Wheel", Vec3::ZERO, Vec3::new(1.0, 1.0, 0.2));
    scene
        .insert_one(
            wheel,
            Joint::hinge(Vec3::Z).motor(JointMotor {
                speed: 3.0,
                ..JointMotor::default()
            }),
        )
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 60);
    let spin = physics.angular_velocity(wheel).z;
    assert!((spin - 3.0).abs() < 0.3, "spinning at {spin}");
    assert!(position(&scene, wheel).length() < 0.05, "the axle moved");

    // Retarget in place: pull to 0.5 rad and hold it.
    scene.get::<&mut Joint>(wheel).unwrap().motor = Some(JointMotor::target(0.5));
    step_n(&mut physics, &mut scene, 240);
    let angle = state(&scene, wheel).value;
    assert!((angle - 0.5).abs() < 0.05, "held at {angle}");
}

#[test]
fn a_slider_moves_along_its_axis_within_its_limits() {
    let mut scene = SceneWorld::new();
    let drawer = block(
        &mut scene,
        "Drawer",
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::splat(0.4),
    );
    scene
        .insert_one(
            drawer,
            Joint::slider(Vec3::X)
                .limits(0.0, 0.6)
                .motor(JointMotor::speed(2.0)),
        )
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 120);
    let p = position(&scene, drawer);
    assert!((p.x - 0.6).abs() < 0.03, "stopped at {p}");
    assert!(
        (p.y - 1.0).abs() < 0.03 && p.z.abs() < 0.03,
        "left its axis: {p}"
    );
    assert!((state(&scene, drawer).value - 0.6).abs() < 0.03);
}

#[test]
fn a_rope_is_slack_up_close_and_taut_at_its_length() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Hook", Vec3::new(0.0, 5.0, 0.0));
    // Starts 1 m below the hook on a 3 m rope: falls, then the rope catches it.
    let load = block(
        &mut scene,
        "Load",
        Vec3::new(0.0, 4.0, 0.0),
        Vec3::splat(0.3),
    );
    scene
        .insert_one(load, Joint::rope(3.0).connected_to("Hook"))
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 10);
    assert!(position(&scene, load).y < 3.9, "a slack rope held it up");
    step_n(&mut physics, &mut scene, 180);
    let p = position(&scene, load);
    assert!((p.y - 2.0).abs() < 0.08, "hangs at {p}");
    assert!((state(&scene, load).value - 3.0).abs() < 0.08);
}

#[test]
fn a_rope_with_no_length_keeps_the_starting_distance() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Hook", Vec3::new(0.0, 5.0, 0.0));
    let load = block(
        &mut scene,
        "Load",
        Vec3::new(2.0, 5.0, 0.0),
        Vec3::splat(0.3),
    );
    scene
        .insert_one(load, Joint::rope(0.0).connected_to("Hook"))
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 240);
    let p = position(&scene, load);
    assert!(
        (p.distance(Vec3::new(0.0, 5.0, 0.0)) - 2.0).abs() < 0.08,
        "{p}"
    );
}

#[test]
fn a_spring_bobs_and_settles_below_its_rest_length() {
    let mut scene = SceneWorld::new();
    let load = block(
        &mut scene,
        "Load",
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::splat(0.3),
    );
    // Hangs from the world 1 m above it, rest length 1 m.
    scene
        .insert_one(
            load,
            Joint::spring(1.0, 40.0, 3.0).connected_anchor(Vec3::new(0.0, 0.0, 0.0)),
        )
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    let mut lowest = f32::MAX;
    for _ in 0..60 {
        step_n(&mut physics, &mut scene, 1);
        lowest = lowest.min(position(&scene, load).y);
    }
    step_n(&mut physics, &mut scene, 600);
    let settled = position(&scene, load).y;
    // Gravity stretches it by g / k (acceleration-based stiffness).
    let stretch = 9.81 / 40.0;
    assert!(
        (settled + 1.0 + stretch).abs() < 0.06,
        "settled at {settled}"
    );
    assert!(
        lowest < settled - 0.05,
        "it never bounced ({lowest} vs {settled})"
    );
}

#[test]
fn a_fixed_joint_welds_two_bodies() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Wall", Vec3::new(0.0, 2.0, 0.0));
    let shelf = block(
        &mut scene,
        "Shelf",
        Vec3::new(0.8, 2.0, 0.0),
        Vec3::new(1.2, 0.1, 0.4),
    );
    scene
        .insert_one(shelf, Joint::fixed().connected_to("Wall"))
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 120);
    let p = position(&scene, shelf);
    assert!(p.distance(Vec3::new(0.8, 2.0, 0.0)) < 0.05, "sagged to {p}");
    assert!(rotation(&scene, shelf).angle_between(Quat::IDENTITY) < 0.05);
}

#[test]
fn a_chain_of_links_hangs_together() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Ceiling", Vec3::new(0.0, 6.0, 0.0));
    let mut links = Vec::new();
    for i in 0..8 {
        #[allow(clippy::cast_precision_loss)]
        let at = Vec3::new(0.5 * (i + 1) as f32, 6.0, 0.0);
        let name = format!("Link {i}");
        let link = block(&mut scene, &name, at, Vec3::new(0.45, 0.1, 0.1));
        let previous = if i == 0 {
            "Ceiling".to_string()
        } else {
            format!("Link {}", i - 1)
        };
        scene
            .insert_one(
                link,
                Joint::ball()
                    .connected_to(previous)
                    .anchor(Vec3::new(-0.25, 0.0, 0.0)),
            )
            .unwrap();
        links.push(link);
    }
    let mut physics = PhysicsWorld3D::new();
    let mut lowest = f32::MAX;
    for _ in 0..240 {
        step_n(&mut physics, &mut scene, 1);
        lowest = lowest.min(position(&scene, links[7]).y);
        // Each link's left end stays on its neighbour's right end.
        for pair in links.windows(2) {
            let end = |e, x| position(&scene, e) + rotation(&scene, e) * Vec3::new(x, 0.0, 0.0);
            let gap = end(pair[0], 0.25).distance(end(pair[1], -0.25));
            assert!(gap < 0.05, "links pulled apart: {gap}");
        }
    }
    assert_eq!(physics.joint_count(), 8);
    // Laid out sideways, it swings down through the bottom.
    assert!(lowest < 2.5, "the end only got down to {lowest}");
}

#[test]
fn a_joint_snaps_past_its_break_force() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Hook", Vec3::new(0.0, 5.0, 0.0));
    // A 1 m cube of density 1 is 1 kg: about 9.8 N hanging.
    let heavy = block(&mut scene, "Heavy", Vec3::new(0.0, 4.0, 0.0), Vec3::ONE);
    scene
        .insert_one(heavy, Joint::ball().connected_to("Hook").break_force(5.0))
        .unwrap();
    let light = block(&mut scene, "Light", Vec3::new(3.0, 4.0, 0.0), Vec3::ONE);
    let mut strong = Joint::ball().break_force(30.0);
    strong.connected_anchor = Some(Vec3::new(3.0, 5.0, 0.0));
    scene.insert_one(light, strong).unwrap();
    let mut physics = PhysicsWorld3D::new();
    let broken = step_n(&mut physics, &mut scene, 30);
    assert_eq!(broken, vec![heavy]);
    let snapped = state(&scene, heavy);
    assert!(snapped.broken && !snapped.attached);
    assert!(
        position(&scene, heavy).y < 3.5,
        "it should fall once snapped"
    );
    let held = state(&scene, light);
    assert!(held.attached && !held.broken);
    assert!(
        held.force > 8.0 && held.force < 12.0,
        "force {}",
        held.force
    );
    // Stays snapped, and only a structural change rebuilds it.
    step_n(&mut physics, &mut scene, 5);
    assert!(state(&scene, heavy).broken);
}

#[test]
fn disabling_lets_go_and_removing_the_component_drops_the_joint() {
    let mut scene = SceneWorld::new();
    post(&mut scene, "Hook", Vec3::new(0.0, 5.0, 0.0));
    let load = block(
        &mut scene,
        "Load",
        Vec3::new(0.0, 4.0, 0.0),
        Vec3::splat(0.3),
    );
    scene
        .insert_one(load, Joint::ball().connected_to("Hook"))
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 30);
    assert!((position(&scene, load).y - 4.0).abs() < 0.05);
    scene.get::<&mut Joint>(load).unwrap().enabled = false;
    step_n(&mut physics, &mut scene, 30);
    assert!(
        position(&scene, load).y < 3.5,
        "a disabled joint still held it"
    );
    assert!(!state(&scene, load).attached);
    scene.remove_one::<Joint>(load).unwrap();
    step_n(&mut physics, &mut scene, 1);
    assert_eq!(physics.joint_count(), 0);
}

#[test]
fn a_joint_waits_for_its_partner_and_lets_go_when_it_goes() {
    let mut scene = SceneWorld::new();
    let load = block(
        &mut scene,
        "Load",
        Vec3::new(0.0, 4.0, 0.0),
        Vec3::splat(0.3),
    );
    scene
        .insert_one(load, Joint::ball().connected_to("Hook"))
        .unwrap();
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 2);
    assert_eq!(physics.joint_count(), 0, "there's no Hook yet");
    assert!(!state(&scene, load).attached);
    let above = position(&scene, load) + Vec3::Y;
    let hook = post(&mut scene, "Hook", above);
    step_n(&mut physics, &mut scene, 1);
    assert_eq!(physics.joint_count(), 1);
    scene.despawn(hook).unwrap();
    step_n(&mut physics, &mut scene, 2);
    assert_eq!(physics.joint_count(), 0);
    assert!(!state(&scene, load).attached && !state(&scene, load).broken);
}

#[test]
fn joining_two_dynamic_bodies_moves_them_together() {
    let mut scene = SceneWorld::new();
    let a = block(&mut scene, "A", Vec3::new(0.0, 0.0, 0.0), Vec3::splat(0.5));
    let b = block(&mut scene, "B", Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.5));
    scene
        .insert_one(b, Joint::fixed().connected_to("A"))
        .unwrap();
    for e in [a, b] {
        scene.get::<&mut RigidBody>(e).unwrap().gravity_scale = 0.0;
    }
    let mut physics = PhysicsWorld3D::new();
    step_n(&mut physics, &mut scene, 1);
    physics.apply_impulse(a, Vec3::new(0.0, 0.0, 50.0));
    step_n(&mut physics, &mut scene, 60);
    let (pa, pb) = (position(&scene, a), position(&scene, b));
    assert!(pa.z > 0.5 && pb.z > 0.5, "the push didn't carry: {pa} {pb}");
    assert!((pa.distance(pb) - 1.0).abs() < 0.03);
}
