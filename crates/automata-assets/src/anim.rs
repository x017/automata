//! Skeletal and node animation: a model's [`Rig`] (its node hierarchy, the
//! joints that skinned geometry follows) and its [`Clip`]s.
//!
//! A clip animates nodes (translation, rotation, scale). Sampling a clip
//! writes into a [`Pose`], one local transform per node; [`Rig::palette`]
//! turns a pose into the joint matrices the renderer skins with.
//!
//! Two kinds of geometry follow the rig:
//!
//! - **Skinned** meshes (glTF skins): every vertex blends up to four
//!   joints by weight.
//! - **Rigid** meshes under an animated node (a robot's arm, a door): the
//!   whole mesh follows its node, as one joint with weight 1.
//!
//! Joint `j`'s matrix is `import × global(node_j) × inverse_bind_j`, which
//! takes a vertex from bind space into model space (after the model's
//! import settings).

use automata_renderer::{SkinVertex, Vertex};
use glam::{Mat3, Mat4, Quat, Vec3};

pub use automata_renderer::MAX_JOINTS;

/// A local transform: translation, rotation, scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trs {
    /// Translation.
    pub translation: Vec3,
    /// Rotation (unit quaternion).
    pub rotation: Quat,
    /// Scale.
    pub scale: Vec3,
}

impl Default for Trs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Trs {
    /// No movement.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// The transform as a matrix.
    #[must_use]
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Splits a matrix into translation, rotation and scale.
    #[must_use]
    pub fn from_matrix(m: Mat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Self {
            translation,
            rotation: rotation.normalize(),
            scale,
        }
    }

    /// Blends towards `other`: `t = 0` keeps `self`, `t = 1` gives `other`.
    #[must_use]
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: nlerp(self.rotation, other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Normalized linear quaternion blend along the shorter arc.
fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    let q = a * (1.0 - t) + b * t;
    if q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        a
    }
}

/// One node of a rig.
#[derive(Debug, Clone, PartialEq)]
pub struct RigNode {
    /// The node's name in the file (`"Node 3"` when it has none).
    pub name: String,
    /// The parent, always earlier in [`Rig::nodes`].
    pub parent: Option<usize>,
    /// The transform in the file (the rest pose).
    pub rest: Trs,
}

/// What a channel animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Property {
    /// The node's translation.
    Translation,
    /// The node's rotation.
    Rotation,
    /// The node's scale.
    Scale,
}

impl Property {
    fn width(self) -> usize {
        if self == Self::Rotation {
            4
        } else {
            3
        }
    }
}

/// How a channel fills the time between keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    /// Straight lines (spherical for rotations).
    Linear,
    /// Hold each key until the next.
    Step,
    /// Hermite curves with per-key tangents.
    CubicSpline,
}

/// One animated property of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// The node (index into [`Rig::nodes`]).
    pub node: usize,
    /// What is animated.
    pub property: Property,
    /// The interpolation between keys.
    pub interpolation: Interpolation,
    /// Key times in seconds, increasing.
    pub times: Vec<f32>,
    /// Key values, flat: 3 floats per key (4 for rotations, `x y z w`).
    /// Cubic-spline keys hold in-tangent, value, out-tangent.
    pub values: Vec<f32>,
}

