//! Animation graphs: `.anim.ron` files.
//!
//! A graph decides which of a model's clips play, so scripts only set
//! *parameters* (a speed, a "grounded" flag, a "jump" trigger) and the
//! graph picks and blends the clips. It holds:
//!
//! - [`Parameter`]s: named floats, ints, bools and triggers (a trigger is
//!   a bool that resets itself when a transition uses it);
//! - [`GraphLayer`]s, each a state machine. The first layer animates the
//!   whole model; later ones override the bones in their *mask* (an
//!   upper-body layer waves while the legs keep walking), with a weight;
//! - in each layer, [`State`]s playing a [`Motion`]: one clip, a 1D blend
//!   space (Idle, Walk and Run by speed) or a 2D one (strafing by two
//!   parameters), or nothing;
//! - and [`Transition`]s between states: when their [`Condition`]s hold
//!   (and, optionally, once the state has played to its exit time), the
//!   layer crossfades to the next state. A transition from [`ANY_STATE`]
//!   can leave any state.
//!
//! [`AnimGraph::problems`] lists what's wrong with a graph (unknown states,
//! parameters or clips); [`load_anim_graph`] reads one through a cache,
//! and editors show unsaved versions with [`set_anim_graph_preview`].
//! `automata_scene`'s animator runs graphs.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The `from` of a transition that can leave any state of its layer.
pub const ANY_STATE: &str = "*";

/// The file extension of animation graphs (after the name: `x.anim.ron`).
pub const EXTENSION: &str = ".anim.ron";

/// Whether `path` names an animation graph (`*.anim.ron`).
#[must_use]
pub fn is_anim_graph_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(EXTENSION))
}

/// A parameter's type, with its starting value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ParamKind {
    /// A number (a speed, a direction).
    Float(f32),
    /// A whole number (a weapon slot).
    Int(i32),
    /// On or off (grounded, crouching).
    Bool(bool),
    /// A one-off signal (jump, wave): set by a script, cleared when a
    /// transition that tests it fires.
    Trigger,
}

impl ParamKind {
    /// The starting value as the runtime stores it (bools and triggers as
    /// 0 or 1).
    #[must_use]
    pub fn initial(self) -> f32 {
        match self {
            ParamKind::Float(v) => v,
            #[allow(clippy::cast_precision_loss)] // small ints
            ParamKind::Int(v) => v as f32,
            ParamKind::Bool(v) => f32::from(u8::from(v)),
            ParamKind::Trigger => 0.0,
        }
    }

    /// The type's name, for messages and the editor.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ParamKind::Float(_) => "float",
            ParamKind::Int(_) => "int",
            ParamKind::Bool(_) => "bool",
            ParamKind::Trigger => "trigger",
        }
    }
}

/// A named value scripts set and conditions test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// The name scripts use.
    pub name: String,
    /// Its type and starting value.
    pub kind: ParamKind,
}

impl Parameter {
    /// A float parameter starting at `value`.
    #[must_use]
    pub fn float(name: &str, value: f32) -> Self {
        Self {
            name: name.into(),
            kind: ParamKind::Float(value),
        }
    }

    /// A bool parameter starting at `value`.
    #[must_use]
    pub fn boolean(name: &str, value: bool) -> Self {
        Self {
            name: name.into(),
            kind: ParamKind::Bool(value),
        }
    }

    /// A trigger.
    #[must_use]
    pub fn trigger(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: ParamKind::Trigger,
        }
    }
}

/// A clip at a point along a 1D blend space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendPoint1D {
    /// The clip's name.
    pub clip: String,
    /// The parameter value where this clip plays alone.
    pub at: f32,
}

/// A clip at a point in a 2D blend space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendPoint2D {
    /// The clip's name.
    pub clip: String,
    /// The two parameter values where this clip plays alone.
    pub at: [f32; 2],
}

/// What a state plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Motion {
    /// Nothing: the rest pose on the first layer, and on later layers the
    /// layer lets the ones below show through (an "off" state).
    Empty,
    /// One clip, by name.
    Clip(String),
    /// Clips along one parameter (Idle at 0, Walk at 2, Run at 5): the two
    /// around the parameter's value blend, kept in step.
    Blend1D {
        /// The parameter.
        param: String,
        /// The clips and where they sit.
        points: Vec<BlendPoint1D>,
    },
    /// Clips over two parameters (forward and sideways speed): every
    /// clip weighs in by how close the point is (gradient band
    /// interpolation), kept in step.
    Blend2D {
        /// The horizontal parameter.
        x: String,
        /// The vertical parameter.
        y: String,
        /// The clips and where they sit.
        points: Vec<BlendPoint2D>,
    },
}

