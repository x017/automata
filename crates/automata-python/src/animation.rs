//! `at.Animation` (the component) and `AnimationRef` (a running entity's
//! playback, `view.animation`).

use std::cell::Cell;

use automata_scene::{
    Animation, AnimationCommand, AnimationState, GraphLayerState, GraphParam, PlayOptions,
};
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict};

use crate::scene::WriteQueue;

/// Which clip an animated model plays: what Oberon's Inspector calls an
/// Animation. The model is the entity's `MeshRenderer.model(...)`; `clip`
/// names one of its animations (empty: the rest pose). `play_on_start`
/// plays it as the scene starts; off, the model holds the clip's first
/// frame until a script plays something. `root_motion` lets the clip's own
/// travel across the ground move the entity (through its character
/// controller, if it has one) instead of sliding the model away from it.
/// `graph` names an animation graph (`.anim.ron`) that picks the clips
/// from parameters scripts set; then `clip` only matters if it doesn't
/// load.
///
/// ```python
/// robot = at.Entity("Robot").add(at.Transform()).add(
///     at.MeshRenderer.model("models/robot.glb")).add(at.Animation("Idle"))
/// hero.add(at.Animation(graph="anims/robot.anim.ron"))
/// ```
#[pyclass(name = "Animation", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyAnimation {
    inner: Animation,
}

#[pymethods]
impl PyAnimation {
    /// Creates an animation component.
    #[new]
    #[pyo3(signature = (
        clip="", speed=1.0, looping=true, play_on_start=true, root_motion=false, graph="",
        foot_ik=false
    ))]
    #[allow(clippy::fn_params_excessive_bools)] // Python keyword arguments
    fn new(
        clip: &str,
        speed: f32,
        looping: bool,
        play_on_start: bool,
        root_motion: bool,
        graph: &str,
        foot_ik: bool,
    ) -> PyResult<Self> {
        if !speed.is_finite() {
            return Err(PyValueError::new_err("speed must be a finite number"));
        }
        Ok(Self {
            inner: Animation {
                clip: clip.to_string(),
                speed,
                looping,
                play_on_start,
                root_motion,
                graph: graph.to_string(),
                foot_ik,
            },
        })
    }

    /// Whether the feet follow the ground (slopes, stairs).
    #[getter]
    fn foot_ik(&self) -> bool {
        self.inner.foot_ik
    }

    /// The animation graph (`.anim.ron` path) picking the clips ("" for
    /// none).
    #[getter]
    fn graph(&self) -> String {
        self.inner.graph.clone()
    }

    /// Whether the clip's travel moves the entity (root motion).
    #[getter]
    fn root_motion(&self) -> bool {
        self.inner.root_motion
    }

    /// The clip played on start ("" for the rest pose).
    #[getter]
    fn clip(&self) -> String {
        self.inner.clip.clone()
    }

    /// Playback rate (1 is normal speed, negative plays backwards).
    #[getter]
    fn speed(&self) -> f32 {
        self.inner.speed
    }

    /// Whether the clip starts over when it ends.
    #[getter]
    fn looping(&self) -> bool {
        self.inner.looping
    }

    /// Whether the clip plays as the scene starts.
    #[getter]
    fn play_on_start(&self) -> bool {
        self.inner.play_on_start
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let py_bool = |b: bool| if b { "True" } else { "False" };
        format!(
            "Animation({:?}, speed={}, looping={}, play_on_start={}{}{}{})",
            self.inner.clip,
            self.inner.speed,
            py_bool(self.inner.looping),
            py_bool(self.inner.play_on_start),
            if self.inner.root_motion {
                ", root_motion=True"
            } else {
                ""
            },
            if self.inner.graph.is_empty() {
                String::new()
            } else {
                format!(", graph={:?}", self.inner.graph)
            },
            if self.inner.foot_ik {
                ", foot_ik=True"
            } else {
                ""
            },
        )
    }
}

