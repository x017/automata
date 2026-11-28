//! Spring bones on a real model: Ponytail Park's runner (its ponytail and
//! skirt are spring chains set up in `runner.glb.import.ron`) runs, stops,
//! and the chains swing and settle.

use std::path::Path;

use automata_scene::{Animator, PlayOptions};
use glam::{Mat4, Vec3};

const MODEL: &str = "../../examples/oberon/ponytail_park/models/runner.glb";
const DT: f32 = 1.0 / 60.0;
const RUN_SPEED: f32 = 4.0;

fn runner() -> Animator {
    let model = automata_assets::load_file(Path::new(MODEL)).expect("runner.glb loads");
    let mut anim = Animator::for_model(&model).expect("the runner has a rig");
    assert!(anim.has_springs(), "its import settings add spring chains");
    anim.play("Idle");
    anim
}

/// Where the simulated tip of `bone` is, in the runner's own frame.
fn tip(anim: &Animator, bone: &str, world: Mat4) -> Vec3 {
    let node = anim.rig().node(bone).expect("bone exists");
    let (_, at) = anim
        .springs()
        .expect("springs")
        .tips()
        .into_iter()
        .find(|(n, _)| *n == node)
        .unwrap_or_else(|| panic!("{bone} swings"));
    world.inverse().transform_point3(at)
}

/// The skirt's hem, one tip per panel.
fn hem(anim: &Animator, world: Mat4) -> Vec<Vec3> {
    (0..8)
        .map(|i| tip(anim, &format!("Skirt{i}B"), world))
        .collect()
}

/// How far the hair is inside the head or chest colliders (0 if outside).
fn hair_inside_body(anim: &Animator, world: Mat4) -> f32 {
    let rig = anim.rig();
    let globals = rig.globals(anim.pose());
    let body = [("Head", 0.09, 0.13 + 0.03), ("Chest", 0.1, 0.14 + 0.03)];
    let mut worst = 0.0f32;
    for (bone, up, reach) in body {
        let centre = world.transform_point3(
            globals[rig.node(bone).unwrap()].transform_point3(Vec3::new(0.0, up, 0.0)),
        );
        // Hair1 hangs from inside the head; the rest must stay out.
        for name in ["Hair2", "Hair3", "Hair4", "Hair5"] {
            let at = tip(anim, name, Mat4::IDENTITY); // world space
            worst = worst.max(reach - at.distance(centre));
        }
    }
    worst
}

#[test]
fn the_ponytail_and_skirt_swing_as_the_runner_runs_and_settle_when_it_stops() {
    let mut anim = runner();
    let mut world = Mat4::IDENTITY;

    // Stand for 3 s: everything settles into the idle pose.
    for _ in 0..180 {
        anim.advance_in(DT, world);
    }
    let hair_still = tip(&anim, "Hair5", world);
    let hem_still = hem(&anim, world);

    // Run forwards (+Z) for 2.5 s.
    anim.play_with(
        "Run",
        PlayOptions {
            fade: 0.2,
            ..PlayOptions::default()
        },
    );
    let mut z = 0.0;
    let mut hair_run = Vec::new();
    let mut hem_run = Vec::new();
    let mut deepest = 0.0f32;
    for frame in 0..150 {
        z += RUN_SPEED * DT;
        world = Mat4::from_translation(Vec3::new(0.0, 0.0, z));
        anim.advance_in(DT, world);
        deepest = deepest.max(hair_inside_body(&anim, world));
        if frame >= 90 {
            hair_run.push(tip(&anim, "Hair5", world));
            hem_run.push(hem(&anim, world));
        }
    }
    let mean = |v: &[Vec3]| v.iter().copied().sum::<Vec3>() / v.len() as f32;
    let hair_running = mean(&hair_run);
    // The ponytail streams out behind (-Z) and higher than when standing.
    assert!(
        hair_running.z < hair_still.z - 0.04,
        "trails: {hair_running} vs {hair_still}"
    );
    assert!(
        hair_running.y > hair_still.y + 0.01,
        "lifts: {hair_running} vs {hair_still}"
    );
    // The hem swings: each panel's tip moves around with the stride.
    let swing = (0..8)
        .map(|i| {
            let (lo, hi) = hem_run.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| {
                (lo.min(h[i].y), hi.max(h[i].y))
            });
            hi - lo
        })
        .fold(0.0f32, f32::max);
    assert!(swing > 0.02, "the hem swings ({swing} m)");

    // Stop dead and stand there for 3 s.
    anim.play_with(
        "Idle",
        PlayOptions {
            fade: 0.2,
            ..PlayOptions::default()
        },
    );
    let mut forward = f32::MIN;
    let mut last_frames = Vec::new();
    for frame in 0..180 {
        anim.advance_in(DT, world);
        deepest = deepest.max(hair_inside_body(&anim, world));
        let hair = tip(&anim, "Hair5", world);
        if frame < 60 {
            forward = forward.max(hair.z);
        }
        if frame >= 170 {
            last_frames.push(hair);
        }
    }
    // Stopping throws the ponytail forwards, past where it hangs at rest...
    assert!(
        forward > hair_still.z + 0.01,
        "swings forward on stopping: {forward} vs {}",
        hair_still.z
    );
    // ...and it never goes through the head or back.
    assert!(deepest < 0.006, "hair {deepest} m inside the body");
    // Then everything settles back to the idle pose.
    let hair_end = tip(&anim, "Hair5", world);
    assert!(
        hair_end.distance(hair_still) < 0.015,
        "hair settles: {hair_end} vs {hair_still}"
    );
    let drift = last_frames
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .fold(0.0f32, f32::max);
    assert!(drift < 0.002, "hair still swinging {drift} m/frame");
    for (i, (now, still)) in hem(&anim, world).iter().zip(&hem_still).enumerate() {
        assert!(
            now.distance(*still) < 0.015,
            "skirt panel {i} settles: {now} vs {still}"
        );
    }
}

#[test]
fn a_runner_on_a_treadmill_has_no_wind_in_its_hair() {
    // Running on the spot: only the animation moves the hair, so it trails
    // far less than when the runner really moves.
    let mut anim = runner();
    anim.play("Run");
    let mut zs = Vec::new();
    for frame in 0..150 {
        anim.advance_in(DT, Mat4::IDENTITY);
        if frame >= 90 {
            zs.push(tip(&anim, "Hair5", Mat4::IDENTITY).z);
        }
    }
    let on_the_spot = zs.iter().sum::<f32>() / zs.len() as f32;

    let mut anim = runner();
    anim.play("Run");
    let mut zs = Vec::new();
    for frame in 0..150 {
        let world = Mat4::from_translation(Vec3::Z * RUN_SPEED * DT * (frame + 1) as f32);
        anim.advance_in(DT, world);
        if frame >= 90 {
            zs.push(tip(&anim, "Hair5", world).z);
        }
    }
    let moving = zs.iter().sum::<f32>() / zs.len() as f32;
    assert!(moving < on_the_spot - 0.03, "{moving} vs {on_the_spot}");
}
