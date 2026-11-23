//! Physics joints: hinges, ropes, springs and friends.
//!
//! A [`Joint`] on an entity ties its rigid body to another entity's body
//! (named by [`Joint::connected`]) or, with no name, to a fixed point in
//! the world. Entities are referred to by name, like everywhere else in
//! scene files and scripts, so a joint survives saving and loading and
//! reads naturally in RON (`connected: Some("Door Frame")`).
//!
//! Joints start *satisfied*: the anchor is placed on this entity, and the
//! matching point on the other body is wherever that anchor sits at the
//! moment the joint is created. A door hinged to its frame therefore hangs
//! exactly where it was placed, and a rope with `length: 0.0` takes the
//! distance between its ends as its length.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// What a [`Joint`] lets the two bodies do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum JointKind {
    /// Welded together: no relative movement at all.
    Fixed,
    /// Turns around one axis, like a door or a wheel. Limits are angles
    /// (radians); a motor spins it.
    #[default]
    Hinge,
    /// Turns any way around the anchor, like a shoulder or a chain link.
    /// Limits are how far it may swing from where it started (radians).
    Ball,
    /// Slides along one axis, like a drawer or a piston. Limits are
    /// distances (metres); a motor pushes it.
    Slider,
    /// Keeps the anchors no further apart than [`Joint::length`]; closer is
    /// fine, like a rope or a chain.
    Rope,
    /// Pulls the anchors towards [`Joint::length`] apart with
    /// [`Joint::stiffness`] and [`Joint::damping`], like a bungee.
    Spring,
}

impl JointKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 6] = [
        Self::Hinge,
        Self::Ball,
        Self::Slider,
        Self::Fixed,
        Self::Rope,
        Self::Spring,
    ];

    /// A short display name (`"Hinge"`).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Hinge => "Hinge",
            Self::Ball => "Ball",
            Self::Slider => "Slider",
            Self::Rope => "Rope",
            Self::Spring => "Spring",
        }
    }

    /// The lowercase name scripts and Python use (`"hinge"`).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Hinge => "hinge",
            Self::Ball => "ball",
            Self::Slider => "slider",
            Self::Rope => "rope",
            Self::Spring => "spring",
        }
    }

    /// The kind called `name` (`"hinge"`, case-insensitive), if any.
    #[must_use]
    pub fn from_key(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.key().eq_ignore_ascii_case(name.trim()))
    }

    /// Whether the kind has an axis ([`Joint::axis`]).
    #[must_use]
    pub fn has_axis(self) -> bool {
        matches!(self, Self::Hinge | Self::Slider)
    }

    /// Whether the kind takes [`Joint::limits`].
    #[must_use]
    pub fn has_limits(self) -> bool {
        matches!(self, Self::Hinge | Self::Slider | Self::Ball)
    }

    /// Whether the kind takes a [`Joint::motor`].
    #[must_use]
    pub fn has_motor(self) -> bool {
        matches!(self, Self::Hinge | Self::Slider)
    }

    /// Whether the kind uses [`Joint::length`].
    #[must_use]
    pub fn has_length(self) -> bool {
        matches!(self, Self::Rope | Self::Spring)
    }
}

/// Drives a hinge or slider: towards a speed, or towards a position like a
/// spring (a door that swings shut, a piston that returns).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JointMotor {
    /// The speed to reach: radians per second for a hinge, metres per
    /// second for a slider.
    pub speed: f32,
    /// A position to pull towards (radians or metres from where the joint
    /// started), or `None` to just hold `speed`.
    pub target: Option<f32>,
    /// How hard it pulls towards `target`. The defaults (400 / 80) reach a
    /// target in about half a second without overshooting; halve both for
    /// a lazier swing.
    pub stiffness: f32,
    /// How hard it resists moving at anything but `speed`.
    pub damping: f32,
    /// The most force (or torque) it may use; `0.0` means no limit.
    pub max_force: f32,
}

impl Default for JointMotor {
    fn default() -> Self {
        Self {
            speed: 0.0,
            target: None,
            stiffness: 400.0,
            damping: 80.0,
            max_force: 0.0,
        }
    }
}

impl JointMotor {
    /// Spins (or slides) at `speed`.
    #[must_use]
    pub fn speed(speed: f32) -> Self {
        Self {
            speed,
            ..Self::default()
        }
    }

    /// Pulls towards `target` like a spring.
    #[must_use]
    pub fn target(target: f32) -> Self {
        Self {
            target: Some(target),
            ..Self::default()
        }
    }
}