impl PyAnimation {
    /// Wraps an engine component.
    #[must_use]
    pub fn from_inner(inner: Animation) -> Self {
        Self { inner }
    }

    /// The engine component.
    #[must_use]
    pub fn inner(&self) -> Animation {
        self.inner.clone()
    }
}

/// Follows a bone of another entity's animated model: a sword in a hand,
/// a hat on a head. The entity's `Transform` is then where it sits in the
/// bone's space; it keeps its own size.
///
/// ```python
/// sword = at.Entity("Sword").add(at.Transform(rotation=...)).add(
///     at.MeshRenderer.model("models/sword.glb")).add(at.Attach("Robot", "Palm2.R"))
/// ```
#[pyclass(name = "Attach", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyAttach {
    pub(crate) inner: automata_core::scene_file::AttachData,
}

#[pymethods]
impl PyAttach {
    /// Follows `bone` of the entity named `to` (no bone: the entity itself).
    #[new]
    #[pyo3(signature = (to, bone=""))]
    fn new(to: &str, bone: &str) -> Self {
        Self {
            inner: automata_core::scene_file::AttachData {
                to: to.to_string(),
                bone: bone.to_string(),
            },
        }
    }

    /// The name of the entity followed.
    #[getter]
    fn to(&self) -> String {
        self.inner.to.clone()
    }

    /// The bone followed ("" for the entity's origin).
    #[getter]
    fn bone(&self) -> String {
        self.inner.bone.clone()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        if self.inner.bone.is_empty() {
            format!("Attach({:?})", self.inner.to)
        } else {
            format!("Attach({:?}, {:?})", self.inner.to, self.inner.bone)
        }
    }
}

/// A running entity's animation (`view.animation`). Reads show the start
/// of the frame; `play`, `stop`, and setting `time` or `speed` apply after
/// the update callback returns. With an animation graph, `set`,
/// `trigger` and `play_state` steer it (applied the same way).
#[pyclass(name = "AnimationRef", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct PyAnimationRef {
    id: u64,
    state: AnimationState,
    writes: WriteQueue,
    /// A new graph was asked for: its names aren't known yet, so they
    /// aren't checked.
    graph_changed: Cell<bool>,
}

impl PyAnimationRef {
    pub(crate) fn new(id: u64, state: AnimationState, writes: WriteQueue) -> Self {
        Self {
            id,
            state,
            writes,
            graph_changed: Cell::new(false),
        }
    }

    fn push(&self, command: AnimationCommand) {
        self.writes.borrow_mut().animations.push((self.id, command));
    }

    /// The running graph's parameter `name` (`None` while unchecked).
    fn param(&self, name: &str, what: &str) -> PyResult<Option<&GraphParam>> {
        if self.graph_changed.get() {
            return Ok(None);
        }
        let graph = self.need_graph(what)?;
        match graph.params.iter().find(|p| p.name == name) {
            Some(p) => Ok(Some(p)),
            None => Err(PyKeyError::new_err(format!(
                "the graph has no parameter {name:?}; it has: {}",
                names(graph.params.iter().map(|p| p.name.as_str()))
            ))),
        }
    }

    /// The running graph's layer `layer` (the first for `None`; `None`
    /// while unchecked).
    fn layer(&self, layer: Option<&str>, what: &str) -> PyResult<Option<&GraphLayerState>> {
        if self.graph_changed.get() {
            return Ok(None);
        }
        let graph = self.need_graph(what)?;
        let found = match layer {
            None => graph.layers.first(),
            Some(name) => graph.layers.iter().find(|l| l.name == name),
        };
        match (found, layer) {
            (Some(l), _) => Ok(Some(l)),
            (None, Some(name)) => Err(PyKeyError::new_err(format!(
                "the graph has no layer {name:?}; it has: {}",
                names(graph.layers.iter().map(|l| l.name.as_str()))
            ))),
            (None, None) => Ok(None),
        }
    }

