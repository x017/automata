//! Spring bone simulation: hair, skirts and tails that swing and settle on
//! top of the animation. The set-up comes from the model's import settings
//! ([`automata_assets::springs`]); [`crate::Animator`] runs it.
//!
//! Each swinging bone has a point at its tip, simulated in world space
//! with Verlet integration: it keeps its momentum (less `drag`), is pulled
//! back towards where the animation puts it (`stiffness`), falls with
//! `gravity`, is blown by `wind`, stays the bone's length from the bone's
//! origin, and is pushed out of the body's colliders. The bone is then
//! turned to point at it. Because the points live in world space, moving
//! the entity leaves them behind for a moment, which is the swing.

use automata_assets::anim::{Pose, Rig};
use automata_assets::springs::SpringChain;
use glam::{Mat4, Quat, Vec3};

/// The longest step simulated at once; longer frames are split.
const MAX_STEP: f32 = 1.0 / 60.0;
/// The most steps one frame is split into (slower frames run slow motion).
const MAX_STEPS: u8 = 4;
/// Spring rate at stiffness 1, as an angular frequency (rad/s).
const STIFF_OMEGA: f32 = 25.0;
/// Damping rate at drag 1 (velocity falls by e every 1/15 s).
const DRAG_RATE: f32 = 15.0;
/// Damping that comes with the spring, as a share of its rate. Each bone
/// of a chain shakes the next; lightly damped springs in a row pass a swing
/// down the chain and let it build up at the end, so springs always damp a
/// little of their own.
const SPRING_DAMPING: f32 = 0.4;
/// Gravity, m/s².
const GRAVITY: f32 = 9.81;
/// The push (m/s²) of the scene's wind at full strength.
const SCENE_WIND: f32 = 8.0;

#[derive(Debug, Clone)]
struct Joint {
    node: usize,
    chain: usize,
    /// The tip's node (its origin is the tip), or `None` for a made-up tip.
    child: Option<usize>,
    /// The made-up tip, in the node's own space (branch ends).
    tip: Vec3,
    prev: Vec3,
    cur: Vec3,
}

#[derive(Debug, Clone)]
struct Collider {
    node: usize,
    offset: Vec3,
    radius: f32,
    length: f32,
}

/// The running state of a model's spring bones.
#[derive(Debug, Clone)]
pub struct SpringSim {
    chains: Vec<SpringChain>,
    joints: Vec<Joint>,
    colliders: Vec<Collider>,
    started: bool,
    time: f32,
    last_world: Option<Mat4>,
    /// The scene's wind (direction times 0..1 strength).
    wind: Vec3,
}

impl SpringSim {
    /// The simulation for `rig`'s spring settings, or `None` when no chain
    /// names a bone the rig has.
    #[must_use]
    pub fn new(rig: &Rig) -> Option<Self> {
        let n = rig.nodes.len();
        let children = |i: usize| -> Vec<usize> {
            (0..n).filter(|&c| rig.nodes[c].parent == Some(i)).collect()
        };
        let mut owner: Vec<Option<usize>> = vec![None; n];
        for (ci, chain) in rig.springs.chains.iter().enumerate() {
            let Some(root) = rig.node(&chain.root) else {
                tracing::warn!(
                    "spring chain root {:?} isn't a bone of the model",
                    chain.root
                );
                continue;
            };
            // Parents come before children, so one pass finds the subtree.
            let mut inside = vec![false; n];
            inside[root] = true;
            for i in root + 1..n {
                inside[i] = rig.nodes[i].parent.is_some_and(|p| inside[p]);
            }
            for (i, inside) in inside.into_iter().enumerate() {
                if inside && owner[i].is_none() {
                    owner[i] = Some(ci);
                }
            }
        }
        let mut joints = Vec::new();
        for (i, chain) in owner.iter().enumerate() {
            let Some(chain) = *chain else { continue };
            let kids = children(i);
            let (child, tip) = match kids.as_slice() {
                [only] => (Some(*only), Vec3::ZERO),
                [] => {
                    // A branch end: continue the bone half as far again.
                    let rest = rig.nodes[i].rest;
                    let tip = rest.rotation.inverse() * rest.translation * 0.5;
                    if tip.length_squared() < 1e-12 {
                        continue;
                    }
                    (None, tip)
                }
                // A fork (a skirt's waistband) stays put; its branches swing.
                _ => continue,
            };
            joints.push(Joint {
                node: i,
                chain,
                child,
                tip,
                prev: Vec3::ZERO,
                cur: Vec3::ZERO,
            });
        }
        if joints.is_empty() {
            return None;
        }
        let colliders = rig
            .springs
            .colliders
            .iter()
            .filter_map(|c| {
                let node = rig.node(&c.bone);
                if node.is_none() {
                    tracing::warn!(
                        "spring collider bone {:?} isn't a bone of the model",
                        c.bone
                    );
                }
                Some(Collider {
                    node: node?,
                    offset: Vec3::from(c.offset),
                    radius: c.radius.max(0.0),
                    length: c.length,
                })
            })
            .collect();
        Some(Self {
            chains: rig.springs.chains.clone(),
            joints,
            colliders,
            started: false,
            time: 0.0,
            last_world: None,
            wind: Vec3::ZERO,
        })
    }

