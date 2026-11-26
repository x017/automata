//! Inverse kinematics on an animated pose: hands reaching for things,
//! heads turning to look, feet standing on slopes and stairs.
//!
//! Everything here works on a rig's local [`Pose`] after the clips (or
//! the animation graph) have set it, in the rig's own space, and changes
//! only rotations (and, for feet that aren't children of the shin, the
//! foot's position):
//!
//! - [`solve_two_bone`] bends a limb (upper, middle, end) so the end
//!   reaches a point, keeping the bend the animation gave it (an elbow
//!   stays an elbow). The end can also hang elsewhere in the skeleton, as
//!   in rigs exported with IK controls where the foot isn't the shin's
//!   child: then it's moved to the shin's tip.
//! - [`look_at`] turns a bone (a head, shared with its neck) towards a
//!   point, as far as a limit.
//! - [`Ik`] is an animator's IK: the hands reaching for targets, where the
//!   head looks, and whether the feet follow the ground ([`Ik::apply`]).
//!
//! Limbs and heads are found by name ([`find_legs`], [`find_head`]):
//! `UpperLeg.L` / `LowerLeg.L` / `Foot.L`, `LeftUpLeg` / `LeftLeg` /
//! `LeftFoot` (Mixamo), `thigh_l` / `calf_l` / `foot_l` and the like.

use automata_assets::{Pose, Rig};
use glam::{Mat4, Quat, Vec3};

/// A limb of three bones: the end reaches, the middle bends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limb {
    /// The bone the limb hangs from (a thigh, an upper arm).
    pub upper: usize,
    /// The bending bone (a shin, a forearm).
    pub mid: usize,
    /// The bone that reaches (a foot, a hand).
    pub end: usize,
}

impl Limb {
    /// The limb ending at `end`: its parent and grandparent.
    #[must_use]
    pub fn ending_at(rig: &Rig, end: usize) -> Option<Self> {
        let mid = rig.nodes.get(end)?.parent?;
        let upper = rig.nodes[mid].parent?;
        Some(Self { upper, mid, end })
    }

    /// Whether `end` hangs below `mid` (so it follows the bend by itself).
    fn end_follows(&self, rig: &Rig) -> bool {
        let mut at = rig.nodes[self.end].parent;
        while let Some(p) = at {
            if p == self.mid {
                return true;
            }
            at = rig.nodes[p].parent;
        }
        false
    }
}

/// The side letter of a camel-case name (`ThighL`, `ForearmR`): `L` or
/// `R` straight after a lower-case letter, ending the name.
fn camel_side(name: &str) -> Option<bool> {
    let n = name.rsplit(':').next().unwrap_or(name);
    let mut chars = n.chars().rev();
    let last = chars.next()?;
    let before = chars.next()?;
    if !before.is_ascii_lowercase() {
        return None;
    }
    match last {
        'L' => Some(true),
        'R' => Some(false),
        _ => None,
    }
}

/// Which side a bone is on, from its name: `Some(true)` left,
/// `Some(false)` right.
pub(crate) fn side(name: &str) -> Option<bool> {
    if let Some(left) = camel_side(name) {
        return Some(left);
    }
    let n = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();
    let ends = |s: &str| {
        [".", "_", "-", " "]
            .iter()
            .any(|sep| n.ends_with(&format!("{sep}{s}")))
    };
    let starts = |s: &str| {
        [".", "_", "-", " "]
            .iter()
            .any(|sep| n.starts_with(&format!("{s}{sep}")))
    };
    if n.contains("left") || ends("l") || starts("l") {
        Some(true)
    } else if n.contains("right") || ends("r") || starts("r") {
        Some(false)
    } else {
        None
    }
}

/// A bone name without its side and separators: `UpperLeg.L` →
/// `upperleg`, `mixamorig:LeftUpLeg` → `upleg`, `thigh_l` → `thigh`,
/// `ShinL` → `shin`.
pub(crate) fn base(name: &str) -> String {
    let n = name.rsplit(':').next().unwrap_or(name);
    let n = if camel_side(n).is_some() {
        &n[..n.len() - 1]
    } else {
        n
    };
    let n = n.to_ascii_lowercase();
    let mut n = n.replace("left", "").replace("right", "");
    for sep in [".", "_", "-", " "] {
        for s in ["l", "r"] {
            if let Some(stripped) = n.strip_suffix(&format!("{sep}{s}")) {
                n = stripped.to_string();
            }
            if let Some(stripped) = n.strip_prefix(&format!("{s}{sep}")) {
                n = stripped.to_string();
            }
        }
    }
    n.chars().filter(char::is_ascii_alphanumeric).collect()
}

const FEET: [&str; 2] = ["foot", "ankle"];
const SHINS: [&str; 6] = ["lowerleg", "leg", "calf", "shin", "knee", "lowleg"];
const THIGHS: [&str; 4] = ["upperleg", "upleg", "thigh", "upleg1"];