/// Component: ties this entity's rigid body to another body, or to the
/// world.
///
/// ```rust
/// # use automata_physics::{Joint, JointKind};
/// # use glam::Vec3;
/// // A door that turns on its left edge, at most 100° either way.
/// let hinge = Joint::hinge(Vec3::Y)
///     .connected_to("Door Frame")
///     .anchor(Vec3::new(-0.5, 0.0, 0.0))
///     .limits(-1.75, 1.75);
/// assert_eq!(hinge.kind, JointKind::Hinge);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Joint {
    /// What the joint lets the bodies do.
    pub kind: JointKind,
    /// The name of the entity this one is joined to, or `None` for a fixed
    /// point in the world.
    pub connected: Option<String>,
    /// Where the joint is, in this entity's own space (metres from its
    /// position, turning with it; scale doesn't apply).
    pub anchor: Vec3,
    /// The matching point on the other body, in its own space (or in the
    /// world, without one). `None` picks it for you: for a rope or spring,
    /// the other body's position (or, joined to the world, a point
    /// `length` above the anchor); for everything else, wherever the
    /// anchor is when the joint is made, so it starts satisfied.
    pub connected_anchor: Option<Vec3>,
    /// The axis a hinge turns around or a slider moves along, in this
    /// entity's own space.
    pub axis: Vec3,
    /// Hinge: smallest and largest angle (radians). Slider: nearest and
    /// furthest position (metres). Ball: how far it may swing each way
    /// (radians; the larger magnitude is used). All measured from where
    /// the joint started. `None` means no limit.
    pub limits: Option<[f32; 2]>,
    /// Rope: the longest it may get. Spring: the length it pulls towards.
    /// `0.0` uses the distance at the start.
    pub length: f32,
    /// Spring: how hard it pulls (scaled by the bodies' mass, so the same
    /// value feels the same on light and heavy bodies).
    pub stiffness: f32,
    /// Spring: how quickly it stops bouncing.
    pub damping: f32,
    /// Hinge or slider: a motor driving it.
    pub motor: Option<JointMotor>,
    /// Whether the two joined bodies still bump into each other (off by
    /// default: neighbouring chain links would fight).
    pub collide_connected: bool,
    /// The force (newtons) that snaps the joint; `0.0` means unbreakable.
    pub break_force: f32,
    /// Set to `false` to let go without removing the joint.
    pub enabled: bool,
}

impl Default for Joint {
    fn default() -> Self {
        Self {
            kind: JointKind::default(),
            connected: None,
            anchor: Vec3::ZERO,
            connected_anchor: None,
            axis: Vec3::Y,
            limits: None,
            length: 0.0,
            stiffness: 30.0,
            damping: 2.0,
            motor: None,
            collide_connected: false,
            break_force: 0.0,
            enabled: true,
        }
    }
}

impl Joint {
    /// A joint of `kind` with every other field at its default.
    #[must_use]
    pub fn new(kind: JointKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Welds this body to the other.
    #[must_use]
    pub fn fixed() -> Self {
        Self::new(JointKind::Fixed)
    }

    /// Turns around `axis` (this entity's space).
    #[must_use]
    pub fn hinge(axis: Vec3) -> Self {
        Self {
            axis,
            ..Self::new(JointKind::Hinge)
        }
    }

    /// Turns any way around the anchor.
    #[must_use]
    pub fn ball() -> Self {
        Self::new(JointKind::Ball)
    }

    /// Slides along `axis` (this entity's space).
    #[must_use]
    pub fn slider(axis: Vec3) -> Self {
        Self {
            axis,
            ..Self::new(JointKind::Slider)
        }
    }

    /// At most `length` apart (`0.0`: the distance at the start).
    #[must_use]
    pub fn rope(length: f32) -> Self {
        Self {
            length,
            ..Self::new(JointKind::Rope)
        }
    }

    /// Pulls towards `length` apart (`0.0`: the distance at the start).
    #[must_use]
    pub fn spring(length: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            length,
            stiffness,
            damping,
            ..Self::new(JointKind::Spring)
        }
    }

    /// Joins to the entity called `name`.
    #[must_use]
    pub fn connected_to(mut self, name: impl Into<String>) -> Self {
        self.connected = Some(name.into());
        self
    }

    /// Puts the joint at `anchor` in this entity's space.
    #[must_use]
    pub fn anchor(mut self, anchor: Vec3) -> Self {
        self.anchor = anchor;
        self
    }

    /// Puts the other end at `anchor` in the other body's space (or the
    /// world's).
    #[must_use]
    pub fn connected_anchor(mut self, anchor: Vec3) -> Self {
        self.connected_anchor = Some(anchor);
        self
    }

    /// Limits the joint to `min..=max` (radians or metres).
    #[must_use]
    pub fn limits(mut self, min: f32, max: f32) -> Self {
        self.limits = Some([min.min(max), min.max(max)]);
        self
    }

    /// Adds a motor.
    #[must_use]
    pub fn motor(mut self, motor: JointMotor) -> Self {
        self.motor = Some(motor);
        self
    }

    /// Snaps under more than `force` newtons.
    #[must_use]
    pub fn break_force(mut self, force: f32) -> Self {
        self.break_force = force.max(0.0);
        self
    }

    /// Lets the joined bodies collide with each other.
    #[must_use]
    pub fn collide_connected(mut self, on: bool) -> Self {
        self.collide_connected = on;
        self
    }

