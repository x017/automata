//! `at.Animation` (the component) and `AnimationRef` (a running entity's
//! playback, `view.animation`).

use automata_scene::{Animation, AnimationCommand, AnimationState, PlayOptions};
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;

use crate::scene::WriteQueue;

/// Which clip an animated model plays: what Oberon's Inspector calls an
/// Animation. The model is the entity's `MeshRenderer.model(...)`; `clip`
/// names one of its animations (empty: the rest pose). `play_on_start`
/// plays it as the scene starts; off, the model holds the clip's first
/// frame until a script plays something.
///
/// ```python
/// robot = at.Entity("Robot").add(at.Transform()).add(
///     at.MeshRenderer.model("models/robot.glb")).add(at.Animation("Idle"))
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
    #[pyo3(signature = (clip="", speed=1.0, looping=true, play_on_start=true))]
    fn new(clip: &str, speed: f32, looping: bool, play_on_start: bool) -> PyResult<Self> {
        if !speed.is_finite() {
            return Err(PyValueError::new_err("speed must be a finite number"));
        }
        Ok(Self {
            inner: Animation {
                clip: clip.to_string(),
                speed,
                looping,
                play_on_start,
            },
        })
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
            "Animation({:?}, speed={}, looping={}, play_on_start={})",
            self.inner.clip,
            self.inner.speed,
            py_bool(self.inner.looping),
            py_bool(self.inner.play_on_start),
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

/// A running entity's animation (`view.animation`). Reads show the start
/// of the frame; `play`, `stop`, and setting `time` or `speed` apply after
/// the update callback returns.
#[pyclass(name = "AnimationRef", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct PyAnimationRef {
    id: u64,
    state: AnimationState,
    writes: WriteQueue,
}

impl PyAnimationRef {
    pub(crate) fn new(id: u64, state: AnimationState, writes: WriteQueue) -> Self {
        Self { id, state, writes }
    }

    fn push(&self, command: AnimationCommand) {
        self.writes.borrow_mut().animations.push((self.id, command));
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

    fn __repr__(&self) -> String {
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