/// The legs (left first), found by bone names. A rig whose feet are the
/// shins' children gets its limbs from the skeleton; one whose feet hang
/// elsewhere (IK-control exports) is matched by name and side.
#[must_use]
pub fn find_legs(rig: &Rig) -> Vec<Limb> {
    let is = |i: usize, names: &[&str]| names.contains(&base(&rig.nodes[i].name).as_str());
    let mut out = Vec::new();
    let mut globals: Option<Vec<Mat4>> = None;
    for left in [true, false] {
        let on_side = |i: &usize| side(&rig.nodes[*i].name) == Some(left);
        let Some(end) = (0..rig.nodes.len()).filter(on_side).find(|&i| is(i, &FEET)) else {
            continue;
        };
        if let Some(limb) = Limb::ending_at(rig, end) {
            if is(limb.mid, &SHINS) && is(limb.upper, &THIGHS) {
                out.push(limb);
                continue;
            }
        }
        // Of the shins hanging from a thigh, the one whose bones reach
        // nearest the foot (IK-control rigs also hang helper bones named
        // like legs off the thigh).
        let rest = globals.get_or_insert_with(|| rig.globals(&rig.rest_pose()));
        let foot = position(&rest[end]);
        let reach = |mid: usize| {
            (0..rig.nodes.len())
                .filter(|&i| descends_from(rig, i, mid))
                .map(|i| position(&rest[i]).distance(foot))
                .fold(f32::INFINITY, f32::min)
        };
        let mid = (0..rig.nodes.len())
            .filter(on_side)
            .filter(|&i| is(i, &SHINS) && rig.nodes[i].parent.is_some_and(|u| is(u, &THIGHS)))
            .min_by(|&a, &b| reach(a).total_cmp(&reach(b)));
        let Some(mid) = mid else { continue };
        let Some(upper) = rig.nodes[mid].parent else {
            continue;
        };
        out.push(Limb { upper, mid, end });
    }
    out
}

/// Whether `node` is `ancestor` or hangs somewhere below it.
fn descends_from(rig: &Rig, node: usize, ancestor: usize) -> bool {
    let mut at = Some(node);
    for _ in 0..=rig.nodes.len() {
        match at {
            Some(i) if i == ancestor => return true,
            Some(i) => at = rig.nodes[i].parent,
            None => return false,
        }
    }
    false
}

/// The head (and the neck it shares turning with, if it has one).
#[must_use]
pub fn find_head(rig: &Rig) -> Option<(usize, Option<usize>)> {
    let head = (0..rig.nodes.len()).find(|&i| base(&rig.nodes[i].name) == "head")?;
    let neck = rig.nodes[head]
        .parent
        .filter(|&p| base(&rig.nodes[p].name).starts_with("neck"));
    Some((head, neck))
}

fn position(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

fn rotation(m: &Mat4) -> Quat {
    let (_, r, _) = m.to_scale_rotation_translation();
    r.normalize()
}

/// The global rotation of `bone`'s parent (identity for a root).
fn parent_rotation(rig: &Rig, globals: &[Mat4], bone: usize) -> Quat {
    rig.nodes[bone]
        .parent
        .map_or(Quat::IDENTITY, |p| rotation(&globals[p]))
}

/// Turns `bone` by `turn` (rig space), keeping what's below it attached.
fn turn_bone(rig: &Rig, pose: &mut Pose, globals: &[Mat4], bone: usize, turn: Quat) {
    let global = turn * rotation(&globals[bone]);
    let local = parent_rotation(rig, globals, bone).inverse() * global;
    pose.0[bone].rotation = local.normalize();
}

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    a.normalize_or_zero()
        .dot(b.normalize_or_zero())
        .clamp(-1.0, 1.0)
        .acos()
}

