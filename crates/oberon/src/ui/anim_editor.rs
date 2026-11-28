//! The animation graph editor: a `.anim.ron` file opens in a tab with a
//! node canvas for one layer's states and transitions (`anim_canvas`),
//! panels for the parameters, the layer and the selection, and 1D / 2D
//! blend space editors (`anim_panels`). During Play, the live state of an
//! entity using the graph lights up.
//!
//! Like the material editor, the tab's text stays the truth: every edit
//! rewrites it (one undo step), and Edit as Text shows it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use automata_assets::anim_graph::{
    AnimGraph, BlendPoint1D, BlendPoint2D, Condition, GraphLayer, Motion, ParamKind, Parameter,
    State, Transition, ANIMS_DIR, ANY_STATE,
};
use automata_assets::Rig;

use super::{LogLevel, Ui};
use crate::code::{Key, Mods};

/// What's selected on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimSel {
    /// Nothing: the panel shows how graphs work.
    #[default]
    None,
    /// The Any State node.
    Any,
    /// A state, by index in the layer.
    State(usize),
    /// A transition, by index in the layer.
    Transition(usize),
}

/// A node on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimNode {
    /// Any State: its transitions can leave whatever state the layer is in.
    Any,
    /// A state, by index in the layer.
    State(usize),
}

/// A pointer drag on the canvas or a blend space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum AnimDrag {
    /// A state being moved: the pointer's offset from its corner and its
    /// live position (world units), written to the file on release.
    Move {
        state: usize,
        grab: [f32; 2],
        at: [f32; 2],
        moved: bool,
    },
    /// The view being panned.
    Pan { last: (f32, f32) },
    /// A blend space point being dragged, with its live position.
    Point { index: usize, at: [f32; 2] },
}

/// What the selected entity's graph is doing, during Play.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimLive {
    /// The entity shown.
    pub entity: String,
    /// Its graph's playback.
    pub state: automata_scene::GraphState,
}

/// A text field of the graph editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimProp {
    /// A parameter's name.
    ParamName(usize),
    /// A parameter's starting value.
    ParamValue(usize),
    /// The layer's name.
    LayerName,
    /// The layer's weight.
    LayerWeight,
    /// The layer's mask (bone names, comma-separated).
    LayerMask,
    /// The selected state's name.
    StateName,
    /// The selected state's speed.
    StateSpeed,
    /// The selected state's clip.
    Clip,
    /// A blend point's clip.
    PointClip(usize),
    /// A blend point's position (axis 0 or 1).
    PointAt(usize, u8),
    /// The selected transition's fade.
    Fade,
    /// The selected transition's exit time.
    ExitTime,
    /// A condition's value.
    CondValue(usize),
}

/// A graph editor command (from its menus and buttons).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimAction {
    /// Add a parameter of a kind (0 float, 1 int, 2 bool, 3 trigger).
    AddParam(u8),
    /// Remove a parameter (and the conditions testing it).
    DeleteParam(usize),
    /// Show another layer.
    Layer(usize),
    /// Add a layer on top.
    AddLayer,
    /// Remove the shown layer.
    DeleteLayer,
    /// Add a state at a canvas point (world units) playing a motion kind
    /// (0 empty, 1 clip, 2 1D blend, 3 2D blend).
    AddState {
        /// Where, across.
        x: f32,
        /// Where, down.
        y: f32,
        /// The motion kind.
        kind: u8,
    },
    /// Remove a state and its transitions.
    DeleteState(usize),
    /// Make a state the layer's entry state.
    SetEntry(usize),
    /// Start drawing a transition from a node: the next node clicked ends it.
    Link(AnimNode),
    /// Remove a transition.
    DeleteTransition(usize),
    /// Change the selected state's motion kind.
    MotionKind(u8),
    /// Play the model's clip `i` in the selected state.
    Clip(usize),
    /// Put the model's clip (second) on blend point (first).
    PointClip(usize, usize),
    /// Add a blend point.
    AddPoint,
    /// Remove a blend point.
    RemovePoint(usize),
    /// Set a blend axis (0: the 1D parameter or 2D x, 1: 2D y) to
    /// parameter `i`.
    BlendParam(u8, usize),
    /// Scale the selected state's speed by a parameter (`None`: don't).
    SpeedParam(Option<usize>),
    /// Add a condition to the selected transition.
    AddCondition,
    /// Remove a condition.
    RemoveCondition(usize),
    /// Test parameter (second) in condition (first).
    CondParam(usize, usize),
    /// Change condition (first)'s test (see [`cond_op`]).
    CondOp(usize, u8),
    /// Add the model's bone `i` to the layer's mask.
    AddMaskBone(usize),
    /// Fit every node in view.
    FrameAll,
}

/// The graph editor's state (one editor, following the active tab).
#[derive(Default)]
pub struct AnimEditor {
    /// The last text parsed and what it gave.
    pub(super) parsed: Option<(String, Result<AnimGraph, String>)>,
    /// Graph files shown as text instead.
    source: HashSet<PathBuf>,
    /// The layer shown.
    pub(super) layer: usize,
    /// What's selected.
    pub(super) sel: AnimSel,
    /// The view: world point at the canvas centre (negated) and zoom.
    pub(super) pan: [f32; 2],
    pub(super) zoom: f32,
    /// The file the view was last fitted to.
    framed: Option<PathBuf>,
    /// Fit the view again once the canvas has been measured.
    pub(super) refit: bool,
    pub(super) drag: Option<AnimDrag>,
    /// A transition being drawn from this node.
    pub(super) linking: Option<AnimNode>,
    pub(super) left_scroll: f32,
    pub(super) right_scroll: f32,
    /// The model whose clips and bones the editor offers: the entity it
    /// came from and its rig.
    pub(super) model: Option<(String, Arc<Rig>)>,
    model_for: Option<(PathBuf, Instant)>,
    /// The live playback shown during Play.
    pub(super) live: Option<AnimLive>,
    /// Where each node was drawn last frame.
    pub(super) nodes: Vec<(AnimNode, super::Rect)>,
    /// Where the canvas was drawn last frame.
    pub(super) canvas_rect: Option<super::Rect>,
}

/// Node size on the canvas (world units).
pub(super) const NODE_W: f32 = 150.0;
/// Node height on the canvas (world units).
pub(super) const NODE_H: f32 = 46.0;

// ---------------------------------------------------------------------------
// Graph edits, free of the interface (and tested on their own).

/// `base`, or `base 2`, `base 3`… whichever `taken` doesn't claim.
pub(super) fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..u32::MAX)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

/// The names of the motion kinds, by index.
pub(super) const MOTION_KINDS: [&str; 4] = ["Empty", "Clip", "1D Blend", "2D Blend"];

/// The index of a motion's kind.
pub(super) fn motion_kind(m: &Motion) -> u8 {
    match m {
        Motion::Empty => 0,
        Motion::Clip(_) => 1,
        Motion::Blend1D { .. } => 2,
        Motion::Blend2D { .. } => 3,
    }
}