impl Motion {
    /// Every clip this motion uses.
    #[must_use]
    pub fn clips(&self) -> Vec<&str> {
        match self {
            Motion::Empty => Vec::new(),
            Motion::Clip(c) => vec![c.as_str()],
            Motion::Blend1D { points, .. } => points.iter().map(|p| p.clip.as_str()).collect(),
            Motion::Blend2D { points, .. } => points.iter().map(|p| p.clip.as_str()).collect(),
        }
    }

    /// The parameters this motion reads.
    #[must_use]
    pub fn params(&self) -> Vec<&str> {
        match self {
            Motion::Empty | Motion::Clip(_) => Vec::new(),
            Motion::Blend1D { param, .. } => vec![param.as_str()],
            Motion::Blend2D { x, y, .. } => vec![x.as_str(), y.as_str()],
        }
    }

    /// A short description for the editor ("Walking", "1D by speed").
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Motion::Empty => "empty".into(),
            Motion::Clip(c) if c.is_empty() => "no clip".into(),
            Motion::Clip(c) => c.clone(),
            Motion::Blend1D { param, .. } => format!("1D by {param}"),
            Motion::Blend2D { x, y, .. } => format!("2D by {x}, {y}"),
        }
    }
}

/// A state in a layer's state machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct State {
    /// The state's name (unique in its layer).
    pub name: String,
    /// What it plays.
    pub motion: Motion,
    /// Playback rate (1 is normal speed).
    #[serde(skip_serializing_if = "is_one")]
    pub speed: f32,
    /// A float parameter the rate is multiplied by (empty: none).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub speed_param: String,
    /// Whether the motion starts over when it ends. A state that doesn't
    /// loop holds its last frame (and fires the clip's `end` event).
    #[serde(skip_serializing_if = "is_true")]
    pub looping: bool,
    /// Where the editor draws the state's node.
    pub position: [f32; 2],
}

impl Default for State {
    fn default() -> Self {
        Self {
            name: String::new(),
            motion: Motion::Empty,
            speed: 1.0,
            speed_param: String::new(),
            looping: true,
            position: [0.0, 0.0],
        }
    }
}

impl State {
    /// A looping state named `name` playing `clip`.
    #[must_use]
    pub fn clip(name: &str, clip: &str) -> Self {
        Self {
            name: name.into(),
            motion: Motion::Clip(clip.into()),
            ..Self::default()
        }
    }
}

/// A test on a parameter. A transition fires when all of its hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// The parameter is greater than the value.
    Above(String, f32),
    /// The parameter is less than the value.
    Below(String, f32),
    /// The parameter equals the value (for ints).
    Equals(String, f32),
    /// The parameter doesn't equal the value (for ints).
    NotEquals(String, f32),
    /// The bool is on.
    IsTrue(String),
    /// The bool is off.
    IsFalse(String),
    /// The trigger was set (and is cleared when the transition fires).
    Triggered(String),
}

impl Condition {
    /// The parameter tested.
    #[must_use]
    pub fn param(&self) -> &str {
        match self {
            Condition::Above(p, _)
            | Condition::Below(p, _)
            | Condition::Equals(p, _)
            | Condition::NotEquals(p, _)
            | Condition::IsTrue(p)
            | Condition::IsFalse(p)
            | Condition::Triggered(p) => p,
        }
    }

    /// The parameter tested, to rename it.
    pub fn param_mut(&mut self) -> &mut String {
        match self {
            Condition::Above(p, _)
            | Condition::Below(p, _)
            | Condition::Equals(p, _)
            | Condition::NotEquals(p, _)
            | Condition::IsTrue(p)
            | Condition::IsFalse(p)
            | Condition::Triggered(p) => p,
        }
    }

    /// Whether it holds for the parameter's current `value` (bools and
    /// triggers are 0 or 1).
    #[must_use]
    #[allow(clippy::float_cmp)] // ints and flags stored exactly
    pub fn holds(&self, value: f32) -> bool {
        match self {
            Condition::Above(_, v) => value > *v,
            Condition::Below(_, v) => value < *v,
            Condition::Equals(_, v) => value.round() == v.round(),
            Condition::NotEquals(_, v) => value.round() != v.round(),
            Condition::IsTrue(_) | Condition::Triggered(_) => value >= 0.5,
            Condition::IsFalse(_) => value < 0.5,
        }
    }