    /// Sets the scene's wind, a horizontal direction times its strength
    /// (0..1, as [`automata_renderer::Environment::wind_vector`]), which
    /// blows every chain on top of its own `wind`.
    pub fn set_wind(&mut self, wind: Vec3) {
        self.wind = wind;
    }

    /// Forgets the motion: the next update starts at rest (after a
    /// teleport, say).
    pub fn reset(&mut self) {
        self.started = false;
        self.last_world = None;
    }

    /// Advances `dt` seconds with the entity at `world`, turning the
    /// swinging bones of `pose` (the animated pose) to follow their tips.
    /// `dt` 0 only re-applies the current tips.
    pub fn update(&mut self, rig: &Rig, pose: &mut Pose, world: Mat4, dt: f32) {
        let animated = pose.clone();
        let from = self.last_world.unwrap_or(world);
        // Enough steps of at most MAX_STEP, up to MAX_STEPS.
        let mut steps: u8 = 1;
        while steps < MAX_STEPS && dt > MAX_STEP * f32::from(steps) {
            steps += 1;
        }
        let h = if dt > 0.0 {
            (dt / f32::from(steps)).min(MAX_STEP)
        } else {
            0.0
        };
        for k in 1..=steps {
            // Slide the entity across the frame, so fast moves stay smooth.
            let at = lerp_world(from, world, f32::from(k) / f32::from(steps));
            *pose = animated.clone();
            self.step(rig, pose, at, h);
            self.time += h;
        }
        if dt > 0.0 {
            self.last_world = Some(world);
        }
    }