/// `m` turned into motion kind `kind`, keeping its clips where it can.
/// New blend spaces read the first float parameters.
pub(super) fn with_motion_kind(m: &Motion, kind: u8, params: &[Parameter]) -> Motion {
    let clips: Vec<String> = m
        .clips()
        .into_iter()
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    let numeric: Vec<&str> = params
        .iter()
        .filter(|p| matches!(p.kind, ParamKind::Float(_) | ParamKind::Int(_)))
        .map(|p| p.name.as_str())
        .collect();
    let axis = |i: usize| {
        numeric
            .get(i)
            .or(numeric.first())
            .map_or("", |s| s)
            .to_string()
    };
    match kind {
        0 => Motion::Empty,
        1 => Motion::Clip(clips.first().cloned().unwrap_or_default()),
        2 => {
            let clips = if clips.is_empty() {
                vec![String::new()]
            } else {
                clips
            };
            let points = clips
                .into_iter()
                .zip(0u8..)
                .map(|(clip, i)| BlendPoint1D {
                    clip,
                    at: f32::from(i),
                })
                .collect();
            Motion::Blend1D {
                param: axis(0),
                points,
            }
        }
        _ => {
            const SPOTS: [[f32; 2]; 5] =
                [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [-1.0, 0.0], [0.0, -1.0]];
            let clips = if clips.is_empty() {
                vec![String::new()]
            } else {
                clips
            };
            let points = clips
                .into_iter()
                .enumerate()
                .map(|(i, clip)| BlendPoint2D {
                    clip,
                    at: SPOTS[i % SPOTS.len()],
                })
                .collect();
            Motion::Blend2D {
                x: axis(1),
                y: axis(0),
                points,
            }
        }
    }
}

/// Adds a state at `at` playing motion kind `kind` (the first clip for a
/// clip) to layer `layer`. Returns its index.
pub(super) fn add_state(
    g: &mut AnimGraph,
    layer: usize,
    at: [f32; 2],
    kind: u8,
    first_clip: Option<&str>,
) -> Option<usize> {
    let seed = Motion::Clip(first_clip.unwrap_or_default().to_string());
    let motion = with_motion_kind(&seed, kind, &g.parameters);
    let l = g.layers.get_mut(layer)?;
    let base = match kind {
        0 => "Off",
        1 => first_clip.filter(|c| !c.is_empty()).unwrap_or("State"),
        2 => "Blend",
        _ => "Blend 2D",
    };
    let name = unique(base, |n| l.state_index(n).is_some());
    l.states.push(State {
        name,
        motion,
        position: [at[0].round(), at[1].round()],
        ..State::default()
    });
    Some(l.states.len() - 1)
}

/// Removes state `i` of `layer`, its transitions, and (if it was the
/// entry) the entry.
pub(super) fn delete_state(g: &mut AnimGraph, layer: usize, i: usize) {
    let Some(l) = g.layers.get_mut(layer) else {
        return;
    };
    if i >= l.states.len() {
        return;
    }
    let name = l.states.remove(i).name;
    l.transitions.retain(|t| t.from != name && t.to != name);
    if l.entry == name {
        l.entry.clear();
    }
}

/// Adds a transition `from` → `to` (or finds the one there is). Returns
/// its index.
pub(super) fn add_transition(
    g: &mut AnimGraph,
    layer: usize,
    from: &str,
    to: &str,
) -> Option<usize> {
    let l = g.layers.get_mut(layer)?;
    if l.state_index(to).is_none() || (from != ANY_STATE && l.state_index(from).is_none()) {
        return None;
    }
    if let Some(i) = l
        .transitions
        .iter()
        .position(|t| t.from == from && t.to == to)
    {
        return Some(i);
    }
    l.transitions.push(Transition::new(from, to, Vec::new()));
    Some(l.transitions.len() - 1)
}

/// Adds a parameter of kind `kind` (0 float, 1 int, 2 bool, 3 trigger)
/// with a fresh name. Returns its index.
pub(super) fn add_parameter(g: &mut AnimGraph, kind: u8) -> usize {
    let (base, kind) = match kind {
        0 => ("speed", ParamKind::Float(0.0)),
        1 => ("slot", ParamKind::Int(0)),
        2 => ("grounded", ParamKind::Bool(false)),
        _ => ("jump", ParamKind::Trigger),
    };
    let name = unique(base, |n| g.parameter(n).is_some());
    let name = name.replace(' ', "_");
    g.parameters.push(Parameter { name, kind });
    g.parameters.len() - 1
}

/// Removes parameter `i` and the conditions and speed links using it.
pub(super) fn delete_parameter(g: &mut AnimGraph, i: usize) {
    if i >= g.parameters.len() {
        return;
    }
    let name = g.parameters.remove(i).name;
    for l in &mut g.layers {
        for s in &mut l.states {
            if s.speed_param == name {
                s.speed_param.clear();
            }
        }
        for t in &mut l.transitions {
            t.when.retain(|c| c.param() != name);
        }
    }
}

/// The tests a condition can make, by index: their labels.
pub(super) const COND_OPS: [&str; 7] = [">", "<", "=", "≠", "is true", "is false", "fired"];

/// A condition's test, as an index into [`COND_OPS`].
pub(super) fn cond_op(c: &Condition) -> u8 {
    match c {
        Condition::Above(..) => 0,
        Condition::Below(..) => 1,
        Condition::Equals(..) => 2,
        Condition::NotEquals(..) => 3,
        Condition::IsTrue(_) => 4,
        Condition::IsFalse(_) => 5,
        Condition::Triggered(_) => 6,
    }
}

/// A condition's value, for the tests that have one.
pub(super) fn cond_value(c: &Condition) -> Option<f32> {
    match c {
        Condition::Above(_, v)
        | Condition::Below(_, v)
        | Condition::Equals(_, v)
        | Condition::NotEquals(_, v) => Some(*v),
        _ => None,
    }
}

/// Test `op` (see [`COND_OPS`]) on `param` against `value`.
pub(super) fn make_condition(op: u8, param: String, value: f32) -> Condition {
    match op {
        0 => Condition::Above(param, value),
        1 => Condition::Below(param, value),
        2 => Condition::Equals(param, value),
        3 => Condition::NotEquals(param, value),
        4 => Condition::IsTrue(param),
        5 => Condition::IsFalse(param),
        _ => Condition::Triggered(param),
    }
}

/// The tests that suit a parameter of kind `kind`.
pub(super) fn ops_for(kind: ParamKind) -> &'static [u8] {
    match kind {
        ParamKind::Float(_) => &[0, 1],
        ParamKind::Int(_) => &[0, 1, 2, 3],
        ParamKind::Bool(_) => &[4, 5],
        ParamKind::Trigger => &[6],
    }
}