    /// Reads like the editor shows it: `speed > 0.1`, `jump`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Condition::Above(p, v) => format!("{p} > {}", short(*v)),
            Condition::Below(p, v) => format!("{p} < {}", short(*v)),
            Condition::Equals(p, v) => format!("{p} = {}", short(*v)),
            Condition::NotEquals(p, v) => format!("{p} ≠ {}", short(*v)),
            Condition::IsTrue(p) => p.clone(),
            Condition::IsFalse(p) => format!("not {p}"),
            Condition::Triggered(p) => format!("{p}!"),
        }
    }
}

/// A number without needless decimals.
fn short(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.into()
    }
}

/// A way from one state to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transition {
    /// The state it leaves, or [`ANY_STATE`].
    pub from: String,
    /// The state it goes to.
    pub to: String,
    /// What must hold (all of it). With no conditions and no exit time it
    /// fires when the state's motion ends.
    pub when: Vec<Condition>,
    /// Seconds to crossfade.
    pub fade: f32,
    /// How far through its motion (1 = once through) the state must have
    /// played first, or `None` to go as soon as the conditions hold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_time: Option<f32>,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            from: String::new(),
            to: String::new(),
            when: Vec::new(),
            fade: 0.2,
            exit_time: None,
        }
    }
}

impl Transition {
    /// A transition from `from` to `to` when `when` holds.
    #[must_use]
    pub fn new(from: &str, to: &str, when: Vec<Condition>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            when,
            ..Self::default()
        }
    }

    /// When it can fire: the exit time it effectively waits for (`None`:
    /// only the conditions count).
    #[must_use]
    pub fn effective_exit_time(&self) -> Option<f32> {
        match (self.exit_time, self.when.is_empty()) {
            (Some(t), _) => Some(t.max(0.0)),
            (None, true) => Some(1.0),
            (None, false) => None,
        }
    }
}

/// One state machine, animating the whole model (the first layer) or
/// the bones in its mask over the layers below.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphLayer {
    /// The layer's name (scripts set its weight by it).
    pub name: String,
    /// How much it shows over the layers below, 0 to 1 (the first layer
    /// always shows fully).
    #[serde(skip_serializing_if = "is_one")]
    pub weight: f32,
    /// The bones it animates, each with everything below it (empty: the
    /// whole model).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mask: Vec<String>,
    /// The state it starts in (empty: the first one).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub entry: String,
    /// Its states.
    pub states: Vec<State>,
    /// The ways between them.
    pub transitions: Vec<Transition>,
}

impl Default for GraphLayer {
    fn default() -> Self {
        Self {
            name: String::new(),
            weight: 1.0,
            mask: Vec::new(),
            entry: String::new(),
            states: Vec::new(),
            transitions: Vec::new(),
        }
    }
}

impl GraphLayer {
    /// The state named `name`'s index.
    #[must_use]
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    /// The index of the state the layer starts in.
    #[must_use]
    pub fn entry_index(&self) -> Option<usize> {
        if self.states.is_empty() {
            return None;
        }
        Some(self.state_index(&self.entry).unwrap_or(0))
    }
}

/// An animation graph's contents (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimGraph {
    /// What scripts set.
    pub parameters: Vec<Parameter>,
    /// The state machines, bottom first.
    pub layers: Vec<GraphLayer>,
}

const HEADER: &str = "// An animation graph: states play clips, transitions pick the next\n\
// one. Scripts set the parameters (entity.animation.set / trigger);\n\
// an Animation component uses it with graph: \"PATH\".\n";

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)] // serde's signature
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_true(v: &bool) -> bool {
    *v
}

fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
}

impl AnimGraph {
    /// Reads a graph file's text.
    ///
    /// # Errors
    /// The text isn't an animation graph (the message says where and why).
    pub fn parse(text: &str) -> Result<Self, String> {
        ron_options().from_str(text).map_err(|e| e.to_string())
    }