    fn step(&mut self, rig: &Rig, pose: &mut Pose, world: Mat4, h: f32) {
        let model = world * rig.import;
        let scale = uniform_scale(world);
        let mut globals = rig.globals(pose);
        // Each tip springs towards where the animation puts it. Aiming at
        // the swinging parent instead turns a long chain into a row of
        // springs each shaking the next at its own pace, and the end of
        // the chain whips about for seconds.
        let animated = globals.clone();
        // Colliders sit on body bones, which the chains don't move.
        let colliders: Vec<(Vec3, Vec3, f32)> = self
            .colliders
            .iter()
            .filter_map(|c| {
                let m = model * *globals.get(c.node)?;
                let (_, rot, pos) = m.to_scale_rotation_translation();
                let a = pos + rot * (c.offset * scale);
                let b = a + rot * (Vec3::Y * c.length * scale);
                Some((a, b, c.radius * scale))
            })
            .collect();
        let first = !self.started;
        self.started = true;
        for j in &mut self.joints {
            let chain = &self.chains[j.chain];
            let m = model * globals[j.node];
            let head = m.transform_point3(Vec3::ZERO);
            let tip = j.child.map_or(j.tip, |c| pose.0[c].translation);
            let target = m.transform_point3(tip);
            let goal = (model * animated[j.node]).transform_point3(tip);
            let len = (target - head).length();
            if len < 1e-6 {
                continue;
            }
            // Start at rest, and again after a teleport.
            if first || j.cur.distance(target) > 2.0 * len + 1.0 {
                j.prev = target;
                j.cur = target;
            }
            let mut next = j.cur;
            if h > 0.0 {
                let omega = chain.stiffness.clamp(0.0, 1.0) * STIFF_OMEGA;
                let rate = chain.drag.clamp(0.0, 1.0) * DRAG_RATE + SPRING_DAMPING * omega;
                let damp = (-rate * h).exp();
                let t = self.time;
                let phase = j.node as f32 * 0.37;
                let gust = 1.0 + 0.6 * (t * 1.3 + phase).sin() * (t * 0.7).sin();
                let accel = (goal - j.cur) * omega * omega
                    + Vec3::NEG_Y * GRAVITY * chain.gravity
                    + (Vec3::from(chain.wind) + self.wind * SCENE_WIND) * gust;
                next = j.cur + (j.cur - j.prev) * damp + accel * h * h;
            }
            let rest_dir = (target - head) / len;
            next = head + (next - head).try_normalize().unwrap_or(rest_dir) * len;
            let thick = chain.radius.max(0.0) * scale;
            // Two passes settle tips caught between two colliders.
            for _ in 0..2 {
                for &(a, b, r) in &colliders {
                    let q = closest_on_segment(a, b, next);
                    let reach = r + thick;
                    if next.distance_squared(q) < reach * reach {
                        next = push_out(head, len, next, q, reach);
                    }
                }
            }
            if h > 0.0 {
                j.prev = j.cur;
                j.cur = next;
            }
            // Turn the bone (in its parent's frame) to point at the tip.
            let to = (next - head).try_normalize().unwrap_or(rest_dir);
            let turn = Quat::from_rotation_arc(rest_dir, to);
            let parent = rig.nodes[j.node]
                .parent
                .map_or(model, |p| model * globals[p]);
            let (_, parent_rot, _) = parent.to_scale_rotation_translation();
            let local = &mut pose.0[j.node].rotation;
            *local = (parent_rot.inverse() * turn * parent_rot * *local).normalize();
            refresh_globals(rig, pose, &mut globals, j.node);
        }
    }

    /// The simulated tips, in world space (for tests and gizmos).
    #[must_use]
    pub fn tips(&self) -> Vec<(usize, Vec3)> {
        self.joints.iter().map(|j| (j.node, j.cur)).collect()
    }
}

/// Moves `tip`, which is `len` from `head`, out of the sphere at `centre`
/// while keeping it `len` from `head`: to the nearest point of the circle
/// where the two spheres meet.
///
/// Pushing straight away from the centre and then fixing the length isn't
/// the same: when `head` is inside the sphere (hair roots on a head) that
/// pushes sideways drift further sideways every step, and the chain orbits
/// instead of settling.
fn push_out(head: Vec3, len: f32, tip: Vec3, centre: Vec3, reach: f32) -> Vec3 {
    let axis = centre - head;
    let d = axis.length();
    if d < 1e-6 {
        return tip; // the bone pivots on the centre: no way out
    }
    let u = axis / d;
    // The circle's centre is `a` along the axis; its radius is `rho`.
    let a = (d * d + len * len - reach * reach) / (2.0 * d);
    let rho2 = len * len - a * a;
    if rho2 <= 0.0 {
        // Too short to get out: point as far from the centre as it can.
        return if d + len <= reach {
            head - u * len
        } else {
            tip
        };
    }
    let off = tip - head;
    let side = (off - u * off.dot(u))
        .try_normalize()
        .unwrap_or_else(|| u.any_orthonormal_vector());
    head + u * a + side * rho2.sqrt()
}

