//! Ragdolls: an animated runner goes limp, falls onto the floor, takes
//! pushes, and stands back up where it came down.

use automata_physics::prelude::*;
use automata_scene::ragdoll::{RagdollJointKind, RagdollLayout};
use automata_scene::{
    animate, propagate_transforms, Animation, Entity, MeshVisual, Name, Ragdoll, RagdollPose,
    SceneWorld, Transform,
};
use glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;

fn runner_path() -> String {
    format!(
        "{}/../../examples/oberon/meadow/models/runner.glb",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A floor (top at y = 0) and a runner standing on it, idling.
fn stage(ragdoll: Ragdoll) -> (SceneWorld, PhysicsWorld3D, Entity) {
    let mut scene = SceneWorld::new();
    scene.spawn((
        Name::new("Floor"),
        Transform::from_position(Vec3::new(0.0, -0.5, 0.0)),
        Collider3D::cuboid(40.0, 1.0, 40.0),
    ));
    let runner = scene.spawn((
        Name::new("Runner"),
        Transform::from_position(Vec3::ZERO),
        MeshVisual::model(&runner_path(), [1.0; 4]),
        Animation::new("Idle"),
        ragdoll,
    ));
    let physics = PhysicsWorld3D::new();
    (scene, physics, runner)
}

/// One frame: animation, then a physics step (as the engine runs them).
fn frame(scene: &mut SceneWorld, physics: &mut PhysicsWorld3D) {
    propagate_transforms(scene);
    animate(scene, DT);
    physics.sync_from_scene(scene);
    physics.step(scene, DT);
}

fn frames(scene: &mut SceneWorld, physics: &mut PhysicsWorld3D, n: usize) {
    for _ in 0..n {
        frame(scene, physics);
    }
}

fn set_active(scene: &mut SceneWorld, entity: Entity, on: bool) {
    scene.get::<&mut Ragdoll>(entity).unwrap().active = on;
}

fn layout() -> RagdollLayout {
    let model = automata_assets::load(&runner_path()).unwrap();
    RagdollLayout::of(&model).unwrap()
}

/// Each simulated bone's world rotation, by node.
fn bone_rotation(scene: &SceneWorld, entity: Entity, node: usize) -> Quat {
    let state = scene.get::<&RagdollPose>(entity).unwrap();
    state
        .bones
        .iter()
        .find(|b| b.node == node)
        .unwrap()
        .rotation
}

/// How far each hinge has bent (radians, as its limits count it).
fn hinge_angles(
    scene: &SceneWorld,
    entity: Entity,
    layout: &RagdollLayout,
) -> Vec<(String, f32, f32, f32)> {
    let mut out = Vec::new();
    for body in &layout.bodies {
        let (Some(parent), Some(joint)) = (body.parent, body.joint) else {
            continue;
        };
        let RagdollJointKind::Hinge { min, max } = joint.kind else {
            continue;
        };
        let rp = bone_rotation(scene, entity, layout.bodies[parent].node);
        let rc = bone_rotation(scene, entity, body.node);
        let q = (rp * joint.frame_parent).inverse() * (rc * joint.frame_child);
        let q = if q.w < 0.0 { -q } else { q };
        out.push((body.name.clone(), 2.0 * q.x.atan2(q.w), min, max));
    }
    out
}

#[test]
fn a_limp_runner_falls_to_the_floor_and_its_knees_bend_only_backwards() {
    let (mut scene, mut physics, runner) = stage(Ragdoll::default());
    frames(&mut scene, &mut physics, 10);
    assert!(!physics.is_ragdoll(runner), "not limp until switched on");
    set_active(&mut scene, runner, true);
    // A shove backwards as it goes, so it falls rather than folds.
    automata_scene::push_ragdoll(
        &mut scene,
        runner,
        Some("Chest".into()),
        Vec3::new(0.0, 0.0, -40.0),
    );
    frame(&mut scene, &mut physics);
    let layout = layout();
    assert!(physics.is_ragdoll(runner));
    assert_eq!(physics.ragdoll_body_count(runner), layout.bodies.len());
    let start = physics.ragdoll_root(runner).unwrap();
    assert!(
        start.y > 0.7,
        "the hips start where the animation had them: {start}"
    );
    for _ in 0..240 {
        frame(&mut scene, &mut physics);
        for (name, angle, min, max) in hinge_angles(&scene, runner, &layout) {
            assert!(
                angle >= min - 0.15 && angle <= max + 0.15,
                "{name} bent {angle} outside {min}..{max}"
            );
        }
    }
    let hips = physics.ragdoll_root(runner).unwrap();
    assert!(hips.y < 0.4, "the hips came down: {hips}");
    let state = scene.get::<&RagdollPose>(runner).unwrap();
    assert!(state.simulating);
    for bone in &state.bones {
        assert!(
            bone.position.y > -0.05,
            "nothing sinks through the floor: {bone:?}"
        );
    }
    drop(state);
    assert!(physics.ragdoll_resting(runner), "it settles");
}

#[test]
fn switched_off_it_stands_where_it_fell_and_blends_back() {
    let (mut scene, mut physics, runner) = stage(Ragdoll {
        blend_time: 0.5,
        ..Ragdoll::default()
    });
    frames(&mut scene, &mut physics, 5);
    set_active(&mut scene, runner, true);
    automata_scene::push_ragdoll(&mut scene, runner, None, Vec3::new(140.0, 0.0, 0.0));
    frames(&mut scene, &mut physics, 150);
    let hips = physics.ragdoll_root(runner).unwrap();
    assert!(hips.x > 0.3, "the push carried it along: {hips}");
    set_active(&mut scene, runner, false);
    frame(&mut scene, &mut physics);
    assert!(!physics.is_ragdoll(runner));
    let at = scene.get::<&Transform>(runner).unwrap().position;
    assert!(
        (at.x - hips.x).abs() < 0.05 && (at.z - hips.z).abs() < 0.05,
        "{at} under {hips}"
    );
    assert!(at.y.abs() < 0.02, "standing on the floor: {at}");
    // The animation picks it up next frame.
    frame(&mut scene, &mut physics);
    let weight = scene.get::<&RagdollPose>(runner).unwrap().weight;
    assert!(weight > 0.5 && weight < 1.0, "blending back: {weight}");
    frames(&mut scene, &mut physics, 40);
    let state = scene.get::<&RagdollPose>(runner).unwrap();
    assert!(!state.showing(), "the animation has it again");
}

/// How far the bodies turned against their parents since going limp.
fn total_bend(scene: &SceneWorld, entity: Entity, layout: &RagdollLayout, from: &[Quat]) -> f32 {
    layout
        .bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let p = b.parent?;
            let rel = bone_rotation(scene, entity, layout.bodies[p].node).inverse()
                * bone_rotation(scene, entity, b.node);
            let rel0 = from[p].inverse() * from[i];
            Some(rel.angle_between(rel0))
        })
        .sum()
}

