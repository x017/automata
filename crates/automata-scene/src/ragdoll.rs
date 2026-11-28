//! Ragdolls: an animated model going limp, its skeleton thrown about by
//! physics, then handing back to the animation.
//!
//! - [`Ragdoll`] is the component (saved in scene files): whether the
//!   model is limp right now, its mass, how stiff its joints are and how
//!   long the animation takes to take over again.
//! - [`RagdollLayout`] is the physical skeleton worked out from a rigged
//!   model: a capsule per big bone, sized to the skin that bone moves,
//!   and a joint to its parent (knees and elbows bend one way only).
//!   The physics crate builds bodies from it.
//! - [`RagdollPose`] is what physics reports each step (where every
//!   simulated bone is); [`crate::animate`] blends it over the animation,
//!   and when the ragdoll is switched off, blends back from the last pose
//!   over [`Ragdoll::blend_time`].
//!
//! Bones are matched generically: any bone that moves enough of the skin
//! (a tenth of the model's height across) gets a body; smaller ones
//! (fingers, toes, hair, the neck) ride along with the nearest body above
//! them, as their animation has them. Mesh nodes parented straight onto a
//! bone (rigid robots) count as part of that bone.

use automata_assets::anim::Trs;
use automata_assets::{Model, Pose, Rig};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::ik::base;

/// The default mass of a ragdoll, in kilograms.
pub const DEFAULT_RAGDOLL_MASS: f32 = 70.0;

/// How much of the model's height a bone's skin must span to get a body.
const BIG_BONE: f32 = 0.1;

/// At most this many bodies per ragdoll (the biggest bones win).
const MAX_BODIES: usize = 24;

/// An animated model that can go limp (also its ECS component and its
/// record in scene files). It needs a rigged model (an [`crate::Animation`]
/// or spring bones give the entity its [`crate::Animator`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ragdoll {
    /// Limp right now: physics moves the skeleton instead of the
    /// animation. Switching it off blends back to the animation.
    pub active: bool,
    /// The whole body's mass in kilograms, shared out over the bones by
    /// size.
    pub mass: f32,
    /// How hard the joints hold the pose the model had when it went limp:
    /// 0 is a rag doll, 1 holds it about as firmly as it can (a stunned,
    /// stiff fall).
    pub stiffness: f32,
    /// Seconds the animation takes to take over again once switched off.
    pub blend_time: f32,
}

impl Default for Ragdoll {
    fn default() -> Self {
        Self {
            active: false,
            mass: DEFAULT_RAGDOLL_MASS,
            stiffness: 0.0,
            blend_time: 0.5,
        }
    }
}

impl Ragdoll {
    /// The same ragdoll with every value in range (mass 1 to 10 000 kg,
    /// stiffness 0 to 1, blend time 0 to 10 s; nonsense becomes the
    /// default).
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let fix = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        Self {
            active: self.active,
            mass: fix(self.mass, 1.0, 10_000.0, DEFAULT_RAGDOLL_MASS),
            stiffness: fix(self.stiffness, 0.0, 1.0, 0.0),
            blend_time: fix(self.blend_time, 0.0, 10.0, 0.5),
        }
    }
}

/// Where physics last put a simulated bone (world space; the bone's own
/// frame without its scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RagdollBone {
    /// The rig node.
    pub node: usize,
    /// The bone's origin.
    pub position: Vec3,
    /// The bone's rotation.
    pub rotation: Quat,
}

/// A push waiting for the next physics step.
#[derive(Debug, Clone, PartialEq)]
pub struct RagdollPush {
    /// The bone pushed (`None`: the whole body, shared out by mass).
    pub bone: Option<String>,
    /// The impulse in newton-seconds (kg·m/s), world space.
    pub impulse: Vec3,
}

/// A ragdoll's state while it runs (a runtime component, never saved):
/// the bones physics moved, written after every step, and the blend over
/// the animation that [`crate::animate`] keeps.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RagdollPose {
    /// The simulated bones as of the last physics step.
    pub bones: Vec<RagdollBone>,
    /// Physics is moving them right now.
    pub simulating: bool,
    /// How much of the pose is the ragdoll's: 1 while limp, falling to 0
    /// over the blend time once switched off.
    pub weight: f32,
    /// Seconds the blend back takes.
    pub blend_time: f32,
    /// Pushes for the next physics step.
    pub pushes: Vec<RagdollPush>,
    /// Limp and come to rest (every body all but still).
    pub resting: bool,
    /// The bones' local transforms when the ragdoll was switched off,
    /// which the animation blends back from.
    frozen: Vec<(usize, Trs)>,
}