    /// The file's text, with a header naming the file (`name`, as scenes
    /// refer to it).
    #[must_use]
    pub fn to_ron(&self, name: &str) -> String {
        let config = ron::ser::PrettyConfig::new()
            .indentor("    ".into())
            .separate_tuple_members(false)
            .extensions(ron::extensions::Extensions::IMPLICIT_SOME);
        let body = ron_options()
            .to_string_pretty(self, config)
            .unwrap_or_default();
        format!("{}{body}\n", HEADER.replace("PATH", name))
    }

    /// Writes the file (creating its folder). `name` is how scenes refer
    /// to it (for the header).
    ///
    /// # Errors
    /// The file can't be written.
    pub fn save(&self, path: &Path, name: &str) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_ron(name))
    }

    /// A starter graph for a model with `clips`: one layer with a state per
    /// clip (up to eight, in a grid), the first one the entry, and a
    /// `speed` parameter. With an idle- and a walk-like clip it starts as
    /// Idle ⇄ Walk on `speed`.
    #[must_use]
    pub fn starter(clips: &[&str]) -> Self {
        let mut layer = GraphLayer {
            name: "Base".into(),
            ..GraphLayer::default()
        };
        for (i, clip) in clips.iter().take(8).enumerate() {
            #[allow(clippy::cast_precision_loss)] // i < 8
            let (col, row) = ((i % 4) as f32, (i / 4) as f32);
            layer.states.push(State {
                position: [col * 200.0, row * 120.0],
                ..State::clip(clip, clip)
            });
        }
        let find = |words: &[&str]| {
            clips.iter().find(|c| {
                let c = c.to_ascii_lowercase();
                words.iter().any(|w| c.contains(w))
            })
        };
        let idle = find(&["idle", "stand"]);
        let walk = find(&["walk", "run"]);
        if let (Some(idle), Some(walk)) = (idle, walk) {
            layer.entry = (*idle).to_string();
            layer.transitions.push(Transition::new(
                idle,
                walk,
                vec![Condition::Above("speed".into(), 0.1)],
            ));
            layer.transitions.push(Transition::new(
                walk,
                idle,
                vec![Condition::Below("speed".into(), 0.1)],
            ));
        }
        Self {
            parameters: vec![Parameter::float("speed", 0.0)],
            layers: vec![layer],
        }
    }

    /// The parameter named `name`.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }

    /// The layer named `name`'s index.
    #[must_use]
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }

    /// Everything wrong with the graph, as messages for people: duplicate
    /// or unknown names, conditions that don't suit their parameter, and,
    /// given the model's `clips` and `bones`, clips and mask bones it
    /// doesn't have. Empty means it's fine.
    #[must_use]
    pub fn problems(&self, clips: Option<&[&str]>, bones: Option<&[&str]>) -> Vec<String> {
        let mut out = Vec::new();
        let mut names = HashSet::new();
        for p in &self.parameters {
            if p.name.trim().is_empty() {
                out.push("a parameter has no name".into());
            } else if !names.insert(p.name.as_str()) {
                out.push(format!("two parameters are called \"{}\"", p.name));
            }
        }
        if self.layers.is_empty() {
            out.push("the graph has no layers".into());
        }
        let mut layer_names = HashSet::new();
        for (i, layer) in self.layers.iter().enumerate() {
            let at = if layer.name.is_empty() {
                format!("layer {}", i + 1)
            } else {
                if !layer_names.insert(layer.name.as_str()) {
                    out.push(format!("two layers are called \"{}\"", layer.name));
                }
                format!("layer \"{}\"", layer.name)
            };
            self.layer_problems(layer, &at, clips, bones, &mut out);
        }
        out
    }

    fn layer_problems(
        &self,
        layer: &GraphLayer,
        at: &str,
        clips: Option<&[&str]>,
        bones: Option<&[&str]>,
        out: &mut Vec<String>,
    ) {
        if layer.states.is_empty() {
            out.push(format!("{at} has no states"));
        }
        if !layer.entry.is_empty() && layer.state_index(&layer.entry).is_none() {
            out.push(format!(
                "{at} starts in \"{}\", which isn't a state",
                layer.entry
            ));
        }
        if let Some(bones) = bones {
            for bone in &layer.mask {
                if !bones.contains(&bone.as_str()) {
                    out.push(format!(
                        "{at}'s mask names \"{bone}\", which the model doesn't have"
                    ));
                }
            }
        }
        let mut names = HashSet::new();
        for state in &layer.states {
            if state.name.trim().is_empty() {
                out.push(format!("{at} has a state with no name"));
            } else if state.name == ANY_STATE {
                out.push(format!("{at}: \"{ANY_STATE}\" can't name a state"));
            } else if !names.insert(state.name.as_str()) {
                out.push(format!("{at} has two states called \"{}\"", state.name));
            }
            self.state_problems(state, at, clips, out);
        }
        for t in &layer.transitions {
            let what = format!("{at}: the transition {} → {}", t.from, t.to);
            if t.from != ANY_STATE && layer.state_index(&t.from).is_none() {
                out.push(format!("{what} leaves \"{}\", which isn't a state", t.from));
            }
            if layer.state_index(&t.to).is_none() {
                out.push(format!("{what} goes to \"{}\", which isn't a state", t.to));
            }
            if t.from == ANY_STATE && t.when.is_empty() {
                out.push(format!(
                    "{what} has no conditions, so it would fire all the time"
                ));
            }
            for c in &t.when {
                self.condition_problem(c, &what, out);
            }
        }
    }

    fn state_problems(
        &self,
        state: &State,
        at: &str,
        clips: Option<&[&str]>,
        out: &mut Vec<String>,
    ) {
        let what = format!("{at}, state \"{}\"", state.name);
        for param in state.motion.params() {
            match self.parameter(param).map(|p| p.kind) {
                None => out.push(format!(
                    "{what} blends by \"{param}\", which isn't a parameter"
                )),
                Some(ParamKind::Float(_) | ParamKind::Int(_)) => {}
                Some(kind) => out.push(format!(
                    "{what} blends by \"{param}\", a {}; blend spaces need numbers",
                    kind.label()
                )),
            }
        }
        if !state.speed_param.is_empty()
            && !matches!(
                self.parameter(&state.speed_param).map(|p| p.kind),
                Some(ParamKind::Float(_) | ParamKind::Int(_))
            )
        {
            out.push(format!(
                "{what}'s speed follows \"{}\", which isn't a number parameter",
                state.speed_param
            ));
        }
        match &state.motion {
            Motion::Clip(c) if c.is_empty() => out.push(format!("{what} has no clip")),
            Motion::Blend1D { .. } | Motion::Blend2D { .. } if state.motion.clips().is_empty() => {
                out.push(format!("{what}'s blend space has no clips"));
            }
            Motion::Blend2D { points, .. } => {
                let mut seen = Vec::new();
                for p in points {
                    if seen.contains(&p.at) {
                        out.push(format!("{what} has two clips at {:?}", p.at));
                    }
                    seen.push(p.at);
                }
            }
            _ => {}
        }
        if let Some(clips) = clips {
            for clip in state.motion.clips() {
                if !clip.is_empty() && !clips.contains(&clip) {
                    out.push(format!(
                        "{what} plays \"{clip}\", which the model doesn't have"
                    ));
                }
            }
        }
    }

    fn condition_problem(&self, c: &Condition, what: &str, out: &mut Vec<String>) {
        let Some(param) = self.parameter(c.param()) else {
            out.push(format!(
                "{what} tests \"{}\", which isn't a parameter",
                c.param()
            ));
            return;
        };
        let fits = match (c, param.kind) {
            (Condition::Triggered(_), kind) => kind == ParamKind::Trigger,
            (Condition::IsTrue(_) | Condition::IsFalse(_), kind) => {
                matches!(kind, ParamKind::Bool(_))
            }
            (_, kind) => matches!(kind, ParamKind::Float(_) | ParamKind::Int(_)),
        };
        if !fits {
            out.push(format!(
                "{what} tests \"{}\" ({}) with {}",
                param.name,
                param.kind.label(),
                c.describe()
            ));
        }
    }

    /// Renames parameter `from` to `to` everywhere it's used.
    pub fn rename_parameter(&mut self, from: &str, to: &str) {
        for p in &mut self.parameters {
            if p.name == from {
                p.name = to.into();
            }
        }
        for layer in &mut self.layers {
            for state in &mut layer.states {
                if state.speed_param == from {
                    state.speed_param = to.into();
                }
                match &mut state.motion {
                    Motion::Blend1D { param, .. } if param == from => *param = to.into(),
                    Motion::Blend2D { x, y, .. } => {
                        if x == from {
                            *x = to.into();
                        }
                        if y == from {
                            *y = to.into();
                        }
                    }
                    _ => {}
                }
            }
            for t in &mut layer.transitions {
                for c in &mut t.when {
                    if c.param() == from {
                        *c.param_mut() = to.into();
                    }
                }
            }
        }
    }

    /// Renames state `from` to `to` in layer `layer` (transitions and the
    /// entry follow).
    pub fn rename_state(&mut self, layer: usize, from: &str, to: &str) {
        let Some(l) = self.layers.get_mut(layer) else {
            return;
        };
        for s in &mut l.states {
            if s.name == from {
                s.name = to.into();
            }
        }
        if l.entry == from {
            l.entry = to.into();
        }
        for t in &mut l.transitions {
            if t.from == from {
                t.from = to.into();
            }
            if t.to == from {
                t.to = to.into();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Loading, through a cache.

/// Why a graph didn't load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnimGraphError {
    /// No such file.
    NotFound(String),
    /// The file isn't an animation graph.
    Invalid {
        /// The path as given.
        path: String,
        /// What's wrong.
        message: String,
    },
}

impl std::fmt::Display for AnimGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnimGraphError::NotFound(p) => write!(f, "animation graph not found: {p}"),
            AnimGraphError::Invalid { path, message } => {
                write!(f, "{path} isn't an animation graph: {message}")
            }
        }
    }
}