impl Channel {
    /// The value at `time` (clamped to the first and last key).
    fn sample(&self, time: f32) -> Option<[f32; 4]> {
        let keys = self.times.len();
        let width = self.property.width();
        let cubic = self.interpolation == Interpolation::CubicSpline;
        let stride = if cubic { 3 * width } else { width };
        if keys == 0 || self.values.len() < keys * stride {
            return None;
        }
        // Part 0/1/2 of a cubic key: in-tangent, value, out-tangent.
        let value = |key: usize, part: usize| -> [f32; 4] {
            let at = key * stride + part * width;
            let mut out = [0.0; 4];
            out[..width].copy_from_slice(&self.values[at..at + width]);
            out
        };
        let mid = usize::from(cubic);
        if keys == 1 || time <= self.times[0] {
            return Some(value(0, mid));
        }
        if time >= self.times[keys - 1] {
            return Some(value(keys - 1, mid));
        }
        // The key at or before `time`.
        let key = self.times.partition_point(|&t| t <= time).saturating_sub(1);
        let (start, end) = (self.times[key], self.times[key + 1]);
        let span = end - start;
        let frac = if span > 1e-9 {
            (time - start) / span
        } else {
            0.0
        };
        Some(match self.interpolation {
            Interpolation::Step => value(key, 0),
            Interpolation::Linear => {
                let (from, to) = (value(key, 0), value(key + 1, 0));
                if self.property == Property::Rotation {
                    Quat::from_array(from)
                        .slerp(Quat::from_array(to), frac)
                        .to_array()
                } else {
                    std::array::from_fn(|i| from[i] + (to[i] - from[i]) * frac)
                }
            }
            Interpolation::CubicSpline => {
                let (v0, in_tangent) = (value(key, 1), value(key + 1, 0));
                let v1 = value(key + 1, 1);
                let out_tangent = value(key, 2);
                let f2 = frac * frac;
                let f3 = f2 * frac;
                let h00 = 2.0 * f3 - 3.0 * f2 + 1.0;
                let h10 = f3 - 2.0 * f2 + frac;
                let h01 = -2.0 * f3 + 3.0 * f2;
                let h11 = f3 - f2;
                std::array::from_fn(|i| {
                    h00 * v0[i]
                        + h10 * span * out_tangent[i]
                        + h01 * v1[i]
                        + h11 * span * in_tangent[i]
                })
            }
        })
    }
}

/// A named moment in a clip that scripts hear about (a footstep, the
/// frame a sword hits). Set in the model's import settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipEvent {
    /// Seconds into the clip.
    pub time: f32,
    /// The event's name (`"footstep"`).
    pub name: String,
}

/// A named animation: channels over a duration.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// The name in the file (`"Animation 2"` when it has none).
    pub name: String,
    /// The last key time, in seconds.
    pub duration: f32,
    /// What moves.
    pub channels: Vec<Channel>,
    /// Events, by time.
    pub events: Vec<ClipEvent>,
}

impl Clip {
    /// This clip moved onto `rig` under `name`: each channel follows its
    /// node by name, from `nodes` (the clip's own file's nodes) to the
    /// rig's. Channels for nodes the rig lacks are dropped; `None` when
    /// none are left.
    #[must_use]
    pub fn retarget(&self, nodes: &[RigNode], rig: &Rig, name: String) -> Option<Clip> {
        let channels: Vec<Channel> = self
            .channels
            .iter()
            .filter_map(|c| {
                let node = rig.node(&nodes.get(c.node)?.name)?;
                Some(Channel { node, ..c.clone() })
            })
            .collect();
        (!channels.is_empty()).then_some(Clip {
            name,
            duration: self.duration,
            channels,
            events: Vec::new(),
        })
    }

    /// Node `node`'s animated translation at `time`, or `None` when the
    /// clip doesn't move it.
    #[must_use]
    pub fn translation_at(&self, node: usize, time: f32) -> Option<Vec3> {
        self.channels
            .iter()
            .find(|c| c.node == node && c.property == Property::Translation)
            .and_then(|c| c.sample(time))
            .map(|v| Vec3::new(v[0], v[1], v[2]))
    }

    /// Adds an event at `time` (clamped to the clip), keeping them by time.
    pub fn add_event(&mut self, time: f32, name: impl Into<String>) {
        let time = if time.is_finite() {
            time.clamp(0.0, self.duration)
        } else {
            0.0
        };
        let at = self.events.partition_point(|e| e.time <= time);
        self.events.insert(
            at,
            ClipEvent {
                time,
                name: name.into(),
            },
        );
    }

