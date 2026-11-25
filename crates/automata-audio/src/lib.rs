//! AUTOMATA audio: mixing and playback via [kira](https://github.com/tesselode/kira).
//!
//! One [`AudioEngine`] owns the kira [`AudioManager`]. Games call
//! [`AudioEngine::play`] to start a sound and receive an id they can use to
//! control it (pause, resume, stop, fade, seek). The engine picks its backend
//! at compile time via the `cpal` cargo feature:
//!
//! - **`cpal` on** — real device output through `cpal` (ALSA on Linux). This is
//!   the engine's `automata-audio/cpal` opt-in feature.
//! - **`cpal` off** (the default) — a *silent* backend: no audio hardware is
//!   touched and no sound is produced, but playback positions and states still
//!   advance deterministically. This is what headless runs, CI, and machines
//!   with no output device use, so game code paths stay identical everywhere.
//!
//! The silent backend is pumped by the frame loop
//! ([`AudioEngine::pump_frame`]), keeping playback queries deterministic
//! without an audio thread.
//!
//! Volume is expressed as a linear amplitude (`1.0` = unity gain, `0.0` =
//! silence) at the engine boundary and converted to decibels internally.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::PlaybackState;
use kira::track::MainTrackBuilder;
use kira::{AudioManager, AudioManagerSettings, Decibels, Mix, Tween};

/// Errors raised by the audio engine.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The audio file could not be read or decoded.
    #[error("could not load audio file {path:?}: {source}")]
    Load {
        /// The path that failed to load.
        path: String,
        /// The underlying decoder/IO error.
        source: kira::sound::FromFileError,
    },
    /// The sound could not be started (the sound capacity was reached).
    #[error("could not play the sound: {0}")]
    Play(String),
    /// A sound id was not found (it may have already been stopped).
    #[error("no such sound id: {0}")]
    UnknownSound(u64),
}

/// What a sound file contains, for editors (see [`probe`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ClipInfo {
    /// Length in seconds.
    pub duration: f64,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Peak level (0..=1) of each of `PEAK_BUCKETS` equal slices of the
    /// clip: a waveform outline.
    pub peaks: Vec<f32>,
}

/// How many slices [`ClipInfo::peaks`] has.
pub const PEAK_BUCKETS: usize = 96;

/// Decodes a sound file. (kira's `from_file` doesn't exist on wasm32,
/// though the browser build has a file system: Pyodide's.)
fn load_file(path: &str) -> Result<StaticSoundData, kira::sound::FromFileError> {
    StaticSoundData::from_media_source(std::fs::File::open(path)?)
}

/// Decodes a sound file and summarizes it (length, rate, waveform).
///
/// # Errors
///
/// [`AudioError::Load`] if the file cannot be read or decoded.
pub fn probe(path: &str) -> Result<ClipInfo, AudioError> {
    let data = load_file(path).map_err(|source| AudioError::Load {
        path: path.to_owned(),
        source,
    })?;
    let frames = &data.frames;
    let mut peaks = vec![0.0_f32; PEAK_BUCKETS];
    if !frames.is_empty() {
        let per = frames.len().div_ceil(PEAK_BUCKETS).max(1);
        for (bucket, chunk) in frames.chunks(per).enumerate().take(PEAK_BUCKETS) {
            peaks[bucket] = chunk
                .iter()
                .map(|f| f.left.abs().max(f.right.abs()))
                .fold(0.0, f32::max)
                .min(1.0);
        }
    }
    Ok(ClipInfo {
        duration: data.duration().as_secs_f64(),
        sample_rate: data.sample_rate,
        peaks,
    })
}

/// How a sound is currently playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundStatus {
    /// Playing (or fading in/out but still advancing).
    Playing,
    /// Paused.
    Paused,
    /// Stopped (terminal; the id cannot be reused).
    Stopped,
}

impl From<PlaybackState> for SoundStatus {
    fn from(state: PlaybackState) -> Self {
        match state {
            PlaybackState::Playing
            | PlaybackState::Pausing
            | PlaybackState::Resuming
            | PlaybackState::WaitingToResume => Self::Playing,
            PlaybackState::Paused => Self::Paused,
            PlaybackState::Stopping | PlaybackState::Stopped => Self::Stopped,
        }
    }
}