/// A condition on `p` that suits it (keeping `old`'s test and value when
/// they still do).
pub(super) fn condition_for(p: &Parameter, old: Option<&Condition>) -> Condition {
    let ops = ops_for(p.kind);
    let op = old
        .map(cond_op)
        .filter(|op| ops.contains(op))
        .unwrap_or(ops[0]);
    let value = old.and_then(cond_value).unwrap_or(match p.kind {
        ParamKind::Float(_) => 0.1,
        _ => 0.0,
    });
    make_condition(op, p.name.clone(), value)
}

/// The layer's states' bounding box (world units), Any State included.
pub(super) fn layer_bounds(l: &GraphLayer) -> ([f32; 2], [f32; 2]) {
    let any = any_position(l);
    let mut lo = any;
    let mut hi = [any[0] + NODE_W, any[1] + NODE_H];
    for s in &l.states {
        lo = [lo[0].min(s.position[0]), lo[1].min(s.position[1])];
        hi = [
            hi[0].max(s.position[0] + NODE_W),
            hi[1].max(s.position[1] + NODE_H),
        ];
    }
    (lo, hi)
}

/// Where the Any State node sits: above and left of the states, so its
/// arrows come down at them rather than through them.
pub(super) fn any_position(l: &GraphLayer) -> [f32; 2] {
    let x = l
        .states
        .iter()
        .map(|s| s.position[0])
        .fold(f32::INFINITY, f32::min);
    let y = l
        .states
        .iter()
        .map(|s| s.position[1])
        .fold(f32::INFINITY, f32::min);
    if x.is_finite() {
        [x - NODE_W * 0.6, y - NODE_H - 80.0]
    } else {
        [-NODE_W * 0.6, -NODE_H - 80.0]
    }
}