    /// Writes the clip's values at `time` (seconds, clamped to the clip)
    /// into `pose`. Nodes the clip doesn't animate keep their values.
    pub fn sample_into(&self, time: f32, pose: &mut Pose) {
        for channel in &self.channels {
            let Some(v) = channel.sample(time) else {
                continue;
            };
            let Some(trs) = pose.0.get_mut(channel.node) else {
                continue;
            };
            match channel.property {
                Property::Translation => trs.translation = Vec3::new(v[0], v[1], v[2]),
                Property::Scale => trs.scale = Vec3::new(v[0], v[1], v[2]),
                Property::Rotation => {
                    let q = Quat::from_array(v);
                    if q.length_squared() > 1e-12 {
                        trs.rotation = q.normalize();
                    }
                }
            }
        }
    }
}

/// How a clip carries its character across the ground (root motion): the
/// sideways and forward movement of its top moving bone, which a game can
/// take out of the pose and apply to the entity instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipMotion {
    /// The bone: the clip's highest node with a translation channel.
    pub node: usize,
    /// From the bone's parent space to model space (import settings
    /// included).
    to_model: Mat3,
    from_model: Mat3,
    /// The bone's translation at the clip's start.
    start: Vec3,
}

impl ClipMotion {
    /// The motion of `clip` on `rig`, or `None` when it moves no bone.
    #[must_use]
    pub fn of(rig: &Rig, clip: &Clip) -> Option<Self> {
        let depth = |mut n: usize| {
            let mut d = 0;
            while let Some(p) = rig.nodes.get(n).and_then(|node| node.parent) {
                n = p;
                d += 1;
            }
            d
        };
        let node = clip
            .channels
            .iter()
            .filter(|c| c.property == Property::Translation && c.node < rig.nodes.len())
            .map(|c| c.node)
            .min_by_key(|&n| depth(n))?;
        let mut parent = Mat4::IDENTITY;
        let mut at = rig.nodes[node].parent;
        while let Some(p) = at {
            parent = rig.nodes[p].rest.matrix() * parent;
            at = rig.nodes[p].parent;
        }
        let to_model = Mat3::from_mat4(rig.import * parent);
        if to_model.determinant().abs() < 1e-12 {
            return None;
        }
        Some(Self {
            node,
            to_model,
            from_model: to_model.inverse(),
            start: clip.translation_at(node, 0.0)?,
        })
    }

    /// How far across the ground (model space, `y` = 0) the bone is at
    /// `time`, from where it started.
    #[must_use]
    pub fn offset(&self, clip: &Clip, time: f32) -> Vec3 {
        let Some(t) = clip.translation_at(self.node, time) else {
            return Vec3::ZERO;
        };
        let d = self.to_model * (t - self.start);
        Vec3::new(d.x, 0.0, d.z)
    }

    /// Takes the ground movement at `time` out of `pose`, so the bone
    /// stays over the entity's origin (up and down movement is kept).
    pub fn pin(&self, clip: &Clip, time: f32, pose: &mut Pose) {
        let offset = self.offset(clip, time);
        if let Some(trs) = pose.0.get_mut(self.node) {
            trs.translation -= self.from_model * offset;
        }
    }

    /// The ground movement (model space) of a playhead going from `from`
    /// by `advance` seconds (negative backwards) through `clip`: across
    /// the wraps when it loops, up to an end otherwise.
    #[must_use]
    pub fn travel(&self, clip: &Clip, from: f32, advance: f32, looping: bool) -> Vec3 {
        let d = clip.duration;
        if looping && d > 0.0 {
            let unwrapped = from + advance;
            let laps = (unwrapped / d).floor();
            let to = unwrapped - laps * d;
            self.offset(clip, to) - self.offset(clip, from) + self.offset(clip, d) * laps
        } else {
            let to = (from + advance).clamp(0.0, d.max(0.0));
            self.offset(clip, to) - self.offset(clip, from)
        }
    }
}