#[test]
fn stiff_ragdolls_hold_their_pose_better_than_limp_ones() {
    let layout = layout();
    let bend = |stiffness: f32| {
        let (mut scene, mut physics, runner) = stage(Ragdoll {
            active: true,
            stiffness,
            ..Ragdoll::default()
        });
        frame(&mut scene, &mut physics);
        let from: Vec<Quat> = layout
            .bodies
            .iter()
            .map(|b| bone_rotation(&scene, runner, b.node))
            .collect();
        frames(&mut scene, &mut physics, 90);
        total_bend(&scene, runner, &layout, &from)
    };
    let (limp, stiff) = (bend(0.0), bend(1.0));
    assert!(stiff < limp * 0.5, "stiff {stiff} vs limp {limp}");
}

#[test]
fn ragdoll_parts_are_hit_by_rays_as_the_entity() {
    let (mut scene, mut physics, runner) = stage(Ragdoll {
        active: true,
        ..Ragdoll::default()
    });
    frames(&mut scene, &mut physics, 2);
    let hips = physics.ragdoll_root(runner).unwrap();
    let hit = physics
        .cast_ray(
            hips + Vec3::new(0.0, 0.0, 3.0),
            Vec3::NEG_Z,
            10.0,
            &RayFilter::default(),
        )
        .expect("the ray hits the body");
    assert_eq!(hit.entity, runner);
}