/// Bends `limb` so its end reaches `target` (rig space), as far as it's
/// long enough to. The bend keeps the side the animation gave it; a limb
/// held dead straight bends towards `pole` (rig space), which defaults
/// to forward (+Z). With `end_turn`, the end's own rotation is turned by
/// it afterwards (a foot tilting to a slope).
///
/// `globals` must be the pose's ([`Rig::globals`]); it's brought up to
/// date. Returns whether the limb could be solved.
#[allow(clippy::similar_names, clippy::many_single_char_names)]
pub fn solve_two_bone(
    rig: &Rig,
    pose: &mut Pose,
    globals: &mut Vec<Mat4>,
    limb: Limb,
    target: Vec3,
    pole: Option<Vec3>,
    end_turn: Option<Quat>,
) -> bool {
    let n = rig.nodes.len();
    if limb.upper >= n || limb.mid >= n || limb.end >= n || pose.0.len() < n {
        return false;
    }
    let follows = limb.end_follows(rig);
    let a = position(&globals[limb.upper]);
    let b = position(&globals[limb.mid]);
    // The tip: where the end sits, as a point carried by the middle bone.
    let tip_local = globals[limb.mid]
        .inverse()
        .transform_point3(position(&globals[limb.end]));
    let c = globals[limb.mid].transform_point3(tip_local);
    let (lab, lcb) = (a.distance(b), b.distance(c));
    if lab < 1e-5 || lcb < 1e-5 || !target.is_finite() {
        return false;
    }
    let eps = (lab + lcb) * 1e-3;
    let lat = a.distance(target).clamp(eps, lab + lcb - eps);
    let ac_ab_0 = angle_between(c - a, b - a);
    let ba_bc_0 = angle_between(a - b, c - b);
    let cos_a = (lcb * lcb - lab * lab - lat * lat) / (-2.0 * lab * lat);
    let cos_b = (lat * lat - lab * lab - lcb * lcb) / (-2.0 * lab * lcb);
    let ac_ab_1 = cos_a.clamp(-1.0, 1.0).acos();
    let ba_bc_1 = cos_b.clamp(-1.0, 1.0).acos();
    // The bend's axis: across the plane the limb bends in.
    let bend = (c - a).cross(b - a);
    let axis0 = if bend.length_squared() > 1e-10 {
        bend.normalize()
    } else {
        (c - a)
            .cross(pole.unwrap_or(Vec3::Z))
            .try_normalize()
            .unwrap_or(Vec3::X)
    };
    let (ga, gb) = (rotation(&globals[limb.upper]), rotation(&globals[limb.mid]));
    let r0 = Quat::from_axis_angle(ga.inverse() * axis0, ac_ab_1 - ac_ab_0);
    let r1 = Quat::from_axis_angle(gb.inverse() * axis0, ba_bc_1 - ba_bc_0);
    let end_global = rotation(&globals[limb.end]);
    pose.0[limb.upper].rotation = (pose.0[limb.upper].rotation * r0).normalize();
    pose.0[limb.mid].rotation = (pose.0[limb.mid].rotation * r1).normalize();
    *globals = rig.globals(pose);
    // Bent to the right length; now swing the whole limb onto the target.
    let c = globals[limb.mid].transform_point3(tip_local);
    let swing = Quat::from_rotation_arc(
        (c - a).normalize_or_zero(),
        (target - a).normalize_or_zero(),
    );
    if (c - a).length_squared() > 1e-12 && (target - a).length_squared() > 1e-12 {
        turn_bone(rig, pose, globals, limb.upper, swing);
    }
    *globals = rig.globals(pose);
    if !follows {
        // A foot that isn't the shin's child: put it at the shin's tip,
        // facing as it did.
        let tip = globals[limb.mid].transform_point3(tip_local);
        let parent = rig.nodes[limb.end]
            .parent
            .map_or(Mat4::IDENTITY, |p| globals[p]);
        pose.0[limb.end].translation = parent.inverse().transform_point3(tip);
        let local = parent_rotation(rig, globals, limb.end).inverse() * end_global;
        pose.0[limb.end].rotation = local.normalize();
        *globals = rig.globals(pose);
    }
    if let Some(turn) = end_turn {
        let end = if follows {
            rotation(&globals[limb.end])
        } else {
            end_global
        };
        let local = parent_rotation(rig, globals, limb.end).inverse() * (turn * end);
        pose.0[limb.end].rotation = local.normalize();
        *globals = rig.globals(pose);
    }
    true
}

/// Turns `bone` so the way it faced in the rest pose along `forward` (rig
/// space) points at `target`, by `weight` (0 to 1) and at most
/// `max_angle` radians. Returns the angle it still lacked (0 when it got
/// there).
pub fn look_at(
    rig: &Rig,
    pose: &mut Pose,
    globals: &mut Vec<Mat4>,
    bone: usize,
    target: Vec3,
    (forward, weight, max_angle): (Vec3, f32, f32),
) -> f32 {
    if bone >= rig.nodes.len() || !target.is_finite() || weight <= 0.0 {
        return 0.0;
    }
    let rest = rig.globals(&rig.rest_pose());
    let facing_local = rotation(&rest[bone]).inverse() * forward;
    let facing = rotation(&globals[bone]) * facing_local;
    let wanted = (target - position(&globals[bone])).normalize_or_zero();
    if wanted == Vec3::ZERO || facing.length_squared() < 1e-8 {
        return 0.0;
    }
    let full = Quat::from_rotation_arc(facing.normalize(), wanted);
    let (axis, angle) = full.to_axis_angle();
    let reach = angle.min(max_angle) * weight.clamp(0.0, 1.0);
    turn_bone(rig, pose, globals, bone, Quat::from_axis_angle(axis, reach));
    *globals = rig.globals(pose);
    (angle - reach).max(0.0)
}

/// Where an IK goal points: a place in the world, or an entity (by name)
/// it follows.
#[derive(Debug, Clone, PartialEq)]
pub enum IkTarget {
    /// A point in the world.
    Point(Vec3),
    /// The entity with this name: its head, if it has an animated one,
    /// else its origin.
    Entity(String),
}

