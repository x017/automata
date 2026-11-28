//! Python bindings for audio: playing and controlling sounds.
//!
//! `ctx.audio` shares the engine's [`AudioEngine`] through an
//! `Rc<RefCell<..>>` (the same pattern as `ctx.resources`), so operations
//! apply immediately and queries observe live state — including within the
//! frame that issued them. Audio is independent of the scene graph, so
//! mutating it during the update callback cannot invalidate any snapshot the
//! callback holds. Sounds are identified by the integer id returned from
//! [`play`](PyAudio::play).
//!
//! The active backend is chosen at build time. In default builds the engine
//! runs a silent backend (no device, no ALSA) that still advances playback
//! positions deterministically; `is_playing`, `position`, and friends work
//! identically there, which is what the headless test-suite and CI exercise.
//! The `audio-cpal` feature switches to real device output.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use automata_core::AudioCommand;

use automata_audio::{AudioEngine, PlayOptions};
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;

/// Shared ownership of the engine's audio system between `WorldState` and
/// the per-frame `ctx.audio` view.
pub(crate) type SharedAudio = Rc<RefCell<AudioEngine>>;

/// Per-frame audio control, obtained via `ctx.audio`.
///
/// All methods act on the live audio engine immediately. `play` returns a
/// sound id usable with the other methods for the rest of the sound's life.
#[pyclass(name = "Audio", module = "automata._core", unsendable)]
#[derive(Clone)]
pub struct PyAudio {
    engine: SharedAudio,
    /// Requests for the scene's audio-source player, applied after the
    /// callback.
    commands: Rc<RefCell<Vec<AudioCommand>>>,
    /// Entities (bits) whose source was playing at the start of the frame.
    playing: Rc<HashSet<u64>>,
}

/// Queued audio-source requests, shared with the game loop.
pub(crate) type AudioCommands = Rc<RefCell<Vec<AudioCommand>>>;

impl std::fmt::Debug for PyAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio").finish_non_exhaustive()
    }
}

impl PyAudio {
    /// Binds the view with the frame's source-command queue and the
    /// entities whose sources are playing.
    pub(crate) fn for_frame(
        engine: SharedAudio,
        commands: AudioCommands,
        playing: HashSet<u64>,
    ) -> Self {
        Self {
            engine,
            commands,
            playing: Rc::new(playing),
        }
    }
}

fn entity_of(obj: &Bound<'_, PyAny>) -> PyResult<automata_scene::Entity> {
    if let Ok(bits) = obj.extract::<u64>() {
        return crate::scene::entity_from_bits(bits);
    }
    if let Ok(view) = obj.cast::<crate::scene::EntityView>() {
        return crate::scene::entity_from_bits(view.borrow().id_bits());
    }
    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "expected an entity id (int) or EntityView, got {}",
        obj.get_type().name()?
    )))
}

/// Maps an audio error to the right Python exception.
fn audio_err(err: &automata_audio::AudioError) -> PyErr {
    let message = err.to_string();
    match err {
        automata_audio::AudioError::Load { .. } => PyValueError::new_err(message),
        automata_audio::AudioError::Play(_) => crate::error::AutomataError::new_err(message),
        automata_audio::AudioError::UnknownSound(_) => PyKeyError::new_err(message),
    }
}

#[pymethods]
impl PyAudio {
    /// Starts (or restarts) an entity's `AudioSource` after this frame's
    /// callback.
    fn play_source(&self, entity: &Bound<'_, PyAny>) -> PyResult<()> {
        let e = entity_of(entity)?;
        self.commands.borrow_mut().push(AudioCommand::Play(e));
        Ok(())
    }

    /// Stops an entity's `AudioSource` after this frame's callback.
    fn stop_source(&self, entity: &Bound<'_, PyAny>) -> PyResult<()> {
        let e = entity_of(entity)?;
        self.commands.borrow_mut().push(AudioCommand::Stop(e));
        Ok(())
    }

    /// Whether an entity's `AudioSource` was playing at the start of the
    /// frame.
    fn source_playing(&self, entity: &Bound<'_, PyAny>) -> PyResult<bool> {
        let e = entity_of(entity)?;
        Ok(self.playing.contains(&e.to_bits().get()))
    }

    /// Plays a sound file once, resolved like `AudioSource` clips (against
    /// the scene folder). With `position=(x, y, z)` it fades with distance
    /// from the camera. Missing files are logged, not raised.
    #[pyo3(signature = (path, volume=1.0, pitch=1.0, position=None))]
    fn play_at(&self, path: &str, volume: f32, pitch: f32, position: Option<(f32, f32, f32)>) {
        self.commands.borrow_mut().push(AudioCommand::OneShot {
            path: path.replace('\\', "/"),
            volume,
            pitch,
            position: position.map(|(x, y, z)| glam::Vec3::new(x, y, z)),
        });
    }