    fn need_graph(&self, what: &str) -> PyResult<&automata_scene::GraphState> {
        self.state.graph.as_ref().ok_or_else(|| {
            PyRuntimeError::new_err(format!(
                "{what} needs an animation graph; set animation.graph (or the \
                 Animation component's graph) first. If one is set, it didn't \
                 load: the log says why"
            ))
        })
    }
}

/// An IK target from Python: `None`, an entity's name, something with a
/// `name` (an entity view), or a point (three numbers).
fn ik_target(target: Option<&Bound<'_, PyAny>>) -> PyResult<Option<automata_scene::ik::IkTarget>> {
    use automata_scene::ik::IkTarget;
    let Some(target) = target.filter(|t| !t.is_none()) else {
        return Ok(None);
    };
    if let Ok(name) = target.extract::<String>() {
        return Ok(Some(IkTarget::Entity(name)));
    }
    if let Ok((x, y, z)) = target.extract::<(f32, f32, f32)>() {
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            return Err(PyValueError::new_err("the target must be finite"));
        }
        return Ok(Some(IkTarget::Point(glam::Vec3::new(x, y, z))));
    }
    if let Ok(name) = target.getattr("name").and_then(|n| n.extract::<String>()) {
        return Ok(Some(IkTarget::Entity(name)));
    }
    Err(PyTypeError::new_err(
        "the target must be an entity, an entity's name, a point (x, y, z), or None",
    ))
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> String {
    let all: Vec<&str> = it.collect();
    if all.is_empty() {
        "none".into()
    } else {
        all.join(", ")
    }
}

#[pymethods]
impl PyAnimationRef {
    /// Plays `clip`, crossfading from the current pose over `fade`
    /// seconds. Playing the clip that's already playing keeps it going
    /// unless `restart`. `speed` (if given) sets the playback rate.
    /// Raises `KeyError` when the model has no such clip.
    #[pyo3(signature = (clip, r#loop=true, fade=0.0, speed=None, restart=false))]
    fn play(
        &self,
        clip: &str,
        r#loop: bool,
        fade: f32,
        speed: Option<f32>,
        restart: bool,
    ) -> PyResult<()> {
        if self.state.graph.is_some() {
            return self.play_state(clip, None, fade);
        }
        if !self.state.clips.iter().any(|c| c == clip) {
            return Err(PyKeyError::new_err(format!(
                "the model has no clip {clip:?}; it has: {}",
                self.state.clips.join(", ")
            )));
        }
        self.push(AnimationCommand::Play {
            clip: clip.to_string(),
            options: PlayOptions {
                looping: r#loop,
                speed,
                fade: fade.max(0.0),
                restart,
            },
        });
        Ok(())
    }

    /// Turns the head to look at `target` (an entity, its name, or a point;
    /// `None` stops), as far as `weight` (0 to 1). It eases in and out.
    #[pyo3(signature = (target=None, weight=1.0))]
    fn look_at(&self, target: Option<&Bound<'_, PyAny>>, weight: f32) -> PyResult<()> {
        let target = ik_target(target)?;
        self.push(AnimationCommand::LookAt {
            target,
            weight: weight.clamp(0.0, 1.0),
        });
        Ok(())
    }

    /// Reaches the limb ending at `bone` (a hand, a foot) for `target` (an
    /// entity, its name, or a point; `None` stops), as far as `weight`.
    #[pyo3(signature = (bone, target=None, weight=1.0))]
    fn reach(&self, bone: &str, target: Option<&Bound<'_, PyAny>>, weight: f32) -> PyResult<()> {
        let target = ik_target(target)?;
        self.push(AnimationCommand::Reach {
            bone: bone.to_string(),
            target,
            weight: weight.clamp(0.0, 1.0),
        });
        Ok(())
    }

    /// Whether the feet follow the ground (slopes, stairs).
    #[getter]
    fn foot_ik(&self) -> bool {
        self.state.foot_ik
    }