/// One bone reaching for a target.
#[derive(Debug, Clone, PartialEq)]
struct Reach {
    bone: String,
    target: Option<IkTarget>,
    /// How much it reaches when on (0 to 1).
    weight: f32,
    /// How much it reaches this frame (eases in and out).
    now: f32,
    /// Where it last reached (to ease out towards after it stops).
    last: Option<Vec3>,
}

/// The head turning to a target.
#[derive(Debug, Clone, PartialEq, Default)]
struct Look {
    target: Option<IkTarget>,
    weight: f32,
    now: f32,
    last: Option<Vec3>,
}

/// Seconds an IK goal takes to blend fully in or out.
const BLEND: f32 = 0.25;
/// How far the head turns at most (radians).
const MAX_LOOK: f32 = 1.2;
/// How quickly feet settle onto the ground (per second).
const FOOT_RATE: f32 = 14.0;

/// What foot placement needs from the world: the ground below a point,
/// `(from, max_distance)` → the hit point and its normal (world space).
pub type GroundProbe<'a> = &'a dyn Fn(Vec3, f32) -> Option<(Vec3, Vec3)>;

/// The world around an animator for one frame of IK.
pub struct IkWorld<'a> {
    /// The rig's space in the world: the entity's matrix times the
    /// model's import transform.
    pub model: Mat4,
    /// Seconds since the last frame.
    pub dt: f32,
    /// Where targets naming entities are this frame.
    pub find: &'a dyn Fn(&str) -> Option<Vec3>,
    /// The ground, for the feet (none: they stay as animated).
    pub ground: Option<GroundProbe<'a>>,
}

/// An animator's inverse kinematics: hands reaching, the head looking and
/// the feet following the ground, eased in and out as they turn on and off.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ik {
    reach: Vec<Reach>,
    look: Look,
    /// Whether the feet follow the ground.
    pub feet: bool,
    feet_now: f32,
    /// Each leg's smoothed lift (rig units) and the hips' drop.
    lifts: Vec<f32>,
    drop: f32,
}

fn ease(now: &mut f32, goal: f32, dt: f32) {
    let step = if dt > 0.0 { dt / BLEND } else { 1.0 };
    *now += (goal - *now).clamp(-step, step);
}

impl Ik {
    /// Sets (or with `None` stops) `bone` reaching for `target` by
    /// `weight`.
    pub fn set_reach(&mut self, bone: &str, target: Option<IkTarget>, weight: f32) {
        let weight = weight.clamp(0.0, 1.0);
        match self.reach.iter_mut().find(|r| r.bone == bone) {
            Some(r) => {
                r.target = target;
                if r.target.is_some() {
                    r.weight = weight;
                }
            }
            None if target.is_some() => self.reach.push(Reach {
                bone: bone.to_string(),
                target,
                weight,
                now: 0.0,
                last: None,
            }),
            None => {}
        }
    }

    /// Sets (or with `None` stops) the head looking at `target`.
    pub fn set_look(&mut self, target: Option<IkTarget>, weight: f32) {
        if target.is_some() {
            self.look.weight = weight.clamp(0.0, 1.0);
        }
        self.look.target = target;
    }

    /// The bones reaching for something (including ones easing out).
    #[must_use]
    pub fn reaching(&self) -> Vec<&str> {
        self.reach
            .iter()
            .filter(|r| r.target.is_some())
            .map(|r| r.bone.as_str())
            .collect()
    }

    /// Whether the head is set to look at something.
    #[must_use]
    pub fn looking(&self) -> bool {
        self.look.target.is_some()
    }

    /// Whether anything needs solving this frame.
    #[must_use]
    pub fn active(&self) -> bool {
        self.feet
            || self.feet_now > 0.0
            || self.look.target.is_some()
            || self.look.now > 0.0
            || self.reach.iter().any(|r| r.target.is_some() || r.now > 0.0)
    }

    /// Whether any goal follows an entity (so the caller looks names up).
    #[must_use]
    pub fn follows_entities(&self) -> bool {
        let named = |t: &Option<IkTarget>| matches!(t, Some(IkTarget::Entity(_)));
        named(&self.look.target) || self.reach.iter().any(|r| named(&r.target))
    }