/// Converts a linear amplitude (0.0..) to kira decibels.
#[allow(clippy::cast_possible_truncation)] // volume fits comfortably in f32
fn linear_to_decibels(volume: f64) -> Decibels {
    let volume = volume.max(0.0) as f32;
    if volume <= f32::EPSILON {
        return Decibels::SILENCE;
    }
    Decibels(20.0 * volume.log10())
}

/// Clamps a stereo panning value into kira's `-1.0..=1.0` range.
#[allow(clippy::cast_possible_truncation)] // panning fits comfortably in f32
fn clamp_panning(panning: f64) -> f32 {
    (panning as f32).clamp(-1.0, 1.0)
}

fn tween_of(seconds: f64) -> Tween {
    Tween {
        duration: Duration::from_secs_f64(seconds.max(0.0)),
        ..Tween::default()
    }
}

/// Steps the silent backend forward by `delta_seconds` of audio.
fn pump_silent(
    manager: &mut AudioManager<kira::backend::mock::MockBackend>,
    sample_rate: u32,
    delta_seconds: f64,
) {
    if delta_seconds <= 0.0 {
        return;
    }
    let backend = manager.backend_mut();
    // The renderer processes in fixed buffers of `internal_buffer_size` (128
    // by default). Step in whole buffers to cover `delta_seconds`.
    let buffer = 128.0_f64;
    let frames = delta_seconds * f64::from(sample_rate);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // `frames` is positive and far below usize range for any real frame.
    let steps = ((frames / buffer).ceil().max(1.0)) as usize;
    for _ in 0..steps {
        backend.on_start_processing();
        backend.process();
    }
}

/// Options for [`AudioEngine::play`].
#[derive(Debug, Clone, Copy)]
pub struct PlayOptions<'a> {
    /// Path to the audio file.
    pub path: &'a str,
    /// Linear volume (`1.0` = unity).
    pub volume: f64,
    /// Whether to loop the whole sound.
    pub looped: bool,
    /// Playback rate (speed and pitch multiplier; `1.0` = normal).
    pub playback_rate: f64,
    /// Stereo panning in `-1.0..=1.0` (`0.0` = center).
    pub panning: f64,
}

impl<'a> PlayOptions<'a> {
    /// Options that play `path` once at full volume, centered.
    #[must_use]
    pub fn once(path: &'a str) -> Self {
        Self {
            path,
            volume: 1.0,
            looped: false,
            playback_rate: 1.0,
            panning: 0.0,
        }
    }
}

/// The audio system. Owns the kira manager and every live sound.
///
/// The manager is either a real device backend (with the `cpal` feature) or a
/// silent backend. With `cpal` enabled but no output device available (a
/// headless CI box, for example), construction falls back to the silent
/// backend so the engine never panics and game code paths stay identical.
pub struct AudioEngine {
    manager: Manager,
    /// Live sounds keyed by id.
    sounds: HashMap<u64, StaticSoundHandle>,
    /// Next sound id.
    next_id: u64,
    /// Sample rate the silent backend runs at; drives the frame pump.
    sample_rate: u32,
    /// Decoded files, so repeated sounds (footsteps, gunfire) decode once.
    /// Entries are re-read when the file's modification time changes.
    cache: HashMap<PathBuf, (Option<SystemTime>, StaticSoundData)>,
    /// The low-pass filter on everything (the main track), dry while off.
    filter: FilterHandle,
    /// Its cutoff while on (Hz).
    low_pass: Option<f64>,
}

/// The low-pass cutoff (Hz) for hearing from under water: muffled, the
/// highs gone, the thumps and rumbles left.
pub const UNDERWATER_CUTOFF_HZ: f64 = 650.0;

/// A main track with a low-pass filter (off: all dry) and its handle.
fn filtered_main_track() -> (MainTrackBuilder, FilterHandle) {
    let mut track = MainTrackBuilder::new();
    let filter = track.add_effect(FilterBuilder::new().cutoff(20_000.0).mix(Mix::DRY));
    (track, filter)
}