    #[setter]
    fn set_foot_ik(&self, on: bool) {
        self.push(AnimationCommand::FootIk(on));
    }

    /// Whether the head is looking at something.
    #[getter]
    fn looking(&self) -> bool {
        self.state.looking
    }

    /// The bones reaching for something.
    #[getter]
    fn reaching(&self) -> Vec<String> {
        self.state.reaching.clone()
    }

    /// Stops, fading back to the rest pose over `fade` seconds.
    #[pyo3(signature = (fade=0.0))]
    fn stop(&self, fade: f32) {
        self.push(AnimationCommand::Stop(fade.max(0.0)));
    }

    /// The current clip's name, or `None` in the rest pose.
    #[getter]
    fn clip(&self) -> Option<String> {
        self.state.clip.clone()
    }

    /// Seconds into the current clip.
    #[getter]
    fn time(&self) -> f32 {
        self.state.time
    }

    /// Jumps within the current clip.
    #[setter]
    fn set_time(&self, time: f32) {
        self.push(AnimationCommand::Seek(time));
    }

    /// The current clip's length in seconds (0 with no clip).
    #[getter]
    fn duration(&self) -> f32 {
        self.state.duration
    }

    /// Whether time advances (False once a one-shot clip has ended).
    #[getter]
    fn playing(&self) -> bool {
        self.state.playing
    }

    /// Playback rate (1 is normal speed, negative plays backwards).
    #[getter]
    fn speed(&self) -> f32 {
        self.state.speed
    }

    #[setter]
    fn set_speed(&self, speed: f32) {
        self.push(AnimationCommand::Speed(speed));
    }

    /// Whether the current clip loops.
    #[getter]
    fn looping(&self) -> bool {
        self.state.looping
    }

    /// Every clip the model has.
    #[getter]
    fn clips(&self) -> Vec<String> {
        self.state.clips.clone()
    }

    /// Whether the model has spring bones (hair, skirts, tails that swing
    /// on their own; set up in its import settings).
    #[getter]
    fn springs(&self) -> bool {
        self.state.springs
    }

    /// Puts the spring bones back at rest, as if the entity had just
    /// appeared (after moving it somewhere new, say). Big jumps are
    /// noticed on their own; this is for small ones.
    fn reset_springs(&self) {
        self.push(AnimationCommand::ResetSprings);
    }

    /// Whether the clip's travel moves the entity (root motion). Setting
    /// it applies after the update callback.
    #[getter]
    fn root_motion(&self) -> bool {
        self.state.root_motion
    }

    #[setter]
    fn set_root_motion(&self, on: bool) {
        self.push(AnimationCommand::RootMotion(on));
    }

    /// The latest animation events the playhead passed, oldest first:
    /// `(seq, clip, name)`, `seq` counting up per entity. The script
    /// runner turns them into `on_animation_event` calls and channel
    /// messages; most code wants those instead.
    #[getter]
    fn recent_events(&self) -> Vec<(u64, String, String)> {
        self.state
            .events
            .iter()
            .map(|e| (e.seq, e.clip.clone(), e.name.clone()))
            .collect()
    }

    /// A clip's length in seconds (default: the current clip's). Raises
    /// `KeyError` for an unknown clip.
    #[pyo3(signature = (clip=None))]
    fn length(&self, clip: Option<&str>) -> PyResult<f32> {
        let Some(clip) = clip else {
            return Ok(self.state.duration);
        };
        self.state
            .clips
            .iter()
            .position(|c| c == clip)
            .and_then(|i| self.state.lengths.get(i).copied())
            .ok_or_else(|| PyKeyError::new_err(format!("the model has no clip {clip:?}")))
    }

    /// The animation graph (a `.anim.ron` path) picking the clips, or
    /// `None`. Set a path to run one, `None` to go back to clips.
    #[getter]
    fn graph(&self) -> Option<String> {
        self.state.graph_path.clone()
    }