impl RagdollPose {
    /// Whether the ragdoll shows at all (limp, or blending back).
    #[must_use]
    pub fn showing(&self) -> bool {
        self.simulating || (self.weight > 0.0 && !self.bones.is_empty())
    }

    /// Lays the ragdoll over `pose` (an animator's local pose for `rig`,
    /// on an entity placed at `place`), and moves the blend back on by
    /// `dt` seconds. Returns whether it changed the pose.
    pub fn blend_into(&mut self, rig: &Rig, place: Mat4, pose: &mut Pose, dt: f32) -> bool {
        if self.simulating {
            self.weight = 1.0;
            self.frozen.clear();
            let locals = ragdoll_locals(rig, place, pose, &self.bones);
            for (node, trs) in locals {
                if let Some(slot) = pose.0.get_mut(node) {
                    slot.translation = trs.translation;
                    slot.rotation = trs.rotation;
                }
            }
            return true;
        }
        if self.weight <= 0.0 || self.bones.is_empty() {
            self.weight = 0.0;
            return false;
        }
        if self.frozen.is_empty() {
            self.frozen = ragdoll_locals(rig, place, pose, &self.bones);
        }
        // Ease out of the ragdoll: smooth at both ends.
        let w = self.weight * self.weight * (3.0 - 2.0 * self.weight);
        for (node, trs) in &self.frozen {
            if let Some(slot) = pose.0.get_mut(*node) {
                slot.translation = slot.translation.lerp(trs.translation, w);
                slot.rotation = slot.rotation.slerp(trs.rotation, w).normalize();
            }
        }
        self.weight = if self.blend_time > 0.0 {
            (self.weight - dt / self.blend_time).max(0.0)
        } else {
            0.0
        };
        if self.weight <= 0.0 {
            self.bones.clear();
            self.frozen.clear();
        }
        true
    }
}

/// The local transforms (translation and rotation; scale as `pose` has
/// it) that put each simulated bone where physics has it, for an entity
/// placed at `place`. Bones without a body keep `pose`'s.
fn ragdoll_locals(rig: &Rig, place: Mat4, pose: &Pose, bones: &[RagdollBone]) -> Vec<(usize, Trs)> {
    let n = rig.nodes.len();
    let mut body_of: Vec<Option<RagdollBone>> = vec![None; n];
    for bone in bones {
        if let Some(slot) = body_of.get_mut(bone.node) {
            *slot = Some(*bone);
        }
    }
    let model = place * rig.import;
    let to_rig = model.inverse();
    let mut globals: Vec<Mat4> = Vec::with_capacity(n);
    let mut out = Vec::new();
    for (i, node) in rig.nodes.iter().enumerate() {
        let parent = node
            .parent
            .filter(|&p| p < globals.len())
            .map_or(Mat4::IDENTITY, |p| globals[p]);
        let local = pose.0.get(i).copied().unwrap_or(node.rest);
        let Some(bone) = body_of[i] else {
            globals.push(parent * local.matrix());
            continue;
        };
        // The bone's scale in the world, as the animation has it.
        let (world_scale, _, _) = (model * parent * local.matrix()).to_scale_rotation_translation();
        let world =
            Mat4::from_scale_rotation_translation(world_scale, bone.rotation, bone.position);
        let wanted = parent.inverse() * to_rig * world;
        let (_, rotation, translation) = wanted.to_scale_rotation_translation();
        let trs = Trs {
            translation,
            rotation: rotation.normalize(),
            scale: local.scale,
        };
        globals.push(parent * trs.matrix());
        out.push((i, trs));
    }
    out
}