impl std::error::Error for AnimGraphError {}

type GraphEntry = (Option<SystemTime>, Result<Arc<AnimGraph>, String>);

static GRAPHS: Mutex<BTreeMap<PathBuf, GraphEntry>> = Mutex::new(BTreeMap::new());
static PREVIEWS: Mutex<BTreeMap<PathBuf, Arc<AnimGraph>>> = Mutex::new(BTreeMap::new());

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Loads a graph through the shared cache: relative paths resolve like
/// model paths (against the asset root, then the working directory). An
/// editor's unsaved version ([`set_anim_graph_preview`]) wins over the
/// file; a file that changed on disk is read again.
///
/// # Errors
/// The file is missing or isn't an animation graph.
pub fn load_anim_graph(path: &str) -> Result<Arc<AnimGraph>, AnimGraphError> {
    let key = canonical(&crate::resolve(path));
    if let Some(graph) = PREVIEWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return Ok(graph.clone());
    }
    let stamp = crate::modified(&key);
    let invalid = |message: String| AnimGraphError::Invalid {
        path: path.to_string(),
        message,
    };
    if stamp.is_none() {
        return Err(AnimGraphError::NotFound(path.to_string()));
    }
    if let Some((t, result)) = GRAPHS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        if *t == stamp {
            return result.clone().map_err(invalid);
        }
    }
    let result = std::fs::read_to_string(&key)
        .map_err(|e| e.to_string())
        .and_then(|text| AnimGraph::parse(&text))
        .map(Arc::new);
    GRAPHS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, (stamp, result.clone()));
    result.map_err(invalid)
}