/// Recomputes the model-space transforms of `from` and every node after it
/// (children come after their parents, so that covers its subtree).
fn refresh_globals(rig: &Rig, pose: &Pose, globals: &mut [Mat4], from: usize) {
    for i in from..rig.nodes.len() {
        let local = pose.0.get(i).unwrap_or(&rig.nodes[i].rest).matrix();
        globals[i] = match rig.nodes[i].parent {
            Some(p) if p < i => globals[p] * local,
            _ => local,
        };
    }
}

fn uniform_scale(m: Mat4) -> f32 {
    (m.x_axis.truncate().length() + m.y_axis.truncate().length() + m.z_axis.truncate().length())
        / 3.0
}

fn lerp_world(a: Mat4, b: Mat4, t: f32) -> Mat4 {
    if t >= 1.0 || a == b {
        return b;
    }
    let (sa, ra, ta) = a.to_scale_rotation_translation();
    let (sb, rb, tb) = b.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(sa.lerp(sb, t), ra.slerp(rb, t), ta.lerp(tb, t))
}

fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-12 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)] // small frame counts
mod tests {
    use super::*;
    use automata_assets::anim::{RigNode, Trs};
    use automata_assets::springs::{SpringCollider, Springs};

    fn node(name: &str, parent: Option<usize>, at: Vec3) -> RigNode {
        RigNode {
            name: name.into(),
            parent,
            rest: Trs {
                translation: at,
                ..Trs::IDENTITY
            },
        }
    }

    /// A body with a tail sticking out backwards (-Z): Tail1 → Tail2 →
    /// Tip, half a metre per bone.
    fn tail(chain: SpringChain, colliders: Vec<SpringCollider>) -> Rig {
        let nodes = vec![
            node("Body", None, Vec3::ZERO),
            node("Tail1", Some(0), Vec3::ZERO),
            node("Tail2", Some(1), Vec3::new(0.0, 0.0, -0.5)),
            node("Tip", Some(2), Vec3::new(0.0, 0.0, -0.5)),
        ];
        Rig {
            joints: (0..nodes.len()).map(|i| (i, Mat4::IDENTITY)).collect(),
            nodes,
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: Springs {
                chains: vec![chain],
                colliders,
            },
        }
    }

    fn tip_of(rig: &Rig, pose: &Pose, world: Mat4) -> Vec3 {
        (world * rig.globals(pose)[3]).transform_point3(Vec3::ZERO)
    }

    /// Runs `seconds` at 60 fps with the entity at `place(t)`; returns the
    /// tip's last position.
    fn run(
        rig: &Rig,
        sim: &mut SpringSim,
        seconds: f32,
        place: impl Fn(f32) -> Vec3,
        start: f32,
    ) -> (Vec3, Pose) {
        let mut pose = rig.rest_pose();
        let frames = (seconds * 60.0).round() as usize;
        let mut world = Mat4::IDENTITY;
        for f in 1..=frames {
            world = Mat4::from_translation(place(start + f as f32 / 60.0));
            pose = rig.rest_pose();
            sim.update(rig, &mut pose, world, 1.0 / 60.0);
        }
        (tip_of(rig, &pose, world), pose)
    }