/// How a ragdoll body turns against its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RagdollJointKind {
    /// Turns about the joint frame's X axis only, between `min` and `max`
    /// radians (knees, elbows).
    Hinge {
        /// The furthest it bends one way.
        min: f32,
        /// The furthest it bends the other.
        max: f32,
    },
    /// Turns any way: `swing` radians off the frame's X axis (the bone's
    /// direction), `twist` radians about it.
    Ball {
        /// The furthest it swings.
        swing: f32,
        /// The furthest it twists.
        twist: f32,
    },
}

/// The joint holding a ragdoll body to its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RagdollJoint {
    /// Where the joint sits, in the parent body's frame.
    pub anchor_parent: Vec3,
    /// Where the joint sits, in this body's frame.
    pub anchor_child: Vec3,
    /// The joint's axes in the parent body's frame (X: the hinge or the
    /// twist axis). They match `frame_child` in the rest pose.
    pub frame_parent: Quat,
    /// The joint's axes in this body's frame.
    pub frame_child: Quat,
    /// How it turns.
    pub kind: RagdollJointKind,
}

/// One body of a ragdoll: a capsule following a bone.
///
/// Its frame is the bone's (in the model: import settings applied, scale
/// left out), so physics places it where the bone is and the bone goes
/// where physics puts it.
#[derive(Debug, Clone, PartialEq)]
pub struct RagdollBody {
    /// The rig node it moves.
    pub node: usize,
    /// The bone's name.
    pub name: String,
    /// The body it hangs from (an index into [`RagdollLayout::bodies`]);
    /// `None` for the root (the hips).
    pub parent: Option<usize>,
    /// One end of the capsule's core, in the body's frame.
    pub a: Vec3,
    /// The other end.
    pub b: Vec3,
    /// The capsule's radius.
    pub radius: f32,
    /// Its share of the ragdoll's mass (by volume; all shares add to 1).
    pub share: f32,
    /// The joint to `parent`.
    pub joint: Option<RagdollJoint>,
}

impl RagdollBody {
    /// The capsule's volume (model units cubed).
    #[must_use]
    pub fn volume(&self) -> f32 {
        let r = self.radius;
        std::f32::consts::PI * r * r * (self.a.distance(self.b) + 4.0 / 3.0 * r)
    }
}

/// A rigged model's physical skeleton: what [`RagdollBody`]s it has and
/// how they join. Lengths are in model units (the import settings
/// applied); the entity's scale multiplies them.
#[derive(Debug, Clone, PartialEq)]
pub struct RagdollLayout {
    /// The bodies, parents before children; `bodies[0]` is the root.
    pub bodies: Vec<RagdollBody>,
    /// The model's height (model units).
    pub height: f32,
}

impl RagdollLayout {
    /// The body moving `node`, if it has one.
    #[must_use]
    pub fn body_for(&self, node: usize) -> Option<usize> {
        self.bodies.iter().position(|b| b.node == node)
    }

    /// The body whose bone is called `name` (or, failing that, the one
    /// whose bone name matches it in any case), if any.
    #[must_use]
    pub fn body_named(&self, name: &str) -> Option<usize> {
        self.bodies.iter().position(|b| b.name == name).or_else(|| {
            self.bodies
                .iter()
                .position(|b| b.name.eq_ignore_ascii_case(name))
        })
    }