/// Shows `graph` instead of what's on disk for the graph at `path` (an
/// editor's unsaved changes), or goes back to the disk with `None`.
/// Returns whether anything changed.
pub fn set_anim_graph_preview(path: &Path, graph: Option<&AnimGraph>) -> bool {
    let key = canonical(path);
    let mut previews = PREVIEWS.lock().unwrap_or_else(PoisonError::into_inner);
    match graph {
        None => previews.remove(&key).is_some(),
        Some(graph) => {
            if previews.get(&key).is_some_and(|g| **g == *graph) {
                return false;
            }
            let on_disk = std::fs::read_to_string(&key)
                .ok()
                .and_then(|t| AnimGraph::parse(&t).ok());
            if on_disk.as_ref() == Some(graph) {
                return previews.remove(&key).is_some();
            }
            previews.insert(key, Arc::new(graph.clone()));
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn robot() -> AnimGraph {
        AnimGraph {
            parameters: vec![
                Parameter::float("speed", 0.0),
                Parameter::boolean("grounded", true),
                Parameter::trigger("jump"),
            ],
            layers: vec![GraphLayer {
                name: "Base".into(),
                states: vec![
                    State {
                        name: "Move".into(),
                        motion: Motion::Blend1D {
                            param: "speed".into(),
                            points: vec![
                                BlendPoint1D {
                                    clip: "Idle".into(),
                                    at: 0.0,
                                },
                                BlendPoint1D {
                                    clip: "Walking".into(),
                                    at: 2.0,
                                },
                            ],
                        },
                        ..State::default()
                    },
                    State {
                        looping: false,
                        ..State::clip("Jump", "Jump")
                    },
                ],
                transitions: vec![
                    Transition::new(ANY_STATE, "Jump", vec![Condition::Triggered("jump".into())]),
                    Transition {
                        exit_time: Some(0.9),
                        ..Transition::new(
                            "Jump",
                            "Move",
                            vec![Condition::IsTrue("grounded".into())],
                        )
                    },
                ],
                ..GraphLayer::default()
            }],
        }
    }

    #[test]
    fn round_trips_through_ron() {
        let g = robot();
        let text = g.to_ron("anims/robot.anim.ron");
        assert!(text.contains("exit_time: 0.9"), "{text}");
        assert!(text.contains("Triggered(\"jump\")"), "{text}");
        assert_eq!(AnimGraph::parse(&text).unwrap(), g);
        // Hand-written files may leave most things out.
        let small =
            AnimGraph::parse(r#"(layers: [(states: [(name: "Idle", motion: Clip("Idle"))])])"#)
                .unwrap();
        assert_eq!(small.layers[0].states[0].speed, 1.0);
        assert!(small.layers[0].states[0].looping);
        assert!(AnimGraph::parse("(nonsense: 1)").is_err());
    }

    #[test]
    fn finds_problems() {
        let g = robot();
        assert!(
            g.problems(None, None).is_empty(),
            "{:?}",
            g.problems(None, None)
        );
        let clips = ["Idle", "Jump"];
        let p = g.problems(Some(&clips), None);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("Walking"));

        let mut bad = robot();
        bad.layers[0].transitions.push(Transition::new(
            "Move",
            "Nowhere",
            vec![
                Condition::IsTrue("speed".into()),
                Condition::Above("missing".into(), 1.0),
            ],
        ));
        bad.layers[0].mask = vec!["Spine".into()];
        let p = bad.problems(None, Some(&["Hips"]));
        assert!(p.iter().any(|m| m.contains("Nowhere")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("\"speed\" (float)")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("missing")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("Spine")), "{p:?}");
    }

    #[test]
    fn renames_follow_through() {
        let mut g = robot();
        g.rename_parameter("speed", "pace");
        g.rename_state(0, "Jump", "Hop");
        assert!(
            g.problems(None, None).is_empty(),
            "{:?}",
            g.problems(None, None)
        );
        assert_eq!(g.layers[0].transitions[0].to, "Hop");
        assert_eq!(g.layers[0].states[0].motion.params(), vec!["pace"]);
    }

    #[test]
    fn conditions_and_exit_times() {
        assert!(Condition::Above("s".into(), 0.1).holds(0.2));
        assert!(!Condition::Below("s".into(), 0.1).holds(0.2));
        assert!(Condition::Equals("n".into(), 2.0).holds(2.0));
        assert!(Condition::IsFalse("b".into()).holds(0.0));
        assert_eq!(
            Condition::Above("speed".into(), 0.1).describe(),
            "speed > 0.1"
        );
        assert_eq!(
            Transition::new("a", "b", vec![]).effective_exit_time(),
            Some(1.0)
        );
        let t = Transition::new("a", "b", vec![Condition::IsTrue("x".into())]);
        assert_eq!(t.effective_exit_time(), None);
    }

    #[test]
    fn the_manuals_example_is_a_good_graph() {
        let page = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/wiki/oberon/animation-graphs.md");
        let page = std::fs::read_to_string(page).unwrap();
        let start = page.find("```ron\n(\n").unwrap() + "```ron\n".len();
        let end = start + page[start..].find("```").unwrap();
        let graph = AnimGraph::parse(&page[start..end]).unwrap();
        let clips = ["Idle", "Walking", "Running", "Jump", "Wave"];
        let bones = ["Hips", "Torso"];
        assert_eq!(
            graph.problems(Some(&clips), Some(&bones)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn starter_links_idle_and_walk() {
        let g = AnimGraph::starter(&["Dance", "Idle", "Walking"]);
        let clips = ["Dance", "Idle", "Walking"];
        assert_eq!(g.problems(Some(&clips), None), Vec::<String>::new());
        assert_eq!(g.layers[0].entry, "Idle");
        assert_eq!(g.layers[0].transitions.len(), 2);
        assert!(is_anim_graph_path(Path::new("a/robot.ANIM.ron")));
        assert!(!is_anim_graph_path(Path::new("a/robot.mat.ron")));
    }
}