    /// Adds the names of the entities goals follow to `out`.
    pub fn target_names(&self, out: &mut Vec<String>) {
        let targets =
            std::iter::once(&self.look.target).chain(self.reach.iter().map(|r| &r.target));
        for t in targets {
            if let Some(IkTarget::Entity(name)) = t {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
        }
    }

    /// Bends `pose` for this frame: feet onto the ground, hands to their
    /// targets, then the head.
    pub fn apply(&mut self, rig: &Rig, pose: &mut Pose, w: &IkWorld<'_>) {
        if !self.active() || pose.0.len() < rig.nodes.len() {
            return;
        }
        let det = w.model.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return;
        }
        let to_rig = w.model.inverse();
        let resolve = |t: &IkTarget| match t {
            IkTarget::Point(p) => Some(to_rig.transform_point3(*p)),
            IkTarget::Entity(name) => (w.find)(name).map(|p| to_rig.transform_point3(p)),
        };
        let mut globals = rig.globals(pose);
        self.place_feet(rig, pose, &mut globals, w);
        for r in &mut self.reach {
            let goal = r.target.as_ref().and_then(resolve);
            ease(
                &mut r.now,
                if goal.is_some() { r.weight } else { 0.0 },
                w.dt,
            );
            if goal.is_some() {
                r.last = goal;
            }
            let (Some(target), true) = (r.last, r.now > 0.0) else {
                continue;
            };
            let Some(limb) = rig.node(&r.bone).and_then(|e| Limb::ending_at(rig, e)) else {
                continue;
            };
            let from = position(&globals[limb.end]);
            let aim = from.lerp(target, r.now);
            solve_two_bone(rig, pose, &mut globals, limb, aim, None, None);
        }
        self.reach.retain(|r| r.target.is_some() || r.now > 0.0);
        let goal = self.look.target.as_ref().and_then(resolve);
        let weight = if goal.is_some() {
            self.look.weight
        } else {
            0.0
        };
        ease(&mut self.look.now, weight, w.dt);
        if goal.is_some() {
            self.look.last = goal;
        }
        if let (Some(target), true, Some((head, neck))) =
            (self.look.last, self.look.now > 0.0, find_head(rig))
        {
            // Entities face +Z; the import settings may have turned the
            // model to do so.
            let forward = rig.import.inverse().transform_vector3(Vec3::Z);
            let forward = forward.try_normalize().unwrap_or(Vec3::Z);
            let now = self.look.now;
            if let Some(neck) = neck {
                look_at(
                    rig,
                    pose,
                    &mut globals,
                    neck,
                    target,
                    (forward, now * 0.4, MAX_LOOK * 0.4),
                );
            }
            look_at(
                rig,
                pose,
                &mut globals,
                head,
                target,
                (forward, now, MAX_LOOK * 0.6),
            );
        }
        if self.look.target.is_none() && self.look.now <= 0.0 {
            self.look.last = None;
        }
    }