    /// Works out the physical skeleton of `model`, or `None` when it has
    /// no rig or no skin to size bodies from.
    #[must_use]
    pub fn of(model: &Model) -> Option<Self> {
        let rig = model.rig.as_ref()?;
        let n = rig.nodes.len();
        let rest: Vec<Mat4> = rig
            .globals(&rig.rest_pose())
            .into_iter()
            .map(|g| rig.import * g)
            .collect();
        let (mut points, rigid) = skin_points(model, rig);
        let (lo, hi) = points.iter().flatten().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        let height = (hi - lo).max_element();
        if !height.is_finite() || height <= 0.0 {
            return None;
        }
        // Meshes hung rigidly onto a bare bone (a leaf node whose every
        // vertex follows it alone, under a bone moving no skin itself)
        // are that bone.
        let bare: Vec<bool> = points.iter().map(Vec::is_empty).collect();
        for i in (0..n).rev() {
            let Some(p) = rig.nodes[i].parent else {
                continue;
            };
            let leaf = !rig.nodes.iter().any(|c| c.parent == Some(i));
            if leaf && rigid[i] && bare[p] && !points[i].is_empty() {
                let moved = std::mem::take(&mut points[i]);
                points[p].extend(moved);
            }
        }
        // Spring bones (ponytails, skirts) keep swinging on their own.
        let springy = spring_nodes(rig);
        // Big bones get bodies; the rest ride with the nearest one above.
        let mut big: Vec<bool> = points
            .iter()
            .zip(&springy)
            .map(|(pts, &spring)| !spring && pts.len() >= 4 && spread(pts) >= BIG_BONE * height)
            .collect();
        limit_bodies(&mut big, &points);
        for i in (0..n).rev() {
            if big[i] || points[i].is_empty() {
                continue;
            }
            let mut at = rig.nodes[i].parent;
            while let Some(p) = at {
                if big[p] {
                    let moved = std::mem::take(&mut points[i]);
                    points[p].extend(moved);
                    break;
                }
                at = rig.nodes[p].parent;
            }
        }
        let chosen: Vec<usize> = (0..n).filter(|&i| big[i]).collect();
        if chosen.is_empty() {
            return None;
        }
        let shapes: Vec<Capsule> = chosen
            .iter()
            .map(|&i| Capsule::fit(&points[i], &rest[i], height))
            .collect();
        let parents = body_parents(rig, &chosen, &shapes, &rest);
        Some(assemble(rig, &chosen, &shapes, &parents, &rest, height))
    }
}

/// The rest-pose skin's points (model space) by the node moving each
/// most, and whether every one of a node's points follows it alone (a
/// mesh hung rigidly on it).
fn skin_points(model: &Model, rig: &Rig) -> (Vec<Vec<Vec3>>, Vec<bool>) {
    let mut points = vec![Vec::new(); rig.nodes.len()];
    let mut rigid = vec![true; rig.nodes.len()];
    let palette = rig.rest_palette();
    for part in &model.parts {
        let Some(skin) = &part.skin else { continue };
        for (v, sv) in skin.posed(&palette).iter().zip(&skin.skin) {
            let best = sv
                .joints
                .iter()
                .zip(&sv.weights)
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(j, w)| (usize::from(*j), *w));
            let Some((joint, weight)) = best else {
                continue;
            };
            let Some(&(node, _)) = rig.joints.get(joint) else {
                continue;
            };
            if let Some(slot) = points.get_mut(node) {
                slot.push(Vec3::from(v.position));
                rigid[node] &= weight > 0.999;
            }
        }
    }
    (points, rigid)
}

/// Which nodes swing as spring bones (a chain's root and all below it).
fn spring_nodes(rig: &Rig) -> Vec<bool> {
    let mut springy = vec![false; rig.nodes.len()];
    for chain in &rig.springs.chains {
        if let Some(root) = rig.node(&chain.root) {
            springy[root] = true;
        }
    }
    // Parents come before children: marks run down the chains.
    for i in 0..rig.nodes.len() {
        if let Some(p) = rig.nodes[i].parent {
            if p < i && springy[p] {
                springy[i] = true;
            }
        }
    }
    springy
}

/// Keeps at most [`MAX_BODIES`] big bones: those with the most skin.
fn limit_bodies(big: &mut [bool], points: &[Vec<Vec3>]) {
    let mut ranked: Vec<usize> = (0..big.len()).filter(|&i| big[i]).collect();
    if ranked.len() <= MAX_BODIES {
        return;
    }
    ranked.sort_by_key(|&i| std::cmp::Reverse(points[i].len()));
    for &i in &ranked[MAX_BODIES..] {
        big[i] = false;
    }
}