#[test]
fn a_model_without_a_rig_just_stays_put() {
    let mut scene = SceneWorld::new();
    let crate_box = scene.spawn((
        Transform::from_position(Vec3::Y),
        Ragdoll {
            active: true,
            ..Ragdoll::default()
        },
    ));
    let mut physics = PhysicsWorld3D::new();
    frames(&mut scene, &mut physics, 3);
    assert!(!physics.is_ragdoll(crate_box));
}

#[test]
fn a_robot_standing_above_its_origin_stands_back_up_at_the_same_height() {
    let mut scene = SceneWorld::new();
    scene.spawn((
        Name::new("Floor"),
        Transform::from_position(Vec3::new(0.0, -0.5, 0.0)),
        Collider3D::cuboid(40.0, 1.0, 40.0),
    ));
    let path = format!(
        "{}/../../examples/oberon/robot_plaza/models/robot.glb",
        env!("CARGO_MANIFEST_DIR")
    );
    // The robot's origin is at its middle: it stands 0.89 m up.
    let robot = scene.spawn((
        Name::new("Robot"),
        Transform::from_position(Vec3::new(0.0, 0.89, 0.0)),
        MeshVisual::model(&path, [1.0; 4]),
        Animation::new("Idle"),
        Ragdoll::default(),
    ));
    let mut physics = PhysicsWorld3D::new();
    frames(&mut scene, &mut physics, 4);
    set_active(&mut scene, robot, true);
    automata_scene::push_ragdoll(&mut scene, robot, None, Vec3::new(160.0, 0.0, 0.0));
    let mut rested = None;
    for i in 0..400 {
        frame(&mut scene, &mut physics);
        let pose = scene.get::<&RagdollPose>(robot).unwrap();
        for bone in &pose.bones {
            assert!(bone.position.length() < 6.0, "nothing flies off: {bone:?}");
        }
        drop(pose);
        if physics.ragdoll_resting(robot) {
            rested = Some(i);
            break;
        }
    }
    assert!(rested.is_some(), "it comes to rest");
    let hips = physics.ragdoll_root(robot).unwrap();
    assert!(hips.x > 0.5, "the push knocked it over along +x: {hips}");
    set_active(&mut scene, robot, false);
    frames(&mut scene, &mut physics, 2);
    let at = scene.get::<&Transform>(robot).unwrap().position;
    assert!(
        (at.y - 0.89).abs() < 0.05,
        "it stands 0.89 m up again: {at}"
    );
    assert!((at.x - hips.x).abs() < 0.05, "where its hips were: {at}");
}

/// However it lands (pushed from every side), a ragdoll lies still within
/// a few seconds: no limb keeps twitching against a joint limit.
#[test]
fn a_ragdoll_comes_to_rest_however_it_lands() {
    let path = format!(
        "{}/../../examples/oberon/robot_plaza/models/robot.glb",
        env!("CARGO_MANIFEST_DIR")
    );
    for yaw in [-90.0f32, 90.0, 45.0, 0.0, 135.0, -45.0, 180.0, 20.0] {
        let mut scene = SceneWorld::new();
        scene.spawn((
            Name::new("Floor"),
            Transform::from_position(Vec3::new(0.0, -0.15, 0.0)),
            Collider3D::cuboid(40.0, 0.3, 40.0),
        ));
        let mut place = Transform::from_position(Vec3::new(0.7, 0.89, 3.0));
        place.rotation = Quat::from_rotation_y(yaw.to_radians());
        let robot = scene.spawn((
            Name::new("Robot"),
            place,
            MeshVisual::model(&path, [1.0; 4]),
            Animation::new("Idle"),
            Ragdoll::default(),
        ));
        let mut physics = PhysicsWorld3D::new();
        frames(&mut scene, &mut physics, 4);
        set_active(&mut scene, robot, true);
        automata_scene::push_ragdoll(&mut scene, robot, None, Vec3::new(220.0, 33.0, 0.0));
        let rested = (0..300).any(|_| {
            frame(&mut scene, &mut physics);
            physics.ragdoll_resting(robot)
        });
        assert!(rested, "facing {yaw}°, it's still twitching after 5 s");
    }
}