    /// Plays a sound file and returns its id.
    ///
    /// `volume` is a linear amplitude (`1.0` = full). `loop_` repeats the
    /// whole sound. `playback_rate` scales speed and pitch (`1.0` = normal),
    /// and `panning` is `-1.0` (left) to `1.0` (right).
    ///
    /// Raises `ValueError` if the file cannot be read or decoded.
    #[pyo3(signature = (path, volume=1.0, loop_=false, playback_rate=1.0, panning=0.0))]
    fn play(
        &self,
        path: &str,
        volume: f64,
        loop_: bool,
        playback_rate: f64,
        panning: f64,
    ) -> PyResult<u64> {
        self.engine
            .borrow_mut()
            .play(PlayOptions {
                path,
                volume,
                looped: loop_,
                playback_rate,
                panning,
            })
            .map_err(|e| audio_err(&e))
    }

    /// Stops a sound, fading out over `fade_out` seconds (default 0).
    #[pyo3(signature = (id, fade_out=0.0))]
    fn stop(&self, id: u64, fade_out: f64) {
        self.engine.borrow_mut().stop(id, fade_out);
    }

    /// Stops every live sound, fading out over `fade_out` seconds (default 0).
    #[pyo3(signature = (fade_out=0.0))]
    fn stop_all(&self, fade_out: f64) {
        self.engine.borrow_mut().stop_all(fade_out);
    }

    /// Pauses a sound. Raises `KeyError` for an unknown id.
    #[pyo3(signature = (id, fade=0.0))]
    fn pause(&self, id: u64, fade: f64) -> PyResult<()> {
        self.engine
            .borrow_mut()
            .sound(id)
            .map(|mut s| s.pause(fade))
            .map_err(|e| audio_err(&e))
    }

    /// Resumes a paused sound. Raises `KeyError` for an unknown id.
    #[pyo3(signature = (id, fade=0.0))]
    fn resume(&self, id: u64, fade: f64) -> PyResult<()> {
        self.engine
            .borrow_mut()
            .sound(id)
            .map(|mut s| s.resume(fade))
            .map_err(|e| audio_err(&e))
    }

    /// Whether the sound is currently playing. `False` for an unknown id.
    fn is_playing(&self, id: u64) -> bool {
        self.engine.borrow().status(id) == Some(automata_audio::SoundStatus::Playing)
    }

    /// Playback position in seconds, or `None` for an unknown id.
    fn position(&self, id: u64) -> Option<f64> {
        self.engine.borrow().position(id)
    }

    /// Seeks to `position` seconds. Raises `KeyError` for an unknown id.
    fn seek_to(&self, id: u64, position: f64) -> PyResult<()> {
        self.engine
            .borrow_mut()
            .sound(id)
            .map(|mut s| s.seek_to(position))
            .map_err(|e| audio_err(&e))
    }

    /// Sets a sound's volume (linear amplitude). Raises `KeyError` for an
    /// unknown id.
    #[pyo3(signature = (id, volume, fade=0.0))]
    fn set_volume(&self, id: u64, volume: f64, fade: f64) -> PyResult<()> {
        self.engine
            .borrow_mut()
            .sound(id)
            .map(|mut s| s.set_volume(volume, fade))
            .map_err(|e| audio_err(&e))
    }

    /// Sets a sound's playback rate. Raises `KeyError` for an unknown id.
    #[pyo3(signature = (id, rate, fade=0.0))]
    fn set_playback_rate(&self, id: u64, rate: f64, fade: f64) -> PyResult<()> {
        self.engine
            .borrow_mut()
            .sound(id)
            .map(|mut s| s.set_playback_rate(rate, fade))
            .map_err(|e| audio_err(&e))
    }

    /// Sets a sound's stereo panning in `-1.0..=1.0`. Raises `KeyError` for
    /// an unknown id.
    #[pyo3(signature = (id, panning, fade=0.0))]
    fn set_panning(&self, id: u64, panning: f64, fade: f64) -> PyResult<()> {
        self.engine
            .borrow_mut()
            .sound(id)
            .map(|mut s| s.set_panning(panning, fade))
            .map_err(|e| audio_err(&e))
    }

    /// Sets the master volume (linear amplitude), fading over `fade` seconds.
    #[pyo3(signature = (volume, fade=0.0))]
    fn set_master_volume(&self, volume: f64, fade: f64) {
        self.engine.borrow_mut().set_master_volume(volume, fade);
    }

    /// The number of live sounds (playing or paused).
    fn sound_count(&self) -> usize {
        self.engine.borrow().sound_count()
    }

    /// Muffles every sound through a low-pass filter at `cutoff_hz` (20 to
    /// 20 000), or lifts it (`None`), fading over `fade` seconds. The
    /// engine muffles everything by itself while the camera is under
    /// water.
    #[pyo3(signature = (cutoff_hz=None, fade=0.0))]
    fn set_low_pass(&self, cutoff_hz: Option<f64>, fade: f64) {
        self.engine.borrow_mut().set_low_pass(cutoff_hz, fade);
    }

    /// The low-pass cutoff (Hz) while sound is muffled, else None.
    #[getter]
    fn low_pass(&self) -> Option<f64> {
        self.engine.borrow().low_pass()
    }

    /// True while the camera is under water (and sound muffled).
    #[getter]
    fn underwater(&self) -> bool {
        self.engine.borrow().is_underwater()
    }
}
