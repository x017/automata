//! Ragdoll layouts worked out from the example models.

use std::path::Path;

use automata_scene::ragdoll::RagdollJointKind;
use automata_scene::RagdollLayout;

const PONYTAIL: &str = "../../examples/oberon/ponytail_park/models/runner.glb";
const RUNNER: &str = "../../examples/oberon/meadow/models/runner.glb";
const ROBOT: &str = "../../examples/oberon/robot_plaza/models/robot.glb";

fn layout(path: &str) -> RagdollLayout {
    let model = automata_assets::load_file(Path::new(path)).expect("model loads");
    RagdollLayout::of(&model).expect("a ragdoll")
}

fn names(l: &RagdollLayout) -> Vec<&str> {
    l.bodies.iter().map(|b| b.name.as_str()).collect()
}

fn hinges(l: &RagdollLayout) -> Vec<&str> {
    l.bodies
        .iter()
        .filter(|b| {
            matches!(
                b.joint.map(|j| j.kind),
                Some(RagdollJointKind::Hinge { .. })
            )
        })
        .map(|b| b.name.as_str())
        .collect()
}

/// What every layout must be: one root first, parents before children,
/// sensible capsules, and shares adding to 1.
fn check_shape(l: &RagdollLayout) {
    assert!(l.height > 0.5, "height {}", l.height);
    assert!(l.bodies[0].parent.is_none() && l.bodies[0].joint.is_none());
    for (i, b) in l.bodies.iter().enumerate().skip(1) {
        let parent = b.parent.expect("only the root has no parent");
        assert!(parent < i, "{} comes after its parent", b.name);
        assert!(b.joint.is_some(), "{} is joined to its parent", b.name);
    }
    for b in &l.bodies {
        assert!(
            b.radius > 0.0 && b.radius < l.height * 0.3,
            "{} radius {}",
            b.name,
            b.radius
        );
        assert!(
            b.share >= 0.02 && b.share <= 0.35,
            "{} share {}",
            b.name,
            b.share
        );
        assert!(b.a.is_finite() && b.b.is_finite());
    }
    let total: f32 = l.bodies.iter().map(|b| b.share).sum();
    assert!((total - 1.0).abs() < 1e-4, "shares add to {total}");
}

#[test]
fn the_runner_gets_fourteen_bodies_from_its_hips() {
    let l = layout(RUNNER);
    check_shape(&l);
    let names = names(&l);
    assert_eq!(names.len(), 14, "{names:?}");
    assert_eq!(names[0], "Hips");
    for bone in ["Spine", "Chest", "Head"] {
        assert!(names.contains(&bone), "{bone} in {names:?}");
    }
    for side in ["L", "R"] {
        for bone in ["Thigh", "Shin", "Foot", "UpperArm", "Forearm"] {
            assert!(
                names.iter().any(|n| n.contains(bone) && n.ends_with(side)),
                "{bone} {side} in {names:?}"
            );
        }
    }
    let mut bends = hinges(&l);
    bends.sort_unstable();
    assert_eq!(bends.len(), 4, "knees and elbows hinge: {bends:?}");
    assert!(bends
        .iter()
        .all(|n| n.contains("Shin") || n.contains("Forearm")));
}

#[test]
fn spring_bones_are_left_to_the_animation() {
    let l = layout(PONYTAIL);
    check_shape(&l);
    for b in &l.bodies {
        for spring in ["Hair", "Pony", "Skirt", "Tail"] {
            assert!(!b.name.contains(spring), "{} is a spring bone", b.name);
        }
    }
    assert_eq!(l.bodies.len(), 14);
}

#[test]
fn the_robot_gets_twelve_bodies_from_its_body() {
    let l = layout(ROBOT);
    check_shape(&l);
    let names = names(&l);
    assert_eq!(names.len(), 12, "{names:?}");
    assert_eq!(names[0], "Body");
    assert!(names.contains(&"Head"));
    for side in [".L", ".R"] {
        for bone in ["UpperArm", "LowerArm", "UpperLeg", "LowerLeg", "Foot"] {
            let want = format!("{bone}{side}");
            assert!(names.contains(&want.as_str()), "{want} in {names:?}");
        }
    }
    let mut bends = hinges(&l);
    bends.sort_unstable();
    assert_eq!(
        bends,
        ["LowerArm.L", "LowerArm.R", "LowerLeg.L", "LowerLeg.R"]
    );
}