    /// Whether `other` differs from this joint in a way that needs the
    /// joint rebuilt (rather than adjusted in place): its kind, the bodies
    /// it joins, or its geometry.
    #[must_use]
    pub fn needs_rebuild(&self, other: &Self) -> bool {
        self.kind != other.kind
            || self.connected != other.connected
            || self.anchor != other.anchor
            || self.connected_anchor != other.connected_anchor
            || self.axis != other.axis
            || self.length.to_bits() != other.length.to_bits()
    }
}

/// Component the physics world keeps up to date on every entity with a
/// [`Joint`]: how far it has turned or slid, and whether it holds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JointState {
    /// Hinge: the angle (radians) from where it started. Slider: the
    /// position (metres). Rope, spring, ball and fixed: the distance
    /// between the anchors (metres).
    pub value: f32,
    /// Whether the joint is holding the bodies together right now.
    pub attached: bool,
    /// Whether it snapped (past its [`Joint::break_force`]).
    pub broken: bool,
    /// The force on the joint at the last step (newtons); once broken,
    /// the force that broke it.
    pub force: f32,
}

/// A script's change to a [`Joint`]: only the fields it set. Scripts
/// tune a running joint (motor, limits, letting go) without restating it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JointEdit {
    /// Turn it on or off.
    pub enabled: Option<bool>,
    /// A new motor as `(speed, target)`, or `Some(None)` to remove it. Its
    /// stiffness, damping and force limit stay as they were.
    pub motor: Option<Option<(f32, Option<f32>)>>,
    /// New limits, or `Some(None)` to remove them.
    pub limits: Option<Option<[f32; 2]>>,
    /// A new break force.
    pub break_force: Option<f32>,
    /// Join a different entity, or `Some(None)` for the world.
    pub connected: Option<Option<String>>,
}

impl JointEdit {
    /// Whether it changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Applies the edit.
    pub fn apply(&self, joint: &mut Joint) {
        if let Some(on) = self.enabled {
            joint.enabled = on;
        }
        if let Some(motor) = self.motor {
            joint.motor = motor.map(|(speed, target)| JointMotor {
                speed,
                target,
                ..joint.motor.unwrap_or_default()
            });
        }
        if let Some(limits) = self.limits {
            joint.limits = limits.map(|[a, b]| [a.min(b), a.max(b)]);
        }
        if let Some(force) = self.break_force {
            joint.break_force = force.max(0.0);
        }
        if let Some(connected) = &self.connected {
            joint.connected.clone_from(connected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_and_defaults() {
        let j = Joint::hinge(Vec3::X)
            .connected_to("Frame")
            .limits(1.0, -1.0)
            .motor(JointMotor::speed(2.0));
        assert_eq!(j.limits, Some([-1.0, 1.0]));
        assert_eq!(j.connected.as_deref(), Some("Frame"));
        assert!(j.enabled && !j.collide_connected);
        assert_eq!(Joint::rope(0.0).kind, JointKind::Rope);
        assert!(JointKind::Hinge.has_motor() && !JointKind::Rope.has_motor());
        assert_eq!(JointKind::from_key(" Slider "), Some(JointKind::Slider));
        assert_eq!(JointKind::from_key("weld"), None);
    }

    #[test]
    fn missing_fields_take_defaults_in_ron() {
        let j: Joint = ron::from_str("(kind: Rope, connected: Some(\"Post\"))").unwrap();
        assert_eq!(j.kind, JointKind::Rope);
        assert_eq!(j.axis, Vec3::Y);
        assert!(j.enabled);
        let back: Joint = ron::from_str(&ron::to_string(&j).unwrap()).unwrap();
        assert_eq!(back, j);
    }

    #[test]
    fn only_structural_changes_rebuild() {
        let a = Joint::hinge(Vec3::Y);
        let mut b = a.clone().motor(JointMotor::speed(1.0));
        assert!(!a.needs_rebuild(&b));
        b.anchor = Vec3::X;
        assert!(a.needs_rebuild(&b));
    }

    #[test]
    fn edits_change_only_what_they_set() {
        let mut joint = Joint::hinge(Vec3::Y).motor(JointMotor {
            speed: 1.0,
            stiffness: 99.0,
            ..JointMotor::default()
        });
        let edit = JointEdit {
            motor: Some(Some((0.0, Some(1.5)))),
            limits: Some(Some([2.0, -1.0])),
            ..JointEdit::default()
        };
        assert!(!edit.is_empty() && JointEdit::default().is_empty());
        edit.apply(&mut joint);
        let motor = joint.motor.unwrap();
        assert_eq!(motor.target, Some(1.5));
        assert!(
            (motor.stiffness - 99.0).abs() < f32::EPSILON,
            "kept its tuning"
        );
        assert_eq!(joint.limits, Some([-1.0, 2.0]));
        assert!(joint.enabled);
        JointEdit {
            enabled: Some(false),
            motor: Some(None),
            connected: Some(Some("Frame".into())),
            ..JointEdit::default()
        }
        .apply(&mut joint);
        assert!(!joint.enabled && joint.motor.is_none());
        assert_eq!(joint.connected.as_deref(), Some("Frame"));
    }
}