/// How scenes name the graph at `path`: relative to the scene folder
/// (the asset root), else `anims/<file>`.
pub(super) fn scene_path(path: &Path) -> String {
    automata_assets::root()
        .and_then(|root| {
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            abs.strip_prefix(root).ok().map(|p| {
                p.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
        })
        .unwrap_or_else(|| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{ANIMS_DIR}/{name}")
        })
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ---------------------------------------------------------------------------
// The editor's plumbing on `Ui`.

impl Ui {
    /// The path of the visible tab when it's an animation graph.
    #[must_use]
    pub fn anim_editor_path(&self) -> Option<PathBuf> {
        self.code
            .current()
            .filter(|t| automata_assets::anim_graph::is_anim_graph_path(&t.path))
            .map(|t| t.path.clone())
    }

    /// Whether the graph editor is the visible center tab.
    #[must_use]
    pub fn anim_editor_shown(&self) -> bool {
        self.anim_editor_path()
            .is_some_and(|p| !self.animed.source.contains(&p))
    }

    /// Shows the active graph as text (`true`) or in the editor.
    pub fn set_anim_source(&mut self, text: bool) {
        let Some(path) = self.anim_editor_path() else {
            return;
        };
        if text {
            self.animed.source.insert(path);
        } else {
            self.animed.source.remove(&path);
        }
        self.animed.drag = None;
        self.animed.linking = None;
    }

    /// The graph the editor shows, as last parsed (`None` while it doesn't
    /// parse or isn't shown).
    #[must_use]
    pub fn anim_editor_graph(&self) -> Option<&AnimGraph> {
        self.anim_editor_path()?;
        match &self.animed.parsed {
            Some((_, Ok(g))) => Some(g),
            _ => None,
        }
    }

    /// The graph editor's selection and shown layer.
    #[must_use]
    pub fn anim_editor_selection(&self) -> (usize, AnimSel) {
        (self.animed.layer, self.animed.sel)
    }

    /// Every open graph tab whose text differs from its file, with what
    /// it reads as (`None`: saved, or it doesn't read). The app hands
    /// these to [`automata_assets::anim_graph::set_anim_graph_preview`] so
    /// Play uses unsaved changes.
    #[must_use]
    pub fn anim_tab_previews(&self) -> Vec<(PathBuf, Option<AnimGraph>)> {
        self.code
            .tabs
            .iter()
            .filter(|t| automata_assets::anim_graph::is_anim_graph_path(&t.path))
            .map(|t| {
                let graph = t
                    .buf
                    .dirty()
                    .then(|| AnimGraph::parse(&t.buf.text()).ok())
                    .flatten();
                (t.path.clone(), graph)
            })
            .collect()
    }

    /// Shows what an entity's graph is doing (during Play), or nothing.
    pub fn set_anim_live(&mut self, live: Option<AnimLive>) {
        self.animed.live = live;
    }

    /// Finds the model whose clips and bones the open graph offers: an
    /// entity using the graph, else the selected animated entity, else
    /// any animated one. Looked up twice a second.
    pub(super) fn anim_find_model(&mut self, doc: &crate::doc::Document) {
        let Some(path) = self.anim_editor_path() else {
            return;
        };
        let fresh = self
            .animed
            .model_for
            .as_ref()
            .is_some_and(|(p, at)| *p == path && at.elapsed() < Duration::from_millis(500));
        if fresh {
            return;
        }
        self.animed.model_for = Some((path.clone(), Instant::now()));
        let me = canonical(&path);
        let rigged = |e: &crate::doc::EntityEntry| {
            crate::doc::model_rig(&e.data)
                .filter(|r| !r.clips.is_empty())
                .map(|r| (e.name().to_string(), r))
        };
        let users = doc.entities.iter().filter(|e| {
            e.data
                .animation
                .as_ref()
                .map(|a| a.graph.trim())
                .is_some_and(|g| !g.is_empty() && canonical(&doc.resolve(g)) == me)
        });
        let selected = doc.selected_index().map(|i| &doc.entities[i]);
        self.animed.model = users
            .chain(selected)
            .chain(doc.entities.iter())
            .find_map(rigged);
    }

    /// Re-parses the tab's text when it changed; a new file gets fitted
    /// into view.
    pub(super) fn refresh_anim(&mut self, idx: usize) {
        let text = self.code.tabs[idx].buf.text();
        if self.animed.parsed.as_ref().map(|(t, _)| t) != Some(&text) {
            let graph = AnimGraph::parse(&text);
            self.animed.parsed = Some((text, graph));
        }
        let path = self.code.tabs[idx].path.clone();
        if self.animed.framed.as_ref() != Some(&path) {
            self.animed.framed = Some(path);
            self.animed.layer = 0;
            self.animed.sel = AnimSel::None;
            self.animed.linking = None;
            self.animed.drag = None;
            self.animed.refit = true;
            self.anim_frame_all();
        }
        // Undo can take away what was selected.
        let Some((_, Ok(g))) = &self.animed.parsed else {
            return;
        };
        let layer = self.animed.layer.min(g.layers.len().saturating_sub(1));
        let sel = match (self.animed.sel, g.layers.get(layer)) {
            (_, None) => AnimSel::None,
            (AnimSel::State(i), Some(l)) if i >= l.states.len() => AnimSel::None,
            (AnimSel::Transition(i), Some(l)) if i >= l.transitions.len() => AnimSel::None,
            (s, Some(_)) => s,
        };
        self.animed.layer = layer;
        self.animed.sel = sel;
    }

    /// Fits every node of the shown layer in view.
    pub(super) fn anim_frame_all(&mut self) {
        let Some((_, Ok(g))) = &self.animed.parsed else {
            return;
        };
        let Some(l) = g.layers.get(self.animed.layer) else {
            self.animed.pan = [0.0, 0.0];
            self.animed.zoom = 1.0;
            return;
        };
        let (lo, hi) = layer_bounds(l);
        self.animed.pan = [-(lo[0] + hi[0]) / 2.0, -(lo[1] + hi[1]) / 2.0];
        let zoom = self.animed.canvas_rect.map_or(1.0, |c| {
            let fit_w = (c.w - 80.0) / (hi[0] - lo[0]).max(1.0);
            let fit_h = (c.h - 80.0) / (hi[1] - lo[1]).max(1.0);
            fit_w.min(fit_h)
        });
        self.animed.zoom = zoom.clamp(0.4, 1.0);
    }

    /// Changes the visible graph: `f` edits a copy; a real change rewrites
    /// the tab's text as one undo step.
    pub(super) fn edit_graph(&mut self, f: impl FnOnce(&mut AnimGraph)) {
        let Some(idx) = self.code.active.filter(|_| self.anim_editor_shown()) else {
            return;
        };
        self.refresh_anim(idx);
        let Some((_, Ok(before))) = &self.animed.parsed else {
            return;
        };
        let mut graph = before.clone();
        f(&mut graph);
        if graph == *before {
            return;
        }
        let tab = &mut self.code.tabs[idx];
        let text = graph.to_ron(&scene_path(&tab.path));
        tab.buf.set_text(&text);
        self.animed.parsed = Some((text, Ok(graph)));
    }

    /// The model's clip names (empty without a model).
    pub(super) fn anim_clips(&self) -> Vec<String> {
        self.animed
            .model
            .as_ref()
            .map(|(_, r)| r.clips.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The model's bone names (empty without a model).
    pub(super) fn anim_bones(&self) -> Vec<String> {
        self.animed
            .model
            .as_ref()
            .map(|(_, r)| r.nodes.iter().map(|n| n.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Runs a graph editor command.
    #[allow(clippy::too_many_lines)]
    pub(super) fn anim_action(&mut self, a: AnimAction) {
        let layer = self.animed.layer;
        let sel = self.animed.sel;
        match a {
            AnimAction::AddParam(kind) => self.edit_graph(|g| {
                add_parameter(g, kind);
            }),
            AnimAction::DeleteParam(i) => self.edit_graph(|g| delete_parameter(g, i)),
            AnimAction::Layer(i) => {
                self.animed.layer = i;
                self.animed.sel = AnimSel::None;
                self.animed.linking = None;
                self.anim_frame_all();
            }
            AnimAction::AddLayer => {
                let mut added = None;
                self.edit_graph(|g| {
                    let name = unique("Layer", |n| g.layer_index(n).is_some());
                    g.layers.push(GraphLayer {
                        name,
                        states: vec![State {
                            name: "Off".into(),
                            ..State::default()
                        }],
                        ..GraphLayer::default()
                    });
                    added = Some(g.layers.len() - 1);
                });
                if let Some(i) = added {
                    self.anim_action(AnimAction::Layer(i));
                }
            }
            AnimAction::DeleteLayer => {
                self.edit_graph(|g| {
                    if g.layers.len() > 1 && layer < g.layers.len() {
                        g.layers.remove(layer);
                    }
                });
                self.anim_action(AnimAction::Layer(layer.saturating_sub(1)));
            }
            AnimAction::AddState { x, y, kind } => {
                let first = self.anim_clips().into_iter().next();
                let mut added = None;
                self.edit_graph(|g| added = add_state(g, layer, [x, y], kind, first.as_deref()));
                if let Some(i) = added {
                    self.animed.sel = AnimSel::State(i);
                }
            }
            AnimAction::DeleteState(i) => {
                self.edit_graph(|g| delete_state(g, layer, i));
                self.animed.sel = AnimSel::None;
            }
            AnimAction::SetEntry(i) => self.edit_graph(|g| {
                if let Some(l) = g.layers.get_mut(layer) {
                    if let Some(s) = l.states.get(i) {
                        l.entry.clone_from(&s.name);
                    }
                }
            }),
            AnimAction::Link(node) => self.animed.linking = Some(node),
            AnimAction::DeleteTransition(i) => {
                self.edit_graph(|g| {
                    if let Some(l) = g.layers.get_mut(layer).filter(|l| i < l.transitions.len()) {
                        l.transitions.remove(i);
                    }
                });
                self.animed.sel = AnimSel::None;
            }
            AnimAction::FrameAll => self.anim_frame_all(),
            AnimAction::AddMaskBone(i) => {
                let Some(bone) = self.anim_bones().get(i).cloned() else {
                    return;
                };
                self.edit_graph(|g| {
                    if let Some(l) = g.layers.get_mut(layer) {
                        if !l.mask.contains(&bone) {
                            l.mask.push(bone);
                        }
                    }
                });
            }
            _ => self.anim_selection_action(a, layer, sel),
        }
    }

    /// The commands on the selected state or transition.
    #[allow(clippy::too_many_lines)]
    fn anim_selection_action(&mut self, a: AnimAction, layer: usize, sel: AnimSel) {
        let clips = self.anim_clips();
        match (a, sel) {
            (AnimAction::MotionKind(kind), AnimSel::State(i)) => self.edit_graph(|g| {
                let params = g.parameters.clone();
                if let Some(s) = g.layers.get_mut(layer).and_then(|l| l.states.get_mut(i)) {
                    if motion_kind(&s.motion) != kind {
                        s.motion = with_motion_kind(&s.motion, kind, &params);
                    }
                }
            }),
            (AnimAction::Clip(c), AnimSel::State(i)) => {
                let Some(clip) = clips.get(c).cloned() else {
                    return;
                };
                self.edit_state(layer, i, |s| s.motion = Motion::Clip(clip));
            }
            (AnimAction::PointClip(p, c), AnimSel::State(i)) => {
                let Some(clip) = clips.get(c).cloned() else {
                    return;
                };
                self.edit_state(layer, i, |s| match &mut s.motion {
                    Motion::Blend1D { points, .. } => {
                        if let Some(pt) = points.get_mut(p) {
                            pt.clip = clip;
                        }
                    }
                    Motion::Blend2D { points, .. } => {
                        if let Some(pt) = points.get_mut(p) {
                            pt.clip = clip;
                        }
                    }
                    _ => {}
                });
            }
            (AnimAction::AddPoint, AnimSel::State(i)) => {
                let clip = clips.first().cloned().unwrap_or_default();
                self.edit_state(layer, i, |s| match &mut s.motion {
                    Motion::Blend1D { points, .. } => {
                        let at = points.iter().map(|p| p.at).fold(0.0, f32::max) + 1.0;
                        points.push(BlendPoint1D { clip, at });
                    }
                    Motion::Blend2D { points, .. } => {
                        points.push(BlendPoint2D {
                            clip,
                            at: [0.5, 0.5],
                        });
                    }
                    _ => {}
                });
            }
            (AnimAction::RemovePoint(p), AnimSel::State(i)) => {
                self.edit_state(layer, i, |s| match &mut s.motion {
                    Motion::Blend1D { points, .. } if p < points.len() => {
                        points.remove(p);
                    }
                    Motion::Blend2D { points, .. } if p < points.len() => {
                        points.remove(p);
                    }
                    _ => {}
                });
            }
            (AnimAction::BlendParam(axis, p), AnimSel::State(i)) => {
                let Some(name) = self.anim_param_name(p) else {
                    return;
                };
                self.edit_state(layer, i, |s| match (&mut s.motion, axis) {
                    (Motion::Blend1D { param, .. }, _) => *param = name,
                    (Motion::Blend2D { x, .. }, 0) => *x = name,
                    (Motion::Blend2D { y, .. }, _) => *y = name,
                    _ => {}
                });
            }
            (AnimAction::SpeedParam(p), AnimSel::State(i)) => {
                let name = p.and_then(|p| self.anim_param_name(p)).unwrap_or_default();
                self.edit_state(layer, i, |s| s.speed_param = name);
            }
            (AnimAction::AddCondition, AnimSel::Transition(t)) => self.edit_graph(|g| {
                let Some(p) = g.parameters.first().cloned() else {
                    return;
                };
                if let Some(tr) = g
                    .layers
                    .get_mut(layer)
                    .and_then(|l| l.transitions.get_mut(t))
                {
                    tr.when.push(condition_for(&p, None));
                }
            }),
            (AnimAction::RemoveCondition(c), AnimSel::Transition(t)) => {
                self.edit_transition(layer, t, |tr| {
                    if c < tr.when.len() {
                        tr.when.remove(c);
                    }
                });
            }
            (AnimAction::CondParam(c, p), AnimSel::Transition(t)) => self.edit_graph(|g| {
                let Some(param) = g.parameters.get(p).cloned() else {
                    return;
                };
                if let Some(cond) = g
                    .layers
                    .get_mut(layer)
                    .and_then(|l| l.transitions.get_mut(t))
                    .and_then(|tr| tr.when.get_mut(c))
                {
                    *cond = condition_for(&param, Some(cond));
                }
            }),
            (AnimAction::CondOp(c, op), AnimSel::Transition(t)) => {
                self.edit_transition(layer, t, |tr| {
                    if let Some(cond) = tr.when.get_mut(c) {
                        let value = cond_value(cond).unwrap_or(0.0);
                        *cond = make_condition(op, cond.param().to_string(), value);
                    }
                });
            }
            _ => {}
        }
    }

    fn anim_param_name(&self, i: usize) -> Option<String> {
        self.anim_editor_graph()?
            .parameters
            .get(i)
            .map(|p| p.name.clone())
    }

    /// Edits state `i` of `layer`.
    pub(super) fn edit_state(&mut self, layer: usize, i: usize, f: impl FnOnce(&mut State)) {
        self.edit_graph(|g| {
            if let Some(s) = g.layers.get_mut(layer).and_then(|l| l.states.get_mut(i)) {
                f(s);
            }
        });
    }

    /// Edits transition `t` of `layer`.
    pub(super) fn edit_transition(
        &mut self,
        layer: usize,
        t: usize,
        f: impl FnOnce(&mut Transition),
    ) {
        self.edit_graph(|g| {
            if let Some(tr) = g
                .layers
                .get_mut(layer)
                .and_then(|l| l.transitions.get_mut(t))
            {
                f(tr);
            }
        });
    }

    /// Finishes a transition being drawn at `to`.
    pub(super) fn anim_finish_link(&mut self, to: AnimNode) {
        let Some(from) = self.animed.linking.take() else {
            return;
        };
        let AnimNode::State(to) = to else {
            return; // nothing goes into Any State
        };
        let layer = self.animed.layer;
        let Some(l) = self.anim_editor_graph().and_then(|g| g.layers.get(layer)) else {
            return;
        };
        let from = match from {
            AnimNode::Any => ANY_STATE.to_string(),
            AnimNode::State(i) => match l.states.get(i) {
                Some(s) => s.name.clone(),
                None => return,
            },
        };
        let Some(to) = l.states.get(to).map(|s| s.name.clone()) else {
            return;
        };
        let mut made = None;
        self.edit_graph(|g| {
            made = add_transition(g, layer, &from, &to);
            // From Any State it'd fire all the time without a condition:
            // start with one on a trigger (else the first parameter).
            let first = g
                .parameters
                .iter()
                .find(|p| p.kind == ParamKind::Trigger)
                .or_else(|| g.parameters.first())
                .cloned();
            if let (Some(i), Some(p), true) = (made, first, from == ANY_STATE) {
                let t = &mut g.layers[layer].transitions[i];
                if t.when.is_empty() {
                    t.when.push(condition_for(&p, None));
                }
            }
        });
        if let Some(i) = made {
            self.animed.sel = AnimSel::Transition(i);
        }
    }

    /// Commits a typed graph field.
    #[allow(clippy::too_many_lines)]
    pub(super) fn commit_anim_prop(&mut self, prop: AnimProp, text: &str) {
        let text = text.trim();
        let layer = self.animed.layer;
        let sel = self.animed.sel;
        let number = super::eval_number(text);
        let needs_number = !matches!(
            prop,
            AnimProp::ParamName(_)
                | AnimProp::LayerName
                | AnimProp::LayerMask
                | AnimProp::StateName
                | AnimProp::Clip
                | AnimProp::PointClip(_)
                | AnimProp::ParamValue(_)
                | AnimProp::ExitTime
        );
        if needs_number && number.is_none() {
            self.log(
                LogLevel::Warn,
                format!("\u{201c}{text}\u{201d} isn't a number"),
            );
            return;
        }
        let v = number.unwrap_or(0.0);
        match (prop, sel) {
            (AnimProp::ParamName(i), _) => self.rename_anim_param(i, text),
            (AnimProp::ParamValue(i), _) => self.edit_graph(|g| {
                if let Some(p) = g.parameters.get_mut(i) {
                    p.kind = match p.kind {
                        ParamKind::Float(old) => ParamKind::Float(number.unwrap_or(old)),
                        #[allow(clippy::cast_possible_truncation)] // typed by a person
                        ParamKind::Int(old) => {
                            ParamKind::Int(number.map_or(old, |n| n.round() as i32))
                        }
                        k => k,
                    };
                }
            }),
            (AnimProp::LayerName, _) => self.rename_anim_layer(layer, text),
            (AnimProp::LayerWeight, _) => self.edit_graph(|g| {
                if let Some(l) = g.layers.get_mut(layer) {
                    l.weight = v.clamp(0.0, 1.0);
                }
            }),
            (AnimProp::LayerMask, _) => self.edit_graph(|g| {
                if let Some(l) = g.layers.get_mut(layer) {
                    l.mask = text
                        .split(',')
                        .map(str::trim)
                        .filter(|b| !b.is_empty())
                        .map(str::to_string)
                        .collect();
                }
            }),
            (AnimProp::StateName, AnimSel::State(i)) => self.rename_anim_state(layer, i, text),
            (AnimProp::StateSpeed, AnimSel::State(i)) => self.edit_state(layer, i, |s| s.speed = v),
            (AnimProp::Clip, AnimSel::State(i)) => {
                self.edit_state(layer, i, |s| s.motion = Motion::Clip(text.to_string()));
            }
            (AnimProp::PointClip(p), AnimSel::State(i)) => {
                let clip = text.to_string();
                self.edit_state(layer, i, |s| match &mut s.motion {
                    Motion::Blend1D { points, .. } => {
                        if let Some(pt) = points.get_mut(p) {
                            pt.clip = clip;
                        }
                    }
                    Motion::Blend2D { points, .. } => {
                        if let Some(pt) = points.get_mut(p) {
                            pt.clip = clip;
                        }
                    }
                    _ => {}
                });
            }
            (AnimProp::PointAt(p, axis), AnimSel::State(i)) => {
                self.edit_state(layer, i, |s| set_point(&mut s.motion, p, axis, v));
            }
            (AnimProp::Fade, AnimSel::Transition(t)) => {
                self.edit_transition(layer, t, |tr| tr.fade = v.max(0.0));
            }
            (AnimProp::ExitTime, AnimSel::Transition(t)) => {
                self.edit_transition(layer, t, |tr| tr.exit_time = number.map(|n| n.max(0.0)));
            }
            (AnimProp::CondValue(c), AnimSel::Transition(t)) => {
                self.edit_transition(layer, t, |tr| {
                    if let Some(cond) = tr.when.get_mut(c) {
                        *cond = make_condition(cond_op(cond), cond.param().to_string(), v);
                    }
                });
            }
            _ => {}
        }
    }

    fn rename_anim_param(&mut self, i: usize, to: &str) {
        let to = to.replace(char::is_whitespace, "_");
        let Some(g) = self.anim_editor_graph() else {
            return;
        };
        let Some(from) = g.parameters.get(i).map(|p| p.name.clone()) else {
            return;
        };
        if to.is_empty() || to == from {
            return;
        }
        if g.parameter(&to).is_some() {
            self.log(
                LogLevel::Warn,
                format!("There's already a parameter called {to}"),
            );
            return;
        }
        self.edit_graph(|g| g.rename_parameter(&from, &to));
    }

    fn rename_anim_layer(&mut self, layer: usize, to: &str) {
        let Some(g) = self.anim_editor_graph() else {
            return;
        };
        if to.is_empty() || g.layer_index(to).is_some_and(|i| i != layer) {
            if !to.is_empty() {
                self.log(
                    LogLevel::Warn,
                    format!("There's already a layer called {to}"),
                );
            }
            return;
        }
        let to = to.to_string();
        self.edit_graph(|g| {
            if let Some(l) = g.layers.get_mut(layer) {
                l.name = to;
            }
        });
    }

    fn rename_anim_state(&mut self, layer: usize, i: usize, to: &str) {
        let Some(l) = self.anim_editor_graph().and_then(|g| g.layers.get(layer)) else {
            return;
        };
        let Some(from) = l.states.get(i).map(|s| s.name.clone()) else {
            return;
        };
        if to.is_empty() || to == from || to == ANY_STATE {
            return;
        }
        if l.state_index(to).is_some() {
            self.log(
                LogLevel::Warn,
                format!("There's already a state called {to}"),
            );
            return;
        }
        let to = to.to_string();
        self.edit_graph(|g| g.rename_state(layer, &from, &to));
    }

    /// Keys while the graph editor has the keyboard: saving, undo and
    /// closing work on the file's text, Delete removes the selection and
    /// Escape stops drawing a transition.
    pub(super) fn anim_editor_key(&mut self, key: Key, m: Mods) -> bool {
        match (key, m.ctrl) {
            (Key::Char('s' | 'z' | 'y' | 'w'), true) => self.code.key(key, m),
            (_, true) => false,
            (Key::Delete | Key::Backspace, false) => {
                match self.animed.sel {
                    AnimSel::State(i) => self.anim_action(AnimAction::DeleteState(i)),
                    AnimSel::Transition(i) => self.anim_action(AnimAction::DeleteTransition(i)),
                    AnimSel::None | AnimSel::Any => {}
                }
                true
            }
            (Key::Escape, _) => {
                self.animed.linking = None;
                self.animed.drag = None;
                true
            }
            _ => !matches!(key, Key::F3 | Key::F4 | Key::F12),
        }
    }
}

/// Sets axis `axis` of blend point `p`.
pub(super) fn set_point(m: &mut Motion, p: usize, axis: u8, v: f32) {
    match m {
        Motion::Blend1D { points, .. } => {
            if let Some(pt) = points.get_mut(p) {
                pt.at = v;
            }
        }
        Motion::Blend2D { points, .. } => {
            if let Some(pt) = points.get_mut(p) {
                pt.at[usize::from(axis.min(1))] = v;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk() -> AnimGraph {
        let mut g = AnimGraph::starter(&["Idle", "Walking", "Running"]);
        g.parameters.push(Parameter::trigger("jump"));
        g
    }

    #[test]
    fn names_stay_unique() {
        let taken = ["Idle", "Idle 2"];
        assert_eq!(unique("Idle", |n| taken.contains(&n)), "Idle 3");
        assert_eq!(unique("Run", |n| taken.contains(&n)), "Run");
    }

    #[test]
    fn states_come_and_go_with_their_transitions() {
        let mut g = walk();
        let i = add_state(&mut g, 0, [10.4, 20.6], 1, Some("Idle")).unwrap();
        let s = &g.layers[0].states[i];
        assert_eq!((s.name.as_str(), s.position), ("Idle 2", [10.0, 21.0]));
        assert_eq!(s.motion, Motion::Clip("Idle".into()));
        // Transitions: one per pair, from states or Any State only.
        let t = add_transition(&mut g, 0, "*", "Idle 2").unwrap();
        assert_eq!(add_transition(&mut g, 0, "*", "Idle 2"), Some(t));
        assert_eq!(add_transition(&mut g, 0, "Nowhere", "Idle"), None);
        let before = g.layers[0].transitions.len();
        delete_state(&mut g, 0, i);
        assert_eq!(g.layers[0].transitions.len(), before - 1);
        // Deleting the entry state clears the entry.
        assert_eq!(g.layers[0].entry, "Idle");
        delete_state(&mut g, 0, 0);
        assert_eq!(g.layers[0].entry, "");
        assert_eq!(g.problems(None, None), Vec::<String>::new());
    }

    #[test]
    fn motions_change_kind_keeping_their_clips() {
        let params = vec![
            Parameter::float("speed", 0.0),
            Parameter::float("side", 0.0),
        ];
        let walk = Motion::Clip("Walking".into());
        let Motion::Blend1D { param, points } = with_motion_kind(&walk, 2, &params) else {
            panic!("a 1D blend");
        };
        assert_eq!((param.as_str(), points.len()), ("speed", 1));
        assert_eq!(points[0].clip, "Walking");
        let Motion::Blend2D { x, y, .. } = with_motion_kind(&walk, 3, &params) else {
            panic!("a 2D blend");
        };
        assert_eq!((x.as_str(), y.as_str()), ("side", "speed"));
        let back = with_motion_kind(&with_motion_kind(&walk, 2, &params), 1, &params);
        assert_eq!(back, walk);
        assert_eq!(with_motion_kind(&walk, 0, &params), Motion::Empty);
    }

    #[test]
    fn parameters_take_their_conditions_with_them() {
        let mut g = walk();
        let i = add_parameter(&mut g, 0);
        assert_eq!(g.parameters[i].name, "speed_2");
        let jump = g.parameters.iter().position(|p| p.name == "jump").unwrap();
        add_transition(&mut g, 0, "*", "Walking").unwrap();
        let last = g.layers[0].transitions.len() - 1;
        g.layers[0].transitions[last]
            .when
            .push(Condition::Triggered("jump".into()));
        delete_parameter(&mut g, jump);
        assert_eq!(g.layers[0].transitions[last].when.len(), 0);
        // Which leaves an Any State transition that would always fire:
        // the status line says so.
        let problems = g.problems(None, None);
        assert!(
            problems.len() == 1 && problems[0].contains("no conditions"),
            "{problems:?}"
        );
    }

    #[test]
    fn conditions_suit_their_parameter() {
        let speed = Parameter::float("speed", 0.0);
        let jump = Parameter::trigger("jump");
        let c = condition_for(&speed, None);
        assert_eq!(c, Condition::Above("speed".into(), 0.1));
        // Switching to a trigger changes the test; back keeps the value.
        assert_eq!(
            condition_for(&jump, Some(&c)),
            Condition::Triggered("jump".into())
        );
        let below = Condition::Below("x".into(), 2.0);
        assert_eq!(
            condition_for(&speed, Some(&below)),
            Condition::Below("speed".into(), 2.0)
        );
        for op in 0..7 {
            assert_eq!(cond_op(&make_condition(op, "p".into(), 1.0)), op);
        }
    }

    #[test]
    fn any_state_sits_left_of_the_states() {
        let g = walk();
        let l = &g.layers[0];
        let any = any_position(l);
        assert!(any[0] < 0.0 && any[1] + NODE_H < 0.0, "{any:?}");
        let (lo, hi) = layer_bounds(l);
        assert_eq!(lo, any);
        assert!(hi[0] >= 400.0 + NODE_W, "{hi:?}");
    }

    // ----- the editor, frame by frame ------------------------------------

    use crate::canvas::Canvas;
    use crate::doc::Document;
    use crate::ui::Pointer;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oberon-animed-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("anims")).unwrap();
        dir
    }

    /// An editor with `g` saved and open in its tab.
    fn open(tag: &str, g: &AnimGraph) -> (PathBuf, PathBuf, Ui, Canvas) {
        let dir = temp(tag);
        let path = dir.join("anims/hero.anim.ron");
        g.save(&path, "hero").unwrap();
        let mut ui = Ui::new();
        let mut canvas = Canvas::new(1400, 900);
        ui.open_code(&path, None);
        for _ in 0..2 {
            ui.frame(&mut canvas, &Document::new(), Pointer::default(), None);
        }
        (dir, path, ui, canvas)
    }

    fn at(x: f32, y: f32, down: bool, pressed: bool) -> Pointer {
        Pointer {
            x,
            y,
            down,
            pressed,
            ..Pointer::default()
        }
    }

    fn node(ui: &Ui, n: AnimNode) -> super::super::Rect {
        ui.animed
            .nodes
            .iter()
            .find(|(m, _)| *m == n)
            .map(|(_, r)| *r)
            .expect("the node is drawn")
    }

    fn graph(ui: &Ui) -> AnimGraph {
        ui.anim_editor_graph().cloned().expect("the graph reads")
    }

    #[test]
    fn a_graph_opens_and_dragging_a_state_moves_it_in_one_step() {
        let (dir, _path, mut ui, mut canvas) = open("drag", &walk());
        let doc = Document::new();
        assert!(ui.anim_editor_shown());
        assert_eq!(ui.anim_editor_graph(), Some(&walk()));
        assert_eq!(ui.anim_tab_previews()[0].1, None, "nothing unsaved");
        // Every state and Any State is on screen after framing.
        let canvas_rect = ui.animed.canvas_rect.unwrap();
        assert_eq!(ui.animed.nodes.len(), 4);
        for (_, r) in &ui.animed.nodes {
            assert!(
                canvas_rect.contains(r.x, r.y) && canvas_rect.contains(r.x + r.w, r.y + r.h),
                "{r:?} in {canvas_rect:?} (pan {:?}, zoom {})",
                ui.animed.pan,
                ui.animed.zoom
            );
        }
        let r = node(&ui, AnimNode::State(1));
        let start = graph(&ui).layers[0].states[1].position;
        let undo = ui.code.tabs[0].buf.undo_depth();
        let (x, y) = (r.x + 20.0, r.y + 10.0);
        ui.frame(&mut canvas, &doc, at(x, y, true, true), None);
        assert_eq!(ui.anim_editor_selection(), (0, AnimSel::State(1)));
        let z = ui.animed.zoom;
        for k in 1..=4 {
            ui.frame(
                &mut canvas,
                &doc,
                at(
                    x + 25.0 * z * k as f32,
                    y + 15.0 * z * k as f32,
                    true,
                    false,
                ),
                None,
            );
        }
        assert_eq!(
            ui.code.tabs[0].buf.undo_depth(),
            undo,
            "not written while dragging"
        );
        ui.frame(
            &mut canvas,
            &doc,
            at(x + 100.0 * z, y + 60.0 * z, false, false),
            None,
        );
        ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let moved = graph(&ui).layers[0].states[1].position;
        assert_eq!(moved, [start[0] + 100.0, start[1] + 60.0]);
        assert_eq!(ui.code.tabs[0].buf.undo_depth(), undo + 1, "one step");
        assert!(
            ui.anim_tab_previews()[0].1.is_some(),
            "Play runs the unsaved graph"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transitions_are_drawn_between_states_and_edited_in_the_panel() {
        let mut g = walk();
        g.layers[0].transitions.clear();
        let (dir, _path, mut ui, mut canvas) = open("link", &g);
        let doc = Document::new();
        // Make Transition from Any State, then click Idle.
        ui.anim_action(AnimAction::Link(AnimNode::Any));
        let r = node(&ui, AnimNode::State(0));
        ui.frame(
            &mut canvas,
            &doc,
            at(r.x + 30.0, r.y + 20.0, true, true),
            None,
        );
        ui.frame(&mut canvas, &doc, Pointer::default(), None);
        let l = &graph(&ui).layers[0];
        assert_eq!(l.transitions.len(), 1);
        assert_eq!(
            (l.transitions[0].from.as_str(), l.transitions[0].to.as_str()),
            ("*", "Idle")
        );
        assert_eq!(ui.anim_editor_selection().1, AnimSel::Transition(0));
        // From Any State it needs a condition: it starts with one on
        // `jump`, the trigger; a second tests the speed.
        assert_eq!(
            l.transitions[0].when,
            vec![Condition::Triggered("jump".into())]
        );
        ui.anim_action(AnimAction::AddCondition);
        ui.anim_action(AnimAction::CondParam(1, 0));
        ui.anim_action(AnimAction::CondOp(1, 1));
        ui.commit_anim_prop(AnimProp::CondValue(1), "2.5");
        ui.commit_anim_prop(AnimProp::Fade, "0.35");
        ui.commit_anim_prop(AnimProp::ExitTime, "0.5");
        let t = graph(&ui).layers[0].transitions[0].clone();
        assert_eq!(
            t.when,
            vec![
                Condition::Triggered("jump".into()),
                Condition::Below("speed".into(), 2.5)
            ]
        );
        assert!((t.fade - 0.35).abs() < 1e-6);
        assert_eq!(t.exit_time, Some(0.5));
        ui.commit_anim_prop(AnimProp::ExitTime, "");
        assert_eq!(graph(&ui).layers[0].transitions[0].exit_time, None);
        // Renaming a state follows into its transitions and the entry.
        ui.animed.sel = AnimSel::State(0);
        ui.commit_anim_prop(AnimProp::StateName, "Rest");
        let l = &graph(&ui).layers[0];
        assert_eq!(
            (l.entry.as_str(), l.transitions[0].to.as_str()),
            ("Rest", "Rest")
        );
        // Esc stops a transition being drawn; Delete removes the selection.
        ui.anim_action(AnimAction::Link(AnimNode::State(1)));
        assert!(ui.anim_editor_key(Key::Escape, Mods::default()));
        assert_eq!(ui.animed.linking, None);
        ui.animed.sel = AnimSel::Transition(0);
        ui.anim_editor_key(Key::Delete, Mods::default());
        assert_eq!(graph(&ui).layers[0].transitions.len(), 0);
        for _ in 0..2 {
            ui.frame(&mut canvas, &doc, Pointer::default(), None);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn blend_spaces_layers_and_parameters_edit_from_the_panels() {
        let (dir, _path, mut ui, mut canvas) = open("blend", &walk());
        let doc = Document::new();
        ui.anim_action(AnimAction::AddState {
            x: 0.0,
            y: 300.0,
            kind: 3,
        });
        let i = graph(&ui).layers[0].states.len() - 1;
        assert_eq!(ui.anim_editor_selection().1, AnimSel::State(i));
        ui.anim_action(AnimAction::AddPoint);
        let Motion::Blend2D { points, .. } = &graph(&ui).layers[0].states[i].motion else {
            panic!("a 2D blend space");
        };
        let last = points.len() - 1;
        ui.commit_anim_prop(AnimProp::PointClip(last), "Walking");
        ui.commit_anim_prop(AnimProp::PointAt(last, 1), "1");
        let Motion::Blend2D { points, .. } = &graph(&ui).layers[0].states[i].motion else {
            panic!("a 2D blend space");
        };
        assert_eq!(
            points.last().map(|p| (p.clip.as_str(), p.at[1])),
            Some(("Walking", 1.0))
        );
        // The panels draw it (with no model, clip names are typed).
        for _ in 0..2 {
            ui.frame(&mut canvas, &doc, Pointer::default(), None);
        }
        // A masked layer on top.
        ui.anim_action(AnimAction::AddLayer);
        assert_eq!(ui.anim_editor_selection().0, 1);
        ui.commit_anim_prop(AnimProp::LayerMask, "Shoulder.R, Shoulder.L");
        ui.commit_anim_prop(AnimProp::LayerWeight, "0.5");
        let l = &graph(&ui).layers[1];
        assert_eq!(l.mask, vec!["Shoulder.R".to_string(), "Shoulder.L".into()]);
        assert!((l.weight - 0.5).abs() < 1e-6);
        // Parameters: add, rename (conditions follow), set the start value.
        ui.anim_action(AnimAction::AddParam(1));
        let p = graph(&ui).parameters.len() - 1;
        ui.commit_anim_prop(AnimProp::ParamName(p), "lives");
        ui.commit_anim_prop(AnimProp::ParamValue(p), "3");
        assert_eq!(
            graph(&ui).parameters[p].kind,
            automata_assets::anim_graph::ParamKind::Int(3)
        );
        ui.commit_anim_prop(AnimProp::ParamName(0), "pace");
        assert_eq!(graph(&ui).parameters[0].name, "pace");
        assert!(graph(&ui).layers[0]
            .transitions
            .iter()
            .all(|t| t.when.iter().all(|c| c.param() != "speed")));
        for _ in 0..2 {
            ui.frame(&mut canvas, &doc, Pointer::default(), None);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_live_state_shows_during_play_and_broken_files_offer_text() {
        let (dir, path, mut ui, mut canvas) = open("live", &walk());
        let doc = Document::new();
        ui.set_anim_live(Some(AnimLive {
            entity: "Robot".into(),
            state: automata_scene::GraphState {
                layers: vec![automata_scene::GraphLayerState {
                    name: "Base".into(),
                    state: "Walking".into(),
                    time: 0.4,
                    from: Some("Idle".into()),
                    fade: 0.5,
                    weight: 1.0,
                    states: vec!["Idle".into(), "Walking".into(), "Running".into()],
                }],
                params: vec![automata_scene::GraphParam {
                    name: "speed".into(),
                    kind: "float",
                    value: 1.5,
                }],
            },
        }));
        for _ in 0..2 {
            ui.frame(&mut canvas, &doc, Pointer::default(), None);
        }
        // Text view and back.
        ui.set_anim_source(true);
        assert!(!ui.anim_editor_shown());
        ui.frame(&mut canvas, &doc, Pointer::default(), None);
        ui.set_anim_source(false);
        assert!(ui.anim_editor_shown());
        // A file that doesn't read: no graph, no panic.
        let _ = path;
        let idx = ui.code.active.unwrap();
        ui.code.tabs[idx].buf.set_text("(layers: [oops");
        ui.frame(&mut canvas, &doc, Pointer::default(), None);
        assert_eq!(ui.anim_editor_graph(), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