    /// Feet onto the ground: each foot moves by how far the ground under
    /// it is from the floor the model stands on, the hips drop for the
    /// lower one, and the feet tilt to the slope.
    #[allow(clippy::too_many_lines)]
    fn place_feet(&mut self, rig: &Rig, pose: &mut Pose, globals: &mut Vec<Mat4>, w: &IkWorld<'_>) {
        let legs = find_legs(rig);
        let goal = if self.feet && w.ground.is_some() && !legs.is_empty() {
            1.0
        } else {
            0.0
        };
        ease(&mut self.feet_now, goal, w.dt);
        if self.feet_now <= 0.0 {
            self.lifts.clear();
            self.drop = 0.0;
            return;
        }
        let Some(ground) = w.ground else { return };
        self.lifts.resize(legs.len(), 0.0);
        // The floor the model stands on: its origin, or its lowest foot
        // in the rest pose if that's lower (a rig rooted at the hips).
        let rest = rig.globals(&rig.rest_pose());
        let floor = legs
            .iter()
            .map(|l| position(&rest[l.end]).y)
            .fold(0.0f32, f32::min);
        let up_world = w.model.transform_vector3(Vec3::Y);
        let units = up_world.length(); // world metres per rig unit, upwards
        if units < 1e-6 {
            return;
        }
        let up_world = up_world / units;
        let to_rig = w.model.inverse();
        let floor_world = w.model.transform_point3(Vec3::new(0.0, floor, 0.0));
        // How far a foot may move: half a leg.
        let reach = |l: &Limb| {
            f32::midpoint(
                position(&rest[l.upper]).distance(position(&rest[l.mid])),
                position(&rest[l.mid]).distance(position(&rest[l.end])),
            )
        };
        let k = 1.0 - (-FOOT_RATE * w.dt.max(0.0)).exp();
        let k = if w.dt > 0.0 { k } else { 1.0 };
        let mut wanted = Vec::with_capacity(legs.len());
        for (i, l) in legs.iter().enumerate() {
            let max = reach(l);
            let foot = w.model.transform_point3(position(&globals[l.end]));
            // The ground under the foot, level with the model's floor.
            let probe = foot - up_world * up_world.dot(foot - floor_world);
            let span = max * units;
            let hit = ground(probe + up_world * span, span * 2.0);
            let (lift, normal) = match hit {
                Some((p, n)) => {
                    let lift = up_world.dot(p - floor_world) / units;
                    if lift.abs() <= max {
                        (lift, to_rig.transform_vector3(n).try_normalize())
                    } else {
                        (0.0, None) // too far: in the air, or a wall
                    }
                }
                None => (0.0, None),
            };
            self.lifts[i] += (lift - self.lifts[i]) * k;
            wanted.push(normal);
        }
        let lowest = self.lifts.iter().copied().fold(0.0f32, f32::min);
        self.drop += (lowest - self.drop) * k;
        let weight = self.feet_now;
        // Drop the hips (the thighs' parent) for the lower foot.
        if let Some(hips) = rig.nodes[legs[0].upper].parent {
            let up_rig = Vec3::Y;
            let down = up_rig * self.drop * weight;
            let parent = rig.nodes[hips]
                .parent
                .map_or(Mat4::IDENTITY, |p| globals[p]);
            let moved = position(&globals[hips]) + down;
            pose.0[hips].translation = parent.inverse().transform_point3(moved);
            *globals = rig.globals(pose);
        }
        for (i, l) in legs.iter().enumerate() {
            let foot = position(&globals[l.end]) - Vec3::Y * self.drop * weight;
            // A foot that isn't the shin's child didn't drop with the hips.
            let foot = if l.end_follows(rig) {
                foot
            } else {
                foot + Vec3::Y * self.drop * weight
            };
            let target = foot + Vec3::Y * self.lifts[i] * weight;
            let tilt = wanted[i].map(|n| {
                let full = Quat::from_rotation_arc(Vec3::Y, n);
                let (axis, angle) = full.to_axis_angle();
                Quat::from_axis_angle(axis, angle.min(0.6) * weight)
            });
            solve_two_bone(rig, pose, globals, *l, target, None, tilt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_assets::anim::{RigNode, Trs};

    fn node(name: &str, parent: Option<usize>, at: [f32; 3]) -> RigNode {
        RigNode {
            name: name.into(),
            parent,
            rest: Trs {
                translation: Vec3::from(at),
                ..Trs::IDENTITY
            },
        }
    }

    /// Hips at 1 m; each leg a 0.5 m thigh and a 0.5 m shin, bent a
    /// little forward at the knee, with the foot under the shin.
    fn legs() -> Rig {
        Rig {
            nodes: vec![
                node("Hips", None, [0.0, 1.0, 0.0]),
                node("UpperLeg.L", Some(0), [0.1, 0.0, 0.0]),
                node("LowerLeg.L", Some(1), [0.0, -0.5, 0.02]),
                node("Foot.L", Some(2), [0.0, -0.5, -0.02]),
                node("UpperLeg.R", Some(0), [-0.1, 0.0, 0.0]),
                node("LowerLeg.R", Some(4), [0.0, -0.5, 0.02]),
                node("Foot.R", Some(5), [0.0, -0.5, -0.02]),
                node("Spine", Some(0), [0.0, 0.3, 0.0]),
                node("Neck", Some(7), [0.0, 0.3, 0.0]),
                node("Head", Some(8), [0.0, 0.1, 0.0]),
            ],
            joints: Vec::new(),
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: automata_assets::springs::Springs::default(),
        }
    }

    /// The robot's layout: feet hanging from the root, not the shins.
    fn ik_rig() -> Rig {
        Rig {
            nodes: vec![
                node("Bone", None, [0.0, 0.0, 0.0]),
                node("Foot.L", Some(0), [0.1, 0.0, 0.0]),
                node("Body", Some(0), [0.0, 1.0, 0.0]),
                node("UpperLeg.L", Some(2), [0.1, 0.0, 0.0]),
                node("LowerLeg.L", Some(3), [0.0, -0.5, 0.02]),
            ],
            joints: Vec::new(),
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: automata_assets::springs::Springs::default(),
        }
    }

    #[test]
    fn names_tell_sides_and_parts() {
        assert_eq!(side("UpperLeg.L"), Some(true));
        assert_eq!(side("mixamorig:RightFoot"), Some(false));
        assert_eq!(side("thigh_r"), Some(false));
        assert_eq!(side("Head"), None);
        assert_eq!(base("UpperLeg.L"), "upperleg");
        assert_eq!(base("mixamorig:LeftUpLeg"), "upleg");
        assert_eq!(base("calf_l"), "calf");
        assert_eq!((side("ShinL"), base("ShinL")), (Some(true), "shin".into()));
        assert_eq!(
            (side("ForearmR"), base("ForearmR")),
            (Some(false), "forearm".into())
        );
        assert_eq!((side("HEAD"), base("Spinal")), (None, "spinal".into()));
        let rig = legs();
        assert_eq!(
            find_legs(&rig),
            vec![
                Limb {
                    upper: 1,
                    mid: 2,
                    end: 3
                },
                Limb {
                    upper: 4,
                    mid: 5,
                    end: 6
                }
            ]
        );
        assert_eq!(find_head(&rig), Some((9, Some(8))));
        assert_eq!(
            find_legs(&ik_rig()),
            vec![Limb {
                upper: 3,
                mid: 4,
                end: 1
            }]
        );
    }

    #[test]
    fn a_limb_reaches_its_target_and_keeps_its_bend() {
        let rig = legs();
        let limb = Limb::ending_at(&rig, 3).unwrap();
        for target in [
            Vec3::new(0.1, 0.3, 0.2),
            Vec3::new(0.3, 0.4, 0.1),
            Vec3::new(0.0, 0.5, 0.4),
        ] {
            let mut pose = rig.rest_pose();
            let mut globals = rig.globals(&pose);
            assert!(solve_two_bone(
                &rig,
                &mut pose,
                &mut globals,
                limb,
                target,
                None,
                None
            ));
            let foot = position(&globals[3]);
            assert!(foot.distance(target) < 1e-3, "{foot} for {target}");
            // The knee still bends forward.
            let knee = position(&globals[2]);
            let (hip, line) = (
                position(&globals[1]),
                (foot - position(&globals[1])).normalize(),
            );
            let off = (knee - hip) - line * line.dot(knee - hip);
            assert!(off.z > 0.0, "the knee went backwards: {off}");
            // Bone lengths hold.
            assert!((knee.distance(hip) - 0.5).abs() < 1e-3);
        }
        // Out of reach: as far as it goes, straight.
        let mut pose = rig.rest_pose();
        let mut globals = rig.globals(&pose);
        solve_two_bone(
            &rig,
            &mut pose,
            &mut globals,
            limb,
            Vec3::new(0.1, -5.0, 0.0),
            None,
            None,
        );
        let foot = position(&globals[3]);
        assert!(
            (foot.distance(position(&globals[1])) - 1.0).abs() < 0.01,
            "{foot}"
        );
    }

    #[test]
    fn a_foot_hanging_elsewhere_moves_to_the_shin() {
        let rig = ik_rig();
        let limb = find_legs(&rig)[0];
        let mut pose = rig.rest_pose();
        let mut globals = rig.globals(&pose);
        // The shin's tip is the foot (it carries the foot as a point).
        let target = Vec3::new(0.1, 0.3, 0.1);
        assert!(solve_two_bone(
            &rig,
            &mut pose,
            &mut globals,
            limb,
            target,
            None,
            None
        ));
        assert!(
            position(&globals[1]).distance(target) < 1e-3,
            "{}",
            position(&globals[1])
        );
        assert_eq!(
            pose.0[1].rotation,
            Quat::IDENTITY,
            "the foot keeps facing as it did"
        );
    }

    #[test]
    fn helper_bones_named_like_legs_are_not_taken_for_the_shin() {
        // The robot's export: a short "Leg.L" helper hangs off the thigh
        // beside the real shin, whose last bone ends where the foot is.
        let rig = Rig {
            nodes: vec![
                node("Bone", None, [0.0, 0.0, 0.0]),
                node("Foot.L", Some(0), [0.1, 0.0, 0.0]),
                node("Body", Some(0), [0.0, 1.0, 0.0]),
                node("UpperLeg.L", Some(2), [0.1, 0.0, 0.0]),
                node("Leg.L", Some(3), [0.0, -0.15, 0.05]),
                node("LowerLeg.L", Some(3), [0.0, -0.5, 0.05]),
                node("LowerLeg.L", Some(5), [0.0, -0.25, -0.02]),
                node("LowerLeg.L_end", Some(6), [0.0, -0.25, -0.03]),
            ],
            joints: Vec::new(),
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: automata_assets::springs::Springs::default(),
        };
        let limb = find_legs(&rig)[0];
        assert_eq!((limb.upper, limb.mid, limb.end), (3, 5, 1));
        let mut pose = rig.rest_pose();
        let mut globals = rig.globals(&pose);
        let shin = position(&globals[5]).distance(position(&globals[1]));
        // Up a 0.15 m step.
        let target = Vec3::new(0.1, 0.15, 0.0);
        assert!(solve_two_bone(
            &rig,
            &mut pose,
            &mut globals,
            limb,
            target,
            None,
            None
        ));
        let (hip, knee) = (position(&globals[3]), position(&globals[5]));
        assert!(position(&globals[1]).distance(target) < 1e-3);
        assert!(
            (knee.distance(target) - shin).abs() < 1e-3,
            "the shin keeps its length"
        );
        assert!(
            knee.y < hip.y && knee.z > 0.05,
            "the knee bends forward, below the hip: {knee}"
        );
    }

    #[test]
    fn heads_turn_to_look_within_limits() {
        let rig = legs();
        let mut pose = rig.rest_pose();
        let mut globals = rig.globals(&pose);
        let head = position(&globals[9]);
        // Off to the left (+X) and a little up.
        let target = head + Vec3::new(2.0, 0.5, 2.0);
        let short = look_at(
            &rig,
            &mut pose,
            &mut globals,
            9,
            target,
            (Vec3::Z, 1.0, 1.5),
        );
        assert!(short < 1e-4);
        let facing = rotation(&globals[9]) * Vec3::Z;
        assert!(facing.angle_between(target - head) < 1e-3, "{facing}");
        // Behind it: turns as far as allowed, no further.
        let mut pose = rig.rest_pose();
        let mut globals = rig.globals(&pose);
        let short = look_at(
            &rig,
            &mut pose,
            &mut globals,
            9,
            head - Vec3::Z,
            (Vec3::Z, 1.0, 1.0),
        );
        let facing = rotation(&globals[9]) * Vec3::Z;
        assert!((facing.angle_between(Vec3::Z) - 1.0).abs() < 1e-3);
        assert!((short - (std::f32::consts::PI - 1.0)).abs() < 1e-3);
    }

    fn feet_y(rig: &Rig, pose: &Pose) -> (f32, f32) {
        let g = rig.globals(pose);
        (position(&g[3]).y, position(&g[6]).y)
    }

    #[test]
    fn feet_stand_on_a_step_and_the_hips_drop() {
        let rig = legs();
        // A step 0.2 m up under the left foot (x > 0); the floor at 0.
        let ground = |from: Vec3, max: f32| {
            let y = if from.x > 0.0 { 0.2 } else { 0.0 };
            (from.y - y <= max && from.y >= y).then_some((Vec3::new(from.x, y, from.z), Vec3::Y))
        };
        let none = |_: &str| None;
        let mut ik = Ik {
            feet: true,
            ..Ik::default()
        };
        let mut pose = rig.rest_pose();
        let rest = feet_y(&rig, &pose);
        for _ in 0..60 {
            pose = rig.rest_pose();
            let world = IkWorld {
                model: Mat4::IDENTITY,
                dt: 1.0 / 30.0,
                find: &none,
                ground: Some(&ground),
            };
            ik.apply(&rig, &mut pose, &world);
        }
        let (l, r) = feet_y(&rig, &pose);
        assert!(
            (l - (rest.0 + 0.2)).abs() < 0.01,
            "left foot on the step: {l}"
        );
        assert!((r - rest.1).abs() < 0.01, "right foot on the floor: {r}");
        // On the step side the floor is higher, so nothing drops: both
        // lifts are up or level.
        assert!(ik.drop.abs() < 1e-3);
        // A hole under the right foot instead: the hips come down.
        let hole = |from: Vec3, max: f32| {
            let y = if from.x < 0.0 { -0.15 } else { 0.0 };
            (from.y - y <= max && from.y >= y).then_some((Vec3::new(from.x, y, from.z), Vec3::Y))
        };
        let g0 = rig.globals(&rig.rest_pose());
        for _ in 0..60 {
            pose = rig.rest_pose();
            let world = IkWorld {
                model: Mat4::IDENTITY,
                dt: 1.0 / 30.0,
                find: &none,
                ground: Some(&hole),
            };
            ik.apply(&rig, &mut pose, &world);
        }
        let g = rig.globals(&pose);
        assert!(
            (position(&g[0]).y - (position(&g0[0]).y - 0.15)).abs() < 0.01,
            "hips dropped"
        );
        let (l, r) = feet_y(&rig, &pose);
        assert!(
            (l - rest.0).abs() < 0.01 && (r - (rest.1 - 0.15)).abs() < 0.01,
            "{l} {r}"
        );
        // Turned off, it eases back to the animation.
        ik.feet = false;
        for _ in 0..30 {
            pose = rig.rest_pose();
            let world = IkWorld {
                model: Mat4::IDENTITY,
                dt: 1.0 / 30.0,
                find: &none,
                ground: Some(&hole),
            };
            ik.apply(&rig, &mut pose, &world);
        }
        assert_eq!(feet_y(&rig, &pose), rest);
        assert!(!ik.active());
    }

    #[test]
    fn reaching_and_looking_follow_entities_and_ease_out() {
        let rig = legs();
        let mut ik = Ik::default();
        let spot = Vec3::new(0.35, 0.45, 0.3);
        ik.set_reach("Foot.L", Some(IkTarget::Entity("Ball".into())), 1.0);
        ik.set_look(Some(IkTarget::Point(Vec3::new(5.0, 2.0, 5.0))), 1.0);
        assert!(ik.follows_entities() && ik.looking());
        assert_eq!(ik.reaching(), vec!["Foot.L"]);
        let find = |name: &str| (name == "Ball").then_some(spot);
        let mut pose = rig.rest_pose();
        for _ in 0..20 {
            pose = rig.rest_pose();
            let world = IkWorld {
                model: Mat4::IDENTITY,
                dt: 0.05,
                find: &find,
                ground: None,
            };
            ik.apply(&rig, &mut pose, &world);
        }
        let g = rig.globals(&pose);
        assert!(position(&g[3]).distance(spot) < 1e-3);
        assert!(rotation(&g[9]) != Quat::IDENTITY, "the head turned");
        // Stopping eases out over BLEND, then nothing's left.
        ik.set_reach("Foot.L", None, 0.0);
        ik.set_look(None, 0.0);
        for _ in 0..10 {
            pose = rig.rest_pose();
            let world = IkWorld {
                model: Mat4::IDENTITY,
                dt: 0.05,
                find: &find,
                ground: None,
            };
            ik.apply(&rig, &mut pose, &world);
        }
        assert!(!ik.active());
        assert_eq!(pose, rig.rest_pose());
    }
}