/// The audio manager, over whichever backend is active.
///
/// Both variants are boxed: the managers are large, so boxing keeps the enum
/// small and its variants uniform in size.
enum Manager {
    /// Real device output (only present with the `cpal` feature).
    #[cfg(feature = "cpal")]
    Device(Box<AudioManager<kira::backend::cpal::CpalBackend>>),
    /// Silent backend: no device, deterministic advancement.
    Silent(Box<AudioManager<kira::backend::mock::MockBackend>>),
}

impl fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioEngine")
            .field("sounds", &self.sounds.len())
            .field("sample_rate", &self.sample_rate)
            .field("backend", &self.backend_name())
            .finish_non_exhaustive()
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    /// Creates the audio engine with a default configuration.
    ///
    /// With the `cpal` feature this opens the default output device, falling
    /// back to the silent backend if no device is available; without `cpal` it
    /// starts the silent backend directly at 44 100 Hz.
    #[must_use]
    pub fn new() -> Self {
        Self::with_sample_rate(44_100)
    }

    /// Creates the audio engine, overriding the silent-backend sample rate.
    ///
    /// The sample rate only drives the silent backend's deterministic
    /// advancement; a real device reports its own rate. Never panics: if the
    /// device backend cannot be created the engine silently falls back.
    #[must_use]
    pub fn with_sample_rate(sample_rate: u32) -> Self {
        let (manager, filter) = Self::manager(sample_rate);
        Self {
            manager,
            sounds: HashMap::new(),
            next_id: 1,
            sample_rate,
            cache: HashMap::new(),
            filter,
            low_pass: None,
        }
    }

    /// The active backend name (`"cpal"` or `"silent"`).
    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        match &self.manager {
            #[cfg(feature = "cpal")]
            Manager::Device(_) => "cpal",
            Manager::Silent(_) => "silent",
        }
    }

    /// Builds the manager: real device when possible, silent otherwise.
    fn manager(sample_rate: u32) -> (Manager, FilterHandle) {
        #[cfg(feature = "cpal")]
        {
            let (main_track_builder, filter) = filtered_main_track();
            let settings = AudioManagerSettings {
                main_track_builder,
                ..AudioManagerSettings::default()
            };
            match AudioManager::<kira::backend::cpal::CpalBackend>::new(settings) {
                Ok(manager) => return (Manager::Device(Box::new(manager)), filter),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "no audio output device; falling back to the silent backend"
                    );
                }
            }
        }
        let (manager, filter) = Self::silent_manager(sample_rate);
        (Manager::Silent(manager), filter)
    }

    fn silent_manager(
        sample_rate: u32,
    ) -> (
        Box<AudioManager<kira::backend::mock::MockBackend>>,
        FilterHandle,
    ) {
        let (main_track_builder, filter) = filtered_main_track();
        let settings = AudioManagerSettings {
            backend_settings: kira::backend::mock::MockBackendSettings { sample_rate },
            main_track_builder,
            ..AudioManagerSettings::default()
        };
        // The mock backend cannot fail to initialize.
        let manager = AudioManager::<kira::backend::mock::MockBackend>::new(settings)
            .expect("failed to create the silent audio manager");
        (Box::new(manager), filter)
    }

    /// The silent manager, when the silent backend is active.
    #[cfg(feature = "cpal")]
    fn silent_manager_mut(
        &mut self,
    ) -> Option<&mut AudioManager<kira::backend::mock::MockBackend>> {
        match &mut self.manager {
            Manager::Device(_) => None,
            Manager::Silent(manager) => Some(&mut **manager),
        }
    }

    /// The silent manager (the only backend without the `cpal` feature).
    #[cfg(not(feature = "cpal"))]
    fn silent_manager_mut(&mut self) -> &mut AudioManager<kira::backend::mock::MockBackend> {
        let Manager::Silent(manager) = &mut self.manager;
        manager
    }

    /// Advances the silent backend by one frame's worth of audio.
    ///
    /// Only meaningful on the silent backend; with a real device this is a
    /// no-op. The frame loop calls this once per frame so playback positions
    /// advance deterministically.
    #[cfg(feature = "cpal")]
    pub fn pump_frame(&mut self, delta_seconds: f64) {
        let sample_rate = self.sample_rate;
        let Some(manager) = self.silent_manager_mut() else {
            return; // real device backend: it advances itself
        };
        pump_silent(manager, sample_rate, delta_seconds);
    }

    /// Advances the silent backend by one frame's worth of audio.
    ///
    /// Without the `cpal` feature the silent backend is the only one, so this
    /// always pumps. The frame loop calls this once per frame so playback
    /// positions advance deterministically.
    #[cfg(not(feature = "cpal"))]
    pub fn pump_frame(&mut self, delta_seconds: f64) {
        let sample_rate = self.sample_rate;
        pump_silent(self.silent_manager_mut(), sample_rate, delta_seconds);
    }

    /// Loads a sound file and starts playing it.
    ///
    /// Returns the id of the new sound for later control.
    ///
    /// # Errors
    ///
    /// [`AudioError::Load`] if the file cannot be read or decoded, or
    /// [`AudioError::Play`] if the sound capacity was reached.
    pub fn play(&mut self, options: PlayOptions<'_>) -> Result<u64, AudioError> {
        let PlayOptions {
            path,
            volume,
            looped,
            playback_rate,
            panning,
        } = options;
        // Finished sounds are forgotten, so one-shots don't pile up.
        self.sounds
            .retain(|_, h| h.state() != PlaybackState::Stopped);
        let mut data = self
            .load(path)?
            .volume(linear_to_decibels(volume))
            .playback_rate(playback_rate)
            .panning(clamp_panning(panning));
        if looped {
            data = data.loop_region(..);
        }
        let handle = match &mut self.manager {
            #[cfg(feature = "cpal")]
            Manager::Device(manager) => manager.play(data),
            Manager::Silent(manager) => manager.play(data),
        }
        .map_err(|err| AudioError::Play(format!("{err}")))?;
        let id = self.next_id;
        self.next_id += 1;
        self.sounds.insert(id, handle);
        Ok(id)
    }

    /// Decodes `path` (or reuses the cached decode while the file is
    /// unchanged).
    fn load(&mut self, path: &str) -> Result<StaticSoundData, AudioError> {
        let key = PathBuf::from(path);
        let stamp = std::fs::metadata(&key).and_then(|m| m.modified()).ok();
        if let Some((cached, data)) = self.cache.get(&key) {
            if *cached == stamp {
                return Ok(data.clone());
            }
        }
        let data = load_file(path).map_err(|source| AudioError::Load {
            path: path.to_owned(),
            source,
        })?;
        self.cache.insert(key, (stamp, data.clone()));
        Ok(data)
    }

    /// The length of a sound file in seconds (decoding it into the cache).
    ///
    /// # Errors
    ///
    /// [`AudioError::Load`] if the file cannot be read or decoded.
    pub fn duration(&mut self, path: &str) -> Result<f64, AudioError> {
        Ok(self.load(path)?.duration().as_secs_f64())
    }

    /// Returns a controller for a live sound.
    ///
    /// # Errors
    ///
    /// [`AudioError::UnknownSound`] if `id` is not a live sound.
    pub fn sound(&mut self, id: u64) -> Result<Sound<'_>, AudioError> {
        let handle = self
            .sounds
            .get_mut(&id)
            .ok_or(AudioError::UnknownSound(id))?;
        Ok(Sound { handle })
    }

    /// The status of a sound, or `None` if `id` is unknown.
    #[must_use]
    pub fn status(&self, id: u64) -> Option<SoundStatus> {
        self.sounds.get(&id).map(|h| SoundStatus::from(h.state()))
    }

    /// The playback position (seconds) of a sound, or `None` if unknown.
    #[must_use]
    pub fn position(&self, id: u64) -> Option<f64> {
        self.sounds.get(&id).map(StaticSoundHandle::position)
    }

    /// Stops and forgets a sound. Unknown ids are ignored.
    pub fn stop(&mut self, id: u64, fade_out_seconds: f64) {
        if let Some(mut handle) = self.sounds.remove(&id) {
            handle.stop(tween_of(fade_out_seconds));
        }
    }

    /// Sets the master volume (linear amplitude; `1.0` = unity).
    pub fn set_master_volume(&mut self, volume: f64, fade_seconds: f64) {
        let db = linear_to_decibels(volume);
        let tween = tween_of(fade_seconds);
        match &mut self.manager {
            #[cfg(feature = "cpal")]
            Manager::Device(manager) => manager.main_track().set_volume(db, tween),
            Manager::Silent(manager) => manager.main_track().set_volume(db, tween),
        }
    }

    /// Muffles everything through a low-pass filter at `cutoff_hz` (20 to
    /// 20 000), or lifts it (`None`), over `fade_seconds`.
    pub fn set_low_pass(&mut self, cutoff_hz: Option<f64>, fade_seconds: f64) {
        let tween = tween_of(fade_seconds);
        if let Some(hz) = cutoff_hz {
            let hz = if hz.is_finite() {
                hz.clamp(20.0, 20_000.0)
            } else {
                20_000.0
            };
            self.filter.set_cutoff(hz, tween);
            self.filter.set_mix(Mix::WET, tween);
            self.low_pass = Some(hz);
        } else {
            self.filter.set_mix(Mix::DRY, tween);
            self.filter.set_cutoff(20_000.0, tween);
            self.low_pass = None;
        }
    }

    /// The low-pass cutoff in Hz while everything is muffled, else `None`.
    #[must_use]
    pub fn low_pass(&self) -> Option<f64> {
        self.low_pass
    }

    /// Hears from under water (a quick fade to [`UNDERWATER_CUTOFF_HZ`]) or
    /// not. The engine calls it each frame with whether its 3D camera is
    /// under water; a no-op when nothing changes.
    pub fn set_underwater(&mut self, under: bool) {
        let muffled = self.low_pass == Some(UNDERWATER_CUTOFF_HZ);
        if under && !muffled {
            self.set_low_pass(Some(UNDERWATER_CUTOFF_HZ), 0.15);
        } else if !under && muffled {
            self.set_low_pass(None, 0.25);
        }
    }

    /// Whether [`Self::set_underwater`] has the sound muffled.
    #[must_use]
    pub fn is_underwater(&self) -> bool {
        self.low_pass == Some(UNDERWATER_CUTOFF_HZ)
    }

    /// Stops every live sound and clears the registry.
    pub fn stop_all(&mut self, fade_out_seconds: f64) {
        let tween = tween_of(fade_out_seconds);
        for (_, mut handle) in self.sounds.drain() {
            handle.stop(tween);
        }
    }

    /// Number of live sounds (playing or paused).
    #[must_use]
    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }
}