    #[setter]
    fn set_graph(&self, path: Option<String>) {
        self.graph_changed.set(true);
        self.push(AnimationCommand::Graph(path.unwrap_or_default()));
    }

    /// Sets a graph parameter: a number for a float or an int, True or
    /// False for a bool. Raises `KeyError` for an unknown name and
    /// `TypeError` for a trigger (use `trigger`).
    fn set(&self, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let param = self.param(name, "set()")?;
        if param.is_some_and(|p| p.kind == "trigger") {
            return Err(PyTypeError::new_err(format!(
                "{name:?} is a trigger; use trigger({name:?})"
            )));
        }
        let v = if value.is_instance_of::<PyBool>() {
            f32::from(u8::from(value.extract::<bool>()?))
        } else {
            value.extract::<f32>()?
        };
        if !v.is_finite() {
            return Err(PyValueError::new_err(format!(
                "{name:?} must be a finite number"
            )));
        }
        self.push(AnimationCommand::Set(name.to_string(), v));
        Ok(())
    }

    /// A graph parameter's value at the start of the frame: a float, an
    /// int, or a bool (bools and triggers).
    fn get(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let Some(p) = self.param(name, "get()")? else {
            return Err(PyKeyError::new_err(format!(
                "the new graph's parameters arrive next frame; {name:?} isn't known yet"
            )));
        };
        param_value(py, p)
    }

    /// Sets a trigger: the next transition that tests it fires (and clears
    /// it). Raises `KeyError` for an unknown name and `TypeError` for a
    /// number.
    fn trigger(&self, name: &str) -> PyResult<()> {
        if let Some(p) = self.param(name, "trigger()")? {
            if p.kind != "trigger" && p.kind != "bool" {
                return Err(PyTypeError::new_err(format!(
                    "{name:?} is a {}; use set({name:?}, value)",
                    p.kind
                )));
            }
        }
        self.push(AnimationCommand::Trigger(name.to_string()));
        Ok(())
    }

    /// Clears a trigger no transition has used yet.
    fn reset_trigger(&self, name: &str) -> PyResult<()> {
        if let Some(p) = self.param(name, "reset_trigger()")? {
            if p.kind != "trigger" {
                return Err(PyTypeError::new_err(format!(
                    "{name:?} is a {}, not a trigger",
                    p.kind
                )));
            }
        }
        self.push(AnimationCommand::ResetTrigger(name.to_string()));
        Ok(())
    }