    #[test]
    fn a_long_chain_settles_instead_of_whipping() {
        // Six bones hanging down: a ponytail. A jolt has to die away
        // rather than build up along the chain (each bone's swing shakes
        // the next one).
        let mut nodes = vec![node("Head", None, Vec3::ZERO)];
        for i in 1..=6 {
            nodes.push(node(
                &format!("Hair{i}"),
                Some(i - 1),
                Vec3::new(0.0, -0.1, -0.02),
            ));
        }
        let rig = Rig {
            joints: (0..nodes.len()).map(|i| (i, Mat4::IDENTITY)).collect(),
            nodes,
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: Springs {
                chains: vec![SpringChain {
                    stiffness: 0.5,
                    drag: 0.25,
                    gravity: 0.6,
                    ..SpringChain::new("Hair1")
                }],
                colliders: Vec::new(),
            },
        };
        let mut sim = SpringSim::new(&rig).unwrap();
        // Dash a metre sideways in a quarter of a second, then stand still.
        let dash = |t: f32| Vec3::new((t * 4.0).min(1.0), 0.0, 0.0);
        let end = |sim: &SpringSim| sim.tips().last().unwrap().1;
        run(&rig, &mut sim, 0.25, dash, 0.0);
        let mut fastest = 0.0f32;
        let mut speeds = Vec::new();
        let mut before = end(&sim);
        for f in 0..150 {
            run(&rig, &mut sim, 1.0 / 60.0, dash, 0.25 + f as f32 / 60.0);
            let speed = end(&sim).distance(before) * 60.0;
            before = end(&sim);
            fastest = fastest.max(speed);
            speeds.push(speed);
        }
        assert!(fastest > 0.5, "the dash swings it: {fastest} m/s");
        let late = speeds[120..].iter().copied().fold(0.0f32, f32::max);
        assert!(late < 0.05, "still whipping at 2 s: {late} m/s");
    }

    #[test]
    fn a_limp_chain_hangs_under_gravity() {
        let limp = SpringChain {
            stiffness: 0.0,
            gravity: 1.0,
            drag: 0.2,
            ..SpringChain::new("Tail1")
        };
        let rig = tail(limp, Vec::new());
        let mut sim = SpringSim::new(&rig).unwrap();
        // Tail1 and Tail2 swing; Tip is a branch end with a made-up tip.
        assert_eq!(sim.tips().len(), 3);
        let (tip, _) = run(&rig, &mut sim, 4.0, |_| Vec3::ZERO, 0.0);
        assert!(tip.y < -0.95, "hangs straight down: {tip}");
        assert!(tip.z.abs() < 0.1, "{tip}");
    }

    #[test]
    fn a_stiff_chain_swings_behind_a_move_and_settles() {
        let stiff = SpringChain {
            stiffness: 0.6,
            gravity: 0.0,
            drag: 0.3,
            ..SpringChain::new("Tail1")
        };
        let rig = tail(stiff, Vec::new());
        let mut sim = SpringSim::new(&rig).unwrap();
        // Run sideways (+X) at 4 m/s for half a second.
        let run_x = |t: f32| Vec3::new(4.0 * t.min(0.5), 0.0, 0.0);
        let mut lag = 0.0f32;
        for f in 0..24 {
            let t = f as f32 / 60.0;
            let (tip, _) = run(&rig, &mut sim, 1.0 / 60.0, run_x, t);
            lag = lag.max(run_x(t + 1.0 / 60.0).x - tip.x);
        }
        assert!(lag > 0.1, "trails behind the body as it sets off: {lag}");
        // Stop: it swings on past, then settles back to straight out behind.
        let mut overshoot = 0.0f32;
        let mut largest_after = 0.0f32;
        let mut last = Vec3::ZERO;
        for i in 0..64 {
            let (tip, _) = run(&rig, &mut sim, 1.0 / 16.0, run_x, 0.4 + i as f32 / 16.0);
            overshoot = overshoot.max(tip.x - 2.0);
            if i >= 48 {
                largest_after = largest_after.max((tip.x - 2.0).abs());
            }
            last = tip;
        }
        assert!(
            overshoot > 0.05,
            "swings past when the body stops: {overshoot}"
        );
        assert!(largest_after < 0.02, "settled: {last}");
        assert!(last.distance(Vec3::new(2.0, 0.0, -1.0)) < 0.02, "{last}");
    }