/// Every node's local transform at one moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pose(pub Vec<Trs>);

impl Pose {
    /// Blends towards `other` by `weight` (0 keeps this pose, 1 gives
    /// `other`), node by node. Used for crossfades.
    pub fn blend(&mut self, other: &Pose, weight: f32) {
        let w = weight.clamp(0.0, 1.0);
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a = a.lerp(b, w);
        }
    }
}

/// A model's node hierarchy, joints and animations.
#[derive(Debug, Clone, PartialEq)]
pub struct Rig {
    /// Every node, parents before children.
    pub nodes: Vec<RigNode>,
    /// The joint palette: `(node, inverse bind matrix)` per joint. Skinned
    /// vertices name joints by their index here.
    pub joints: Vec<(usize, Mat4)>,
    /// The model's import-settings transform, applied after the rig.
    pub import: Mat4,
    /// The animations, in file order.
    pub clips: Vec<Clip>,
    /// Spring bones from the import settings (see [`crate::springs`]).
    pub springs: crate::springs::Springs,
}

impl Rig {
    /// The pose stored in the file.
    #[must_use]
    pub fn rest_pose(&self) -> Pose {
        Pose(self.nodes.iter().map(|n| n.rest).collect())
    }

    /// The clip called `name`.
    #[must_use]
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }

    /// The clips' names, in file order.
    #[must_use]
    pub fn clip_names(&self) -> Vec<&str> {
        self.clips.iter().map(|c| c.name.as_str()).collect()
    }

    /// The node called `name`.
    #[must_use]
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Every node's transform relative to the model (before import
    /// settings) for `pose`.
    #[must_use]
    pub fn globals(&self, pose: &Pose) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            let local = pose.0.get(i).unwrap_or(&node.rest).matrix();
            let global = match node.parent {
                Some(p) if p < out.len() => out[p] * local,
                _ => local,
            };
            out.push(global);
        }
        out
    }

    /// The joint matrices for `pose` (see the module docs), into `out`.
    pub fn palette(&self, pose: &Pose, out: &mut Vec<Mat4>) {
        let globals = self.globals(pose);
        out.clear();
        out.extend(self.joints.iter().map(|&(node, inverse_bind)| {
            self.import * globals.get(node).copied().unwrap_or(Mat4::IDENTITY) * inverse_bind
        }));
    }

    /// The joint matrices of the rest pose.
    #[must_use]
    pub fn rest_palette(&self) -> Vec<Mat4> {
        let mut out = Vec::new();
        self.palette(&self.rest_pose(), &mut out);
        out
    }
}

/// How a part's vertices follow the rig: up to four joints each, by weight.
#[derive(Debug, Clone, PartialEq)]
pub struct PartSkin {
    /// The vertices in bind space (what the joint matrices transform),
    /// matching the part's mesh vertex for vertex.
    pub bind: Vec<Vertex>,
    /// Joints (into [`Rig::joints`]) and weights per vertex.
    pub skin: Vec<SkinVertex>,
}