    /// Every graph parameter and its value (empty without a graph).
    #[getter]
    fn parameters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for p in self.state.graph.iter().flat_map(|g| &g.params) {
            dict.set_item(&p.name, param_value(py, p)?)?;
        }
        Ok(dict)
    }

    /// The state the graph's first layer is in (or fading to), or `None`
    /// without a graph.
    #[getter]
    fn state(&self) -> Option<String> {
        self.state
            .graph
            .as_ref()
            .and_then(|g| g.layers.first())
            .map(|l| l.state.clone())
    }

    /// How far through its motion the first layer's state is, 0 to 1.
    #[getter]
    fn state_time(&self) -> f32 {
        self.state
            .graph
            .as_ref()
            .and_then(|g| g.layers.first())
            .map_or(0.0, |l| l.time)
    }

    /// Whether the first layer is crossfading between states.
    #[getter]
    fn in_transition(&self) -> bool {
        self.state
            .graph
            .as_ref()
            .and_then(|g| g.layers.first())
            .is_some_and(|l| l.from.is_some())
    }

    /// The graph's layer names, bottom first (empty without a graph).
    #[getter]
    fn layers(&self) -> Vec<String> {
        self.state
            .graph
            .iter()
            .flat_map(|g| g.layers.iter().map(|l| l.name.clone()))
            .collect()
    }

    /// The state a graph layer is in. Raises `KeyError` for an unknown
    /// layer.
    fn layer_state(&self, layer: &str) -> PyResult<String> {
        self.layer(Some(layer), "layer_state()")?
            .map(|l| l.state.clone())
            .ok_or_else(|| PyKeyError::new_err(format!("{layer:?} isn't known yet")))
    }

    /// How strongly a graph layer shows over the ones below, 0 to 1.
    fn layer_weight(&self, layer: &str) -> PyResult<f32> {
        Ok(self
            .layer(Some(layer), "layer_weight()")?
            .map_or(1.0, |l| l.weight))
    }

    /// Fades a graph layer in or out (0 hides it, 1 shows it fully).
    fn set_layer_weight(&self, layer: &str, weight: f32) -> PyResult<()> {
        self.layer(Some(layer), "set_layer_weight()")?;
        if !weight.is_finite() {
            return Err(PyValueError::new_err("weight must be a finite number"));
        }
        self.push(AnimationCommand::LayerWeight(
            layer.to_string(),
            weight.clamp(0.0, 1.0),
        ));
        Ok(())
    }

    /// Sends a graph layer (default: the first) straight to `state`,
    /// crossfading over `fade` seconds, whatever its transitions say.
    /// Raises `KeyError` for an unknown layer or state.
    #[pyo3(signature = (state, layer=None, fade=0.0))]
    fn play_state(&self, state: &str, layer: Option<&str>, fade: f32) -> PyResult<()> {
        if let Some(l) = self.layer(layer, "play_state()")? {
            if !l.states.iter().any(|s| s == state) {
                return Err(PyKeyError::new_err(format!(
                    "the layer {:?} has no state {state:?}; it has: {}",
                    l.name,
                    names(l.states.iter().map(String::as_str))
                )));
            }
        }
        self.push(AnimationCommand::PlayState {
            layer: layer.unwrap_or_default().to_string(),
            state: state.to_string(),
            fade: if fade.is_finite() { fade.max(0.0) } else { 0.0 },
        });
        Ok(())
    }

    /// The graph's layers as `(name, state, time, from, fade, weight,
    /// states)` rows, for the script runner.
    #[getter(_graph_layers)]
    #[allow(clippy::type_complexity)] // a plain row for Python
    fn graph_layer_rows(
        &self,
    ) -> Vec<(String, String, f32, Option<String>, f32, f32, Vec<String>)> {
        self.state
            .graph
            .iter()
            .flat_map(|g| &g.layers)
            .map(|l| {
                (
                    l.name.clone(),
                    l.state.clone(),
                    l.time,
                    l.from.clone(),
                    l.fade,
                    l.weight,
                    l.states.clone(),
                )
            })
            .collect()
    }

    /// The graph's parameters as `(name, kind, value)` rows, for the
    /// script runner.
    #[getter(_graph_params)]
    fn graph_param_rows(&self) -> Vec<(String, String, f32)> {
        self.state
            .graph
            .iter()
            .flat_map(|g| &g.params)
            .map(|p| (p.name.clone(), p.kind.to_string(), p.value))
            .collect()
    }

    fn __repr__(&self) -> String {
        if let (Some(path), Some(state)) = (&self.state.graph_path, self.state()) {
            return format!("AnimationRef(graph={path:?}, state={state:?})");
        }
        format!(
            "AnimationRef(clip={}, time={:.2}, playing={})",
            self.state
                .clip
                .as_ref()
                .map_or_else(|| "None".to_string(), |c| format!("{c:?}")),
            self.state.time,
            if self.state.playing { "True" } else { "False" }
        )
    }
}

/// A parameter's value as Python sees it: bool, int or float.
fn param_value(py: Python<'_>, p: &GraphParam) -> PyResult<Py<PyAny>> {
    Ok(match p.kind {
        "bool" | "trigger" => (p.value >= 0.5)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind(),
        #[allow(clippy::cast_possible_truncation)] // small ints, stored exactly
        "int" => (p.value.round() as i64)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        _ => p.value.into_pyobject(py)?.into_any().unbind(),
    })
}