fn origin(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

fn frame_rotation(m: &Mat4) -> Quat {
    let (_, r, _) = m.to_scale_rotation_translation();
    r.normalize()
}

/// How far apart a cloud of points is, along its longest box side.
fn spread(points: &[Vec3]) -> f32 {
    let (lo, hi) = points.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    (hi - lo).max_element()
}

/// A capsule in the model (rest pose).
#[derive(Debug, Clone, Copy)]
struct Capsule {
    a: Vec3,
    b: Vec3,
    radius: f32,
}

impl Capsule {
    /// The capsule wrapping most of `points` (model space), along their
    /// longest direction. `bone` is the bone's rest matrix.
    fn fit(points: &[Vec3], bone: &Mat4, height: f32) -> Self {
        #[allow(clippy::cast_precision_loss)] // a model's vertex count
        let count = points.len() as f32;
        let mean = points.iter().copied().sum::<Vec3>() / count;
        // The longest direction: power iteration on the covariance.
        let mut cov = [Vec3::ZERO; 3];
        for p in points {
            let d = *p - mean;
            cov[0] += d * d.x;
            cov[1] += d * d.y;
            cov[2] += d * d.z;
        }
        let apply = |v: Vec3| cov[0] * v.x + cov[1] * v.y + cov[2] * v.z;
        let start = (mean - origin(bone)).normalize_or(Vec3::Y);
        let mut axis = start;
        for _ in 0..24 {
            axis = apply(axis).normalize_or(axis);
        }
        let (mut t_lo, mut t_hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut across: Vec<f32> = Vec::with_capacity(points.len());
        for p in points {
            let d = *p - mean;
            let t = d.dot(axis);
            t_lo = t_lo.min(t);
            t_hi = t_hi.max(t);
            across.push((d - axis * t).length());
        }
        across.sort_by(f32::total_cmp);
        let pick = across.len() * 7 / 10;
        let radius =
            (across.get(pick).copied().unwrap_or(0.0) * 0.9).clamp(0.02 * height, 0.2 * height);
        let half = ((t_hi - t_lo) * 0.5 - radius).max(0.0);
        let centre = mean + axis * (t_lo + t_hi) * 0.5;
        Self {
            a: centre - axis * half,
            b: centre + axis * half,
            radius,
        }
    }

    fn centre(&self) -> Vec3 {
        (self.a + self.b) * 0.5
    }

    /// How far `p` is from the capsule's core.
    fn distance(&self, p: Vec3) -> f32 {
        let ab = self.b - self.a;
        let t = if ab.length_squared() > 0.0 {
            ((p - self.a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(self.a + ab * t)
    }
}

/// Each chosen bone's parent body (an index into `chosen`): the nearest
/// chosen bone above it, else (bones hung off the root in rigs exported
/// with IK controls, like feet) the body nearest its origin. The root is
/// the bone with the most bodies below it.
fn body_parents(
    rig: &Rig,
    chosen: &[usize],
    shapes: &[Capsule],
    rest: &[Mat4],
) -> Vec<Option<usize>> {
    let index = |node: usize| chosen.iter().position(|&c| c == node);
    let above = |node: usize| {
        let mut at = rig.nodes[node].parent;
        while let Some(p) = at {
            if let Some(i) = index(p) {
                return Some(i);
            }
            at = rig.nodes[p].parent;
        }
        None
    };
    let mut parents: Vec<Option<usize>> = chosen.iter().map(|&c| above(c)).collect();
    let below = |i: usize, parents: &[Option<usize>]| {
        (0..chosen.len())
            .filter(|&j| {
                let mut at = parents[j];
                let mut steps = 0;
                while let Some(p) = at {
                    if p == i {
                        return true;
                    }
                    steps += 1;
                    if steps > chosen.len() {
                        return false;
                    }
                    at = parents[p];
                }
                false
            })
            .count()
    };
    let root = (0..chosen.len())
        .filter(|&i| parents[i].is_none())
        .max_by_key(|&i| (below(i, &parents), std::cmp::Reverse(i)))
        .unwrap_or(0);
    for i in 0..chosen.len() {
        if i == root || parents[i].is_some() {
            continue;
        }
        let joint = origin(&rest[chosen[i]]);
        let mine: Vec<usize> = (0..chosen.len())
            .filter(|&j| j == i || descends(&parents, j, i))
            .collect();
        parents[i] = (0..chosen.len())
            .filter(|j| !mine.contains(j))
            .filter(|&j| parents[j].is_some() || j == root)
            .min_by(|&a, &b| {
                shapes[a]
                    .distance(joint)
                    .total_cmp(&shapes[b].distance(joint))
            });
    }
    // Anything still loose (a chain of orphans) hangs from the root.
    for (i, parent) in parents.iter_mut().enumerate() {
        if i != root && parent.is_none() {
            *parent = Some(root);
        }
    }
    parents[root] = None;
    parents
}

/// Whether body `j` hangs (somewhere) below body `i`.
fn descends(parents: &[Option<usize>], j: usize, i: usize) -> bool {
    let mut at = parents[j];
    for _ in 0..parents.len() {
        match at {
            Some(p) if p == i => return true,
            Some(p) => at = parents[p],
            None => return false,
        }
    }
    false
}

const SHINS: [&str; 6] = ["lowerleg", "leg", "calf", "shin", "knee", "lowleg"];
const FOREARMS: [&str; 5] = ["forearm", "lowerarm", "lowarm", "elbow", "arm2"];

/// How far a bone may swing and twist, from its name.
fn ball_limits(name: &str) -> (f32, f32) {
    let b = base(name);
    let has = |words: &[&str]| words.iter().any(|w| b.contains(w));
    // Twists are a little looser than a person's: a limb pinned against
    // the floor at its twist limit twitches rather than resting.
    if has(&["head", "neck"]) {
        (0.7, 1.0)
    } else if has(&[
        "spine", "chest", "abdomen", "torso", "belly", "pelvis", "body",
    ]) {
        (0.45, 0.5)
    } else if has(&["thigh", "upperleg", "upleg"]) {
        (1.2, 0.8)
    } else if has(&["upperarm", "shoulder", "clavicle", "arm"]) {
        (1.4, 1.2)
    } else if has(&["foot", "hand", "palm", "ankle", "wrist"]) {
        (0.6, 0.5)
    } else {
        (0.8, 0.6)
    }
}

/// The bodies, ordered parents first, with their capsules in their
/// bones' frames and their joints.
fn assemble(
    rig: &Rig,
    chosen: &[usize],
    shapes: &[Capsule],
    parents: &[Option<usize>],
    rest: &[Mat4],
    height: f32,
) -> RagdollLayout {
    // Parents first: depth order.
    let depth = |i: usize| {
        let mut d = 0;
        let mut at = parents[i];
        while let Some(p) = at {
            d += 1;
            if d > parents.len() {
                break;
            }
            at = parents[p];
        }
        d
    };
    let mut order: Vec<usize> = (0..chosen.len()).collect();
    order.sort_by_key(|&i| (depth(i), i));
    let slot = |i: usize| order.iter().position(|&o| o == i);
    let mut bodies: Vec<RagdollBody> = order
        .iter()
        .map(|&i| {
            let node = chosen[i];
            let (rot, at) = (frame_rotation(&rest[node]), origin(&rest[node]));
            let local = |p: Vec3| rot.inverse() * (p - at);
            let joint = parents[i].map(|p| joint_for(rig, chosen, shapes, rest, i, p));
            RagdollBody {
                node,
                name: rig.nodes[node].name.clone(),
                parent: parents[i].and_then(slot),
                a: local(shapes[i].a),
                b: local(shapes[i].b),
                radius: shapes[i].radius,
                share: 0.0,
                joint,
            }
        })
        .collect();
    // Mass goes by volume, but no one part (a robot's big round head)
    // outweighs the rest by too much, and none is so light the joints
    // can't hold it (very uneven neighbours make a joint solver shake).
    let total: f32 = bodies.iter().map(RagdollBody::volume).sum();
    let weights: Vec<f32> = bodies
        .iter()
        .map(|b| b.volume().clamp(total * 0.03, total * 0.25))
        .collect();
    let sum: f32 = weights.iter().sum();
    for (body, w) in bodies.iter_mut().zip(weights) {
        body.share = if sum > 0.0 { w / sum } else { 0.0 };
    }
    RagdollLayout { bodies, height }
}

/// The joint between chosen bone `i` and its parent body `p`, set in the
/// rest pose.
fn joint_for(
    rig: &Rig,
    chosen: &[usize],
    shapes: &[Capsule],
    rest: &[Mat4],
    i: usize,
    p: usize,
) -> RagdollJoint {
    let (child, parent) = (chosen[i], chosen[p]);
    let anchor = origin(&rest[child]);
    let (rot_c, rot_p) = (frame_rotation(&rest[child]), frame_rotation(&rest[parent]));
    let name = &rig.nodes[child].name;
    let b = base(name);
    // The direction the bone runs: from its joint out through its body.
    let out = shapes[i].centre() - anchor;
    let along = if out.length() > 1e-6 {
        out.normalize()
    } else {
        (anchor - origin(&rest[parent])).normalize_or(Vec3::NEG_Y)
    };
    let hinge = SHINS.contains(&b.as_str()) || FOREARMS.contains(&b.as_str());
    let (axis, kind) = if hinge {
        // The bone above runs `upper`; the hinge is across it and the
        // model's forward (+Z). Turning positively swings this bone
        // forwards: elbows bend that way, knees the other.
        let upper = (anchor - origin(&rest[parent])).normalize_or(Vec3::NEG_Y);
        let across = upper.cross(Vec3::Z);
        let axis = if across.length() > 0.3 {
            across.normalize()
        } else {
            Vec3::X
        };
        let knee = SHINS.contains(&b.as_str());
        let kind = if knee {
            RagdollJointKind::Hinge {
                min: -2.6,
                max: 0.05,
            }
        } else {
            RagdollJointKind::Hinge {
                min: -0.05,
                max: 2.6,
            }
        };
        (axis, kind)
    } else {
        let (swing, twist) = ball_limits(name);
        (along, RagdollJointKind::Ball { swing, twist })
    };
    let frame = Quat::from_rotation_arc(Vec3::X, axis);
    RagdollJoint {
        anchor_parent: rot_p.inverse() * (anchor - origin(&rest[parent])),
        anchor_child: Vec3::ZERO,
        frame_parent: (rot_p.inverse() * frame).normalize(),
        frame_child: (rot_c.inverse() * frame).normalize(),
        kind,
    }
}

/// What scripts see of an entity's ragdoll.
#[derive(Debug, Clone, PartialEq)]
pub struct RagdollStatus {
    /// The component's settings.
    pub ragdoll: Ragdoll,
    /// Physics has it right now.
    pub limp: bool,
    /// How much of the pose is the ragdoll's (1 limp, falling to 0 while
    /// the animation takes over again).
    pub weight: f32,
    /// Limp and come to rest.
    pub resting: bool,
    /// Where its hips (the root body) are while it shows, in the world.
    pub hips: Option<Vec3>,
}

/// `entity`'s ragdoll as scripts see it, or `None` without a [`Ragdoll`].
#[must_use]
pub fn ragdoll_status(world: &crate::SceneWorld, entity: hecs::Entity) -> Option<RagdollStatus> {
    let ragdoll = world.get::<&Ragdoll>(entity).ok()?.sanitized();
    let pose = world.get::<&RagdollPose>(entity).ok();
    let (limp, weight, resting, hips) = pose.map_or((false, 0.0, false, None), |p| {
        let hips = p
            .showing()
            .then(|| p.bones.first().map(|b| b.position))
            .flatten();
        (p.simulating, p.weight, p.simulating && p.resting, hips)
    });
    Some(RagdollStatus {
        ragdoll,
        limp,
        weight,
        resting,
        hips,
    })
}

/// A script's changes to a ragdoll; `None` leaves a setting alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RagdollEdit {
    /// Go limp (true) or get back to the animation (false).
    pub active: Option<bool>,
    /// A new mass (kg).
    pub mass: Option<f32>,
    /// A new stiffness (0 to 1).
    pub stiffness: Option<f32>,
    /// A new blend-back time (seconds).
    pub blend_time: Option<f32>,
    /// Pushes for the next physics step.
    pub pushes: Vec<RagdollPush>,
}

impl RagdollEdit {
    /// Applies the changes to `entity` (giving it a [`Ragdoll`] if it has
    /// none and the edit switches it on). Returns `false` when the entity
    /// doesn't exist.
    pub fn apply(self, world: &mut crate::SceneWorld, entity: hecs::Entity) -> bool {
        if !world.contains(entity) {
            return false;
        }
        if world.get::<&Ragdoll>(entity).is_err() {
            let _ = world.insert_one(entity, Ragdoll::default());
        }
        if let Ok(mut ragdoll) = world.get::<&mut Ragdoll>(entity) {
            if let Some(on) = self.active {
                ragdoll.active = on;
            }
            if let Some(mass) = self.mass {
                ragdoll.mass = mass;
            }
            if let Some(stiffness) = self.stiffness {
                ragdoll.stiffness = stiffness;
            }
            if let Some(blend_time) = self.blend_time {
                ragdoll.blend_time = blend_time;
            }
            *ragdoll = ragdoll.sanitized();
        }
        for push in self.pushes {
            push_ragdoll(world, entity, push.bone, push.impulse);
        }
        true
    }
}

/// Switches `entity`'s ragdoll on or off (giving it a [`Ragdoll`] if it
/// has none). Returns `false` when the entity doesn't exist.
pub fn set_ragdoll(world: &mut crate::SceneWorld, entity: hecs::Entity, on: bool) -> bool {
    if let Ok(mut ragdoll) = world.get::<&mut Ragdoll>(entity) {
        ragdoll.active = on;
        return true;
    }
    let ragdoll = Ragdoll {
        active: on,
        ..Ragdoll::default()
    };
    world.insert_one(entity, ragdoll).is_ok()
}

/// Queues a push on `entity`'s ragdoll for the next physics step: on
/// `bone` (`None`: the whole body). Pushes on a model that isn't limp are
/// dropped. Returns `false` when the entity doesn't exist.
pub fn push_ragdoll(
    world: &mut crate::SceneWorld,
    entity: hecs::Entity,
    bone: Option<String>,
    impulse: Vec3,
) -> bool {
    if !impulse.is_finite() {
        return world.contains(entity);
    }
    let push = RagdollPush { bone, impulse };
    if let Ok(mut state) = world.get::<&mut RagdollPose>(entity) {
        state.pushes.push(push);
        return true;
    }
    let state = RagdollPose {
        pushes: vec![push],
        ..RagdollPose::default()
    };
    world.insert_one(entity, state).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_kept_in_range() {
        let wild = Ragdoll {
            active: true,
            mass: -5.0,
            stiffness: f32::NAN,
            blend_time: 99.0,
        }
        .sanitized();
        assert!(wild.active);
        assert!((wild.mass - 1.0).abs() < 1e-6);
        assert!(wild.stiffness.abs() < 1e-6);
        assert!((wild.blend_time - 10.0).abs() < 1e-6);
    }

    #[test]
    fn the_blend_back_eases_from_the_last_pose_to_the_animation() {
        use automata_assets::anim::RigNode;
        let rig = Rig {
            nodes: vec![RigNode {
                name: "Hips".into(),
                parent: None,
                rest: Trs::IDENTITY,
            }],
            joints: vec![(0, Mat4::IDENTITY)],
            import: Mat4::IDENTITY,
            clips: Vec::new(),
            springs: automata_assets::springs::Springs::default(),
        };
        let place = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let mut state = RagdollPose {
            bones: vec![RagdollBone {
                node: 0,
                position: Vec3::new(10.0, 0.2, 1.0),
                rotation: Quat::from_rotation_x(1.5),
            }],
            simulating: true,
            blend_time: 0.5,
            ..RagdollPose::default()
        };
        let mut pose = rig.rest_pose();
        assert!(state.blend_into(&rig, place, &mut pose, 0.1));
        assert!(pose.0[0].translation.distance(Vec3::new(0.0, 0.2, 1.0)) < 1e-5);
        assert!(pose.0[0].rotation.angle_between(Quat::from_rotation_x(1.5)) < 1e-4);
        // Switched off: halfway through, it's halfway back.
        state.simulating = false;
        let mut pose = rig.rest_pose();
        state.blend_into(&rig, place, &mut pose, 0.25);
        assert!((state.weight - 0.5).abs() < 1e-5);
        let mut pose = rig.rest_pose();
        state.blend_into(&rig, place, &mut pose, 0.25);
        let halfway = pose.0[0].translation;
        assert!((halfway.z - 0.5).abs() < 1e-4, "{halfway}");
        assert!(state.weight.abs() < 1e-6);
        assert!(!state.showing());
        let mut pose = rig.rest_pose();
        assert!(!state.blend_into(&rig, place, &mut pose, 0.25));
        assert_eq!(pose, rig.rest_pose());
    }
}