/// A controller for one live sound, borrowed from the [`AudioEngine`].
pub struct Sound<'a> {
    handle: &'a mut StaticSoundHandle,
}

impl Sound<'_> {
    /// Pauses the sound.
    pub fn pause(&mut self, fade_seconds: f64) {
        self.handle.pause(tween_of(fade_seconds));
    }

    /// Resumes a paused sound.
    pub fn resume(&mut self, fade_seconds: f64) {
        self.handle.resume(tween_of(fade_seconds));
    }

    /// Seeks to `position` seconds.
    pub fn seek_to(&mut self, position: f64) {
        self.handle.seek_to(position);
    }

    /// Sets the sound's volume (linear amplitude).
    pub fn set_volume(&mut self, volume: f64, fade_seconds: f64) {
        self.handle
            .set_volume(linear_to_decibels(volume), tween_of(fade_seconds));
    }

    /// Sets the playback rate (speed and pitch multiplier).
    pub fn set_playback_rate(&mut self, rate: f64, fade_seconds: f64) {
        self.handle.set_playback_rate(rate, tween_of(fade_seconds));
    }

    /// Sets stereo panning in `-1.0..=1.0`.
    pub fn set_panning(&mut self, panning: f64, fade_seconds: f64) {
        self.handle
            .set_panning(clamp_panning(panning), tween_of(fade_seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a minimal 44 100 Hz mono PCM WAV of a sine tone.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn write_wav(path: &std::path::Path, seconds: f64, freq: f64) {
        let sample_rate = 44_100u32;
        let n = (seconds * f64::from(sample_rate)) as usize;
        let mut data = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / f64::from(sample_rate);
            let sample = (2.0 * std::f64::consts::PI * freq * t).sin();
            let s = (sample * f64::from(i16::MAX)) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        let data_len = data.len() as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(path, wav).unwrap();
    }

    fn engine() -> AudioEngine {
        AudioEngine::with_sample_rate(44_100)
    }

    #[test]
    fn play_assigns_incrementing_ids() {
        let dir = std::env::temp_dir();
        let path = dir.join("automata_audio_ids.wav");
        write_wav(&path, 0.1, 440.0);
        let mut engine = engine();
        let a = engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        let b = engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(engine.sound_count(), 2);
    }

    #[test]
    fn missing_file_errors() {
        let mut engine = engine();
        let err = engine
            .play(PlayOptions::once("definitely/not/a/real/file.wav"))
            .unwrap_err();
        assert!(matches!(err, AudioError::Load { .. }));
    }

    #[test]
    fn silent_backend_advances_position() {
        let dir = std::env::temp_dir();
        let path = dir.join("automata_audio_advance.wav");
        write_wav(&path, 1.0, 440.0);
        let mut engine = engine();
        let id = engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        assert_eq!(engine.status(id), Some(SoundStatus::Playing));
        // Pump ~0.5 s of audio; the position should move forward.
        for _ in 0..30 {
            engine.pump_frame(1.0 / 60.0);
        }
        let pos = engine.position(id).unwrap();
        assert!(pos > 0.05, "expected position to advance, got {pos}");
    }

    #[test]
    fn stop_and_unknown_sound() {
        let dir = std::env::temp_dir();
        let path = dir.join("automata_audio_stop.wav");
        write_wav(&path, 0.5, 440.0);
        let mut engine = engine();
        let id = engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        engine.stop(id, 0.0);
        assert_eq!(engine.status(id), None);
        assert!(engine.sound(id).is_err());
        // Stopping an unknown id is a no-op.
        engine.stop(999, 0.0);
    }

    #[test]
    fn pause_and_resume() {
        let dir = std::env::temp_dir();
        let path = dir.join("automata_audio_pause.wav");
        write_wav(&path, 1.0, 440.0);
        let mut engine = engine();
        let id = engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        engine.sound(id).unwrap().pause(0.0);
        engine.pump_frame(1.0 / 60.0);
        assert_eq!(engine.status(id), Some(SoundStatus::Paused));
        engine.sound(id).unwrap().resume(0.0);
        engine.pump_frame(1.0 / 60.0);
        assert_eq!(engine.status(id), Some(SoundStatus::Playing));
    }

    #[test]
    fn stop_all_clears_registry() {
        let dir = std::env::temp_dir();
        let path = dir.join("automata_audio_stop_all.wav");
        write_wav(&path, 0.5, 440.0);
        let mut engine = engine();
        engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        engine
            .play(PlayOptions::once(path.to_str().unwrap()))
            .unwrap();
        assert_eq!(engine.sound_count(), 2);
        engine.stop_all(0.0);
        assert_eq!(engine.sound_count(), 0);
    }

    #[test]
    fn low_pass_muffles_and_lifts() {
        let mut audio = engine();
        assert_eq!(audio.low_pass(), None);
        audio.set_low_pass(Some(1000.0), 0.0);
        assert_eq!(audio.low_pass(), Some(1000.0));
        audio.set_low_pass(Some(5.0), 0.0);
        assert_eq!(audio.low_pass(), Some(20.0), "clamped to what's audible");
        audio.set_low_pass(None, 0.1);
        assert_eq!(audio.low_pass(), None);
        audio.pump_frame(0.2);
    }

    #[test]
    fn underwater_muffles_until_surfacing() {
        let mut audio = engine();
        audio.set_underwater(true);
        assert!(audio.is_underwater());
        assert_eq!(audio.low_pass(), Some(UNDERWATER_CUTOFF_HZ));
        audio.set_underwater(true); // no change
        audio.set_underwater(false);
        assert!(!audio.is_underwater());
        assert_eq!(audio.low_pass(), None);
        // A game's own low-pass isn't undone by staying dry.
        audio.set_low_pass(Some(3000.0), 0.0);
        audio.set_underwater(false);
        assert_eq!(audio.low_pass(), Some(3000.0));
    }
}