impl PartSkin {
    /// Skins the bind vertices with `palette` on the CPU (rest poses for
    /// bounds and collision shapes, tests).
    #[must_use]
    pub fn posed(&self, palette: &[Mat4]) -> Vec<Vertex> {
        self.bind
            .iter()
            .zip(&self.skin)
            .map(|(v, skin)| {
                let mut m = Mat4::ZERO;
                let mut total = 0.0;
                for (j, w) in skin.joints.iter().zip(&skin.weights) {
                    if *w > 0.0 {
                        m += palette
                            .get(usize::from(*j))
                            .copied()
                            .unwrap_or(Mat4::IDENTITY)
                            * *w;
                        total += *w;
                    }
                }
                if total < 1e-6 {
                    m = Mat4::IDENTITY;
                }
                let mut out = *v;
                out.position = m.transform_point3(Vec3::from(v.position)).to_array();
                out.normal = m
                    .transform_vector3(Vec3::from(v.normal))
                    .normalize_or(Vec3::Y)
                    .to_array();
                out
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rig whose `hips` (under an `armature` turned 90° about X and
    /// scaled by 0.01, as Mixamo exports are) walks 100 units along the
    /// armature's Y over 2 s while bobbing along its Z.
    fn walker() -> (Rig, Clip) {
        let armature = Trs {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(0.01),
        };
        let clip = Clip {
            name: "Walk".into(),
            duration: 2.0,
            channels: vec![Channel {
                node: 1,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0, 2.0],
                values: vec![0.0, 0.0, 90.0, 0.0, 50.0, 80.0, 0.0, 100.0, 90.0],
            }],
            events: Vec::new(),
        };
        let rig = Rig {
            nodes: vec![
                RigNode {
                    name: "armature".into(),
                    parent: None,
                    rest: armature,
                },
                RigNode {
                    name: "hips".into(),
                    parent: Some(0),
                    rest: Trs::IDENTITY,
                },
            ],
            joints: vec![(1, Mat4::IDENTITY)],
            import: Mat4::IDENTITY,
            clips: vec![clip.clone()],
            springs: crate::springs::Springs::default(),
        };
        (rig, clip)
    }

    #[test]
    fn clip_motion_measures_ground_travel_in_model_space() {
        let (rig, clip) = walker();
        let motion = ClipMotion::of(&rig, &clip).expect("the hips move");
        assert_eq!(motion.node, 1);
        // Armature +Y is model -Z after the turn; 100 units are 1 m. The
        // bob (armature Z, model Y) isn't ground travel.
        let at = |t| motion.offset(&clip, t);
        assert!(at(0.0).abs_diff_eq(Vec3::ZERO, 1e-6));
        assert!(
            at(1.0).abs_diff_eq(Vec3::new(0.0, 0.0, -0.5), 1e-5),
            "{}",
            at(1.0)
        );
        // Travel across a loop: from 1.5 s on by 1 s ends at 0.5 s of the
        // next lap: 0.25 m to the end, then 0.25 m more.
        let lap = motion.travel(&clip, 1.5, 1.0, true);
        assert!(lap.abs_diff_eq(Vec3::new(0.0, 0.0, -0.5), 1e-5), "{lap}");
        // Three whole laps, and backwards; without looping it stops at the end.
        assert!(motion
            .travel(&clip, 0.0, 6.0, true)
            .abs_diff_eq(Vec3::new(0.0, 0.0, -3.0), 1e-4));
        assert!(motion
            .travel(&clip, 0.5, -1.0, true)
            .abs_diff_eq(Vec3::new(0.0, 0.0, 0.5), 1e-5));
        assert!(motion
            .travel(&clip, 1.5, 5.0, false)
            .abs_diff_eq(Vec3::new(0.0, 0.0, -0.25), 1e-5));

        // Pinned, the hips keep their bob but stay over the origin.
        let mut pose = rig.rest_pose();
        clip.sample_into(1.0, &mut pose);
        motion.pin(&clip, 1.0, &mut pose);
        let hips = rig.globals(&pose)[1].w_axis.truncate();
        assert!(hips.abs_diff_eq(Vec3::new(0.0, 0.8, 0.0), 1e-5), "{hips}");
    }

    #[test]
    fn clips_without_moving_bones_have_no_motion_and_events_stay_sorted() {
        let (rig, mut clip) = walker();
        clip.channels[0].property = Property::Scale;
        assert_eq!(ClipMotion::of(&rig, &clip), None);
        clip.add_event(1.5, "b");
        clip.add_event(0.2, "a");
        clip.add_event(9.0, "end");
        let names: Vec<(f32, &str)> = clip
            .events
            .iter()
            .map(|e| (e.time, e.name.as_str()))
            .collect();
        assert_eq!(names, [(0.2, "a"), (1.5, "b"), (2.0, "end")]);
    }

    fn channel(property: Property, interpolation: Interpolation, values: Vec<f32>) -> Channel {
        Channel {
            node: 0,
            property,
            interpolation,
            times: vec![0.0, 1.0, 2.0],
            values,
        }
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn linear_step_and_clamping() {
        let c = channel(
            Property::Translation,
            Interpolation::Linear,
            vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 4.0, 0.0],
        );
        assert!(close(c.sample(0.5).unwrap(), [1.0, 0.0, 0.0, 0.0]));
        assert!(close(c.sample(1.5).unwrap(), [2.0, 2.0, 0.0, 0.0]));
        assert!(close(c.sample(-1.0).unwrap(), [0.0; 4]));
        assert!(close(c.sample(9.0).unwrap(), [2.0, 4.0, 0.0, 0.0]));
        let s = Channel {
            interpolation: Interpolation::Step,
            ..c
        };
        assert!(close(s.sample(0.99).unwrap(), [0.0; 4]));
        assert!(close(s.sample(1.0).unwrap(), [2.0, 0.0, 0.0, 0.0]));
    }

    #[test]
    fn rotations_slerp_and_cubic_splines_pass_through_keys() {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let c = channel(
            Property::Rotation,
            Interpolation::Linear,
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, half, half, 0.0, 0.0, 1.0, 0.0],
        );
        // Halfway between 0° and 90° about Z is 45°.
        let q = Quat::from_array(c.sample(0.5).unwrap());
        let expected = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        assert!(q.dot(expected).abs() > 0.9999);

        // Cubic: zero tangents, keys 0 → 1 → 0 on x: smooth, hits keys.
        let mut values = Vec::new();
        for x in [0.0, 1.0, 0.0] {
            values.extend([0.0, 0.0, 0.0, x, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
        let c = channel(Property::Translation, Interpolation::CubicSpline, values);
        assert!(close(c.sample(1.0).unwrap(), [1.0, 0.0, 0.0, 0.0]));
        assert!(close(c.sample(0.5).unwrap(), [0.5, 0.0, 0.0, 0.0]));
        assert!(c.sample(0.25).unwrap()[0] < 0.25, "eases in");
    }

    #[test]
    fn palettes_follow_the_hierarchy_and_blend() {
        // root at the origin, child 1 up; the child's bind pose is its rest.
        let child_rest = Trs {
            translation: Vec3::Y,
            ..Trs::IDENTITY
        };
        let rig = Rig {
            nodes: vec![
                RigNode {
                    name: "root".into(),
                    parent: None,
                    rest: Trs::IDENTITY,
                },
                RigNode {
                    name: "child".into(),
                    parent: Some(0),
                    rest: child_rest,
                },
            ],
            joints: vec![(1, child_rest.matrix().inverse())],
            import: Mat4::from_scale(Vec3::splat(2.0)),
            clips: Vec::new(),
            springs: crate::springs::Springs::default(),
        };
        // At rest a joint matrix is the import transform alone.
        let rest = rig.rest_palette();
        assert!(rest[0].abs_diff_eq(Mat4::from_scale(Vec3::splat(2.0)), 1e-5));
        // Turning the root 90° about Z swings the child's point (0,1,0)+x.
        let mut pose = rig.rest_pose();
        pose.0[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut palette = Vec::new();
        rig.palette(&pose, &mut palette);
        let p = palette[0].transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!(p.abs_diff_eq(Vec3::new(-2.0, 0.0, 0.0), 1e-4), "{p}");
        // Halfway blend: 45°.
        let mut half = rig.rest_pose();
        half.blend(&pose, 0.5);
        assert!(half.0[0]
            .rotation
            .abs_diff_eq(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4), 1e-4));
        assert_eq!(rig.node("child"), Some(1));
    }
}