    #[test]
    fn colliders_keep_chains_out_of_the_body() {
        let limp = SpringChain {
            stiffness: 0.0,
            gravity: 1.0,
            radius: 0.05,
            ..SpringChain::new("Tail1")
        };
        // A ball under the middle of the tail.
        let ball = SpringCollider {
            bone: "Body".into(),
            offset: [0.0, -0.4, -0.5],
            radius: 0.3,
            length: 0.0,
        };
        let rig = tail(limp, vec![ball]);
        let mut sim = SpringSim::new(&rig).unwrap();
        let (_, pose) = run(&rig, &mut sim, 4.0, |_| Vec3::ZERO, 0.0);
        let g = rig.globals(&pose);
        let centre = Vec3::new(0.0, -0.4, -0.5);
        for (i, m) in g.iter().enumerate().skip(2) {
            let p = m.transform_point3(Vec3::ZERO);
            assert!(
                p.distance(centre) >= 0.35 - 3e-3,
                "bone {i} at {p} is in the ball"
            );
        }
        // A capsule works the same way: one running down from the body.
        let capsule = SpringCollider {
            bone: "Body".into(),
            offset: [0.0, -0.2, -0.3],
            radius: 0.2,
            length: -2.0,
        };
        let rig = tail(
            SpringChain {
                stiffness: 0.0,
                gravity: 1.0,
                ..SpringChain::new("Tail1")
            },
            vec![capsule],
        );
        let mut sim = SpringSim::new(&rig).unwrap();
        let (tip, _) = run(&rig, &mut sim, 4.0, |_| Vec3::ZERO, 0.0);
        assert!(tip.z < -0.5 + 1e-3, "rests against the capsule: {tip}");
    }

    #[test]
    fn forks_stay_put_and_unknown_bones_are_skipped() {
        // Skirt has two branches: it doesn't swing, each branch does.
        let nodes = vec![
            node("Hips", None, Vec3::ZERO),
            node("Skirt", Some(0), Vec3::ZERO),
            node("Front", Some(1), Vec3::new(0.0, 0.0, 0.2)),
            node("FrontEnd", Some(2), Vec3::new(0.0, -0.3, 0.0)),
            node("Back", Some(1), Vec3::new(0.0, 0.0, -0.2)),
            node("BackEnd", Some(4), Vec3::new(0.0, -0.3, 0.0)),
        ];
        let mut rig = Rig {
            joints: (0..nodes.len()).map(|i| (i, Mat4::IDENTITY)).collect(),
            nodes,
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: Springs {
                chains: vec![SpringChain::new("Skirt"), SpringChain::new("Nope")],
                colliders: vec![SpringCollider::new("Nope")],
            },
        };
        let sim = SpringSim::new(&rig).unwrap();
        let nodes: Vec<usize> = sim.tips().iter().map(|(n, _)| *n).collect();
        assert_eq!(nodes, [2, 3, 4, 5]);
        rig.springs.chains = vec![SpringChain::new("Nope")];
        assert!(SpringSim::new(&rig).is_none());
    }

    #[test]
    fn a_still_frame_keeps_the_swing_and_teleports_reset_it() {
        let rig = tail(
            SpringChain {
                stiffness: 0.0,
                gravity: 1.0,
                ..SpringChain::new("Tail1")
            },
            Vec::new(),
        );
        let mut sim = SpringSim::new(&rig).unwrap();
        let (hanging, _) = run(&rig, &mut sim, 3.0, |_| Vec3::ZERO, 0.0);
        let mut pose = rig.rest_pose();
        sim.update(&rig, &mut pose, Mat4::IDENTITY, 0.0);
        assert!(tip_of(&rig, &pose, Mat4::IDENTITY).distance(hanging) < 1e-3);
        // Jumping 50 m starts over from the animated pose.
        let far = Mat4::from_translation(Vec3::new(50.0, 0.0, 0.0));
        let mut pose = rig.rest_pose();
        sim.update(&rig, &mut pose, far, 1.0 / 60.0);
        let tip = tip_of(&rig, &pose, far);
        assert!(tip.distance(Vec3::new(50.0, 0.0, -1.0)) < 0.05, "{tip}");
    }
}
