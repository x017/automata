//! Sounds attached to entities: the [`AudioSource`] component and the
//! [`AudioSourcePlayer`] that plays them.
//!
//! An `AudioSource` names a sound file (`.ogg`, `.wav`, `.mp3`, `.flac`)
//! and how to play it. Scene files store it like any component; Oberon
//! edits it in the Inspector. At runtime one `AudioSourcePlayer` per running
//! scene:
//!
//! - starts every source marked `play_on_start` on its first update,
//! - plays and stops sources on request ([`AudioCommand`]: what scripts'
//!   `entity.audio.play()` turns into), and fires one-shot sounds,
//! - makes `spatial` sources quieter with distance from the listener (the
//!   scene camera) and pans them left or right, every update,
//! - stops a looping source when its entity is despawned.
//!
//! Relative clip paths resolve against a root folder (the scene's folder).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use automata_audio::{AudioEngine, PlayOptions, SoundStatus};
use automata_renderer::Camera3D;
use automata_scene::{Entity, GlobalTransform, SceneWorld, Transform};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// File extensions the audio engine decodes (lowercase, without the dot).
pub const AUDIO_EXTENSIONS: [&str; 4] = ["ogg", "wav", "mp3", "flac"];

/// Whether `path` names a sound file the engine can play.
#[must_use]
pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A sound attached to an entity (also its ECS component).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSource {
    /// The sound file, `/`-separated and relative to the scene's folder.
    pub clip: String,
    /// Linear volume (`1.0` = as recorded).
    pub volume: f32,
    /// Speed and pitch multiplier (`1.0` = normal).
    pub pitch: f32,
    /// Start over when the end is reached.
    pub looping: bool,
    /// Start playing as soon as the scene runs.
    pub play_on_start: bool,
    /// Fade with distance from the camera and pan left/right.
    pub spatial: bool,
    /// For spatial sources: the distance (metres) at which it falls silent.
    pub range: f32,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self {
            clip: String::new(),
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            play_on_start: false,
            spatial: false,
            range: 20.0,
        }
    }
}

impl AudioSource {
    /// A source playing `clip` with default settings.
    #[must_use]
    pub fn new(clip: impl Into<String>) -> Self {
        Self {
            clip: clip.into(),
            ..Self::default()
        }
    }
}

/// A request to the [`AudioSourcePlayer`].
#[derive(Debug, Clone, PartialEq)]
pub enum AudioCommand {
    /// Start (or restart) an entity's [`AudioSource`].
    Play(Entity),
    /// Stop an entity's source.
    Stop(Entity),
    /// Fire and forget a sound file, optionally at a world position (then
    /// it fades with distance like a spatial source of `range` 30 m).
    OneShot {
        /// The sound file (relative paths resolve against the root).
        path: String,
        /// Linear volume.
        volume: f32,
        /// Speed and pitch multiplier.
        pitch: f32,
        /// Where it happens, for distance fading.
        position: Option<Vec3>,
    },
}

/// The range one-shots at a position fade over.
const ONE_SHOT_RANGE: f32 = 30.0;

/// One playing sound.
#[derive(Debug, Clone)]
struct Voice {
    id: u64,
    /// The entity whose source this is (`None` for one-shots).
    entity: Option<Entity>,
    volume: f32,
    /// `Some((position, range))` for distance-faded sounds; the position
    /// follows the entity.
    spatial: Option<(Vec3, f32)>,
}

/// Plays the [`AudioSource`]s of one running scene. See the module docs.
#[derive(Debug, Default)]
pub struct AudioSourcePlayer {
    started: bool,
    voices: Vec<Voice>,
    /// Clips that failed to load, reported once each.
    failed: HashSet<String>,
}

/// Where a listener is and which way is right.
#[derive(Debug, Clone, Copy)]
struct Listener {
    position: Vec3,
    right: Vec3,
}

impl Listener {
    fn of(camera: &Camera3D) -> Self {
        let forward = (camera.target - camera.position).normalize_or(Vec3::NEG_Z);
        let right = forward.cross(camera.up).normalize_or(Vec3::X);
        Self {
            position: camera.position,
            right,
        }
    }

    /// `(gain, pan)` for a sound at `at` that falls silent at `range`.
    fn mix(&self, at: Vec3, range: f32) -> (f32, f32) {
        let offset = at - self.position;
        let distance = offset.length();
        let range = range.max(0.01);
        let near = (1.0 - distance / range).clamp(0.0, 1.0);
        // Quadratic falloff sounds more natural than linear.
        let gain = near * near;
        let pan = if distance > 0.05 {
            (offset / distance).dot(self.right).clamp(-1.0, 1.0) * 0.8
        } else {
            0.0
        };
        (gain, pan)
    }
}

/// The position of an entity for sound purposes.
fn entity_position(world: &SceneWorld, entity: Entity) -> Option<Vec3> {
    if let Ok(g) = world.get::<&GlobalTransform>(entity) {
        return Some(g.0.w_axis.truncate());
    }
    world.get::<&Transform>(entity).ok().map(|t| t.position)
}

/// Resolves a clip path against `root`.
#[must_use]
pub fn resolve_clip(root: Option<&Path>, clip: &str) -> PathBuf {
    let path = Path::new(clip);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match root {
        Some(root) => clip
            .split('/')
            .fold(root.to_path_buf(), |p, part| p.join(part)),
        None => path.to_path_buf(),
    }
}

impl AudioSourcePlayer {
    /// A player that hasn't started yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one frame: starts `play_on_start` sources (first call only),
    /// applies `commands`, then updates distance fading. Returns messages
    /// for the log (clips that failed to load, once each).
    pub fn update(
        &mut self,
        world: &SceneWorld,
        audio: &mut AudioEngine,
        root: Option<&Path>,
        commands: impl IntoIterator<Item = AudioCommand>,
    ) -> Vec<String> {
        let mut messages = Vec::new();
        if !self.started {
            self.started = true;
            let autoplay: Vec<Entity> = world
                .query::<(Entity, &AudioSource)>()
                .iter()
                .filter(|(_, s)| s.play_on_start)
                .map(|(e, _)| e)
                .collect();
            for entity in autoplay {
                self.play_entity(world, audio, root, entity, &mut messages);
            }
        }
        for command in commands {
            match command {
                AudioCommand::Play(entity) => {
                    self.play_entity(world, audio, root, entity, &mut messages);
                }
                AudioCommand::Stop(entity) => self.stop_entity(audio, entity),
                AudioCommand::OneShot {
                    path,
                    volume,
                    pitch,
                    position,
                } => {
                    let file = resolve_clip(root, &path);
                    let spatial = position.map(|p| (p, ONE_SHOT_RANGE));
                    if let Some(id) =
                        self.start(audio, &file, &path, volume, pitch, false, &mut messages)
                    {
                        self.voices.push(Voice {
                            id,
                            entity: None,
                            volume,
                            spatial,
                        });
                    }
                }
            }
        }
        // Forget finished sounds; stop sounds whose entity is gone.
        let mut gone = Vec::new();
        self.voices.retain(|v| {
            let alive = v.entity.is_none_or(|e| world.contains(e));
            if !alive {
                gone.push(v.id);
            }
            alive && !matches!(audio.status(v.id), None | Some(SoundStatus::Stopped))
        });
        for id in gone {
            audio.stop(id, 0.05);
        }
        // Distance fading.
        let listener = world.query::<&Camera3D>().iter().next().map(Listener::of);
        for v in &mut self.voices {
            let Some((mut at, range)) = v.spatial else {
                continue;
            };
            if let Some(p) = v.entity.and_then(|e| entity_position(world, e)) {
                at = p;
                v.spatial = Some((at, range));
            }
            let (gain, pan) = listener.map_or((1.0, 0.0), |l| l.mix(at, range));
            if let Ok(mut sound) = audio.sound(v.id) {
                sound.set_volume(f64::from(v.volume * gain), 0.0);
                sound.set_panning(f64::from(pan), 0.0);
            }
        }
        messages
    }

    /// Whether an entity's source is playing.
    #[must_use]
    pub fn is_playing(&self, entity: Entity) -> bool {
        self.voices.iter().any(|v| v.entity == Some(entity))
    }

    /// Number of playing sounds (sources and one-shots).
    #[must_use]
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// Pauses every sound this player started (the game is paused).
    pub fn pause_all(&self, audio: &mut AudioEngine) {
        for v in &self.voices {
            if let Ok(mut sound) = audio.sound(v.id) {
                sound.pause(0.05);
            }
        }
    }

    /// Resumes what [`Self::pause_all`] paused.
    pub fn resume_all(&self, audio: &mut AudioEngine) {
        for v in &self.voices {
            if let Ok(mut sound) = audio.sound(v.id) {
                sound.resume(0.05);
            }
        }
    }

    /// Stops everything (the scene is ending).
    pub fn stop_all(&mut self, audio: &mut AudioEngine) {
        for v in self.voices.drain(..) {
            audio.stop(v.id, 0.05);
        }
    }

    fn stop_entity(&mut self, audio: &mut AudioEngine, entity: Entity) {
        self.voices.retain(|v| {
            if v.entity == Some(entity) {
                audio.stop(v.id, 0.02);
                false
            } else {
                true
            }
        });
    }

    fn play_entity(
        &mut self,
        world: &SceneWorld,
        audio: &mut AudioEngine,
        root: Option<&Path>,
        entity: Entity,
        messages: &mut Vec<String>,
    ) {
        let Some(source) = world.get::<&AudioSource>(entity).ok().map(|s| (*s).clone()) else {
            return;
        };
        if source.clip.is_empty() {
            return;
        }
        // Restart rather than layer: play() on a playing source starts over.
        self.stop_entity(audio, entity);
        let file = resolve_clip(root, &source.clip);
        let spatial = source.spatial.then(|| {
            (
                entity_position(world, entity).unwrap_or(Vec3::ZERO),
                source.range,
            )
        });
        if let Some(id) = self.start(
            audio,
            &file,
            &source.clip,
            source.volume,
            source.pitch,
            source.looping,
            messages,
        ) {
            self.voices.push(Voice {
                id,
                entity: Some(entity),
                volume: source.volume,
                spatial,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        audio: &mut AudioEngine,
        file: &Path,
        shown: &str,
        volume: f32,
        pitch: f32,
        looped: bool,
        messages: &mut Vec<String>,
    ) -> Option<u64> {
        let path = file.to_string_lossy();
        let result = audio.play(PlayOptions {
            path: &path,
            volume: f64::from(volume.max(0.0)),
            looped,
            playback_rate: f64::from(pitch.clamp(0.05, 8.0)),
            panning: 0.0,
        });
        match result {
            Ok(id) => Some(id),
            Err(err) => {
                if self.failed.insert(shown.to_string()) {
                    messages.push(format!("Couldn't play {shown}: {err}"));
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a short silent WAV (16-bit mono, 44.1 kHz) and returns its path.
    pub(crate) fn write_wav(dir: &Path, name: &str, seconds: f32) -> PathBuf {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let samples = (44_100.0 * seconds) as u32;
        let data_len = samples * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&44_100u32.to_le_bytes());
        bytes.extend_from_slice(&88_200u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(bytes.len() + data_len as usize, 0);
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("audio-source-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn audio_paths_are_recognised_by_extension() {
        assert!(is_audio_path(Path::new("sounds/jump.OGG")));
        assert!(is_audio_path(Path::new("a.wav")));
        assert!(!is_audio_path(Path::new("tree.glb")));
        assert!(!is_audio_path(Path::new("ogg")));
    }

    #[test]
    fn play_on_start_commands_and_despawning_drive_the_voices() {
        let dir = temp_dir("player");
        write_wav(&dir, "hum.wav", 0.5);
        write_wav(&dir, "zap.wav", 0.1);
        let mut world = SceneWorld::new();
        let hum = world.spawn((
            Transform::from_position(Vec3::ZERO),
            AudioSource {
                looping: true,
                play_on_start: true,
                ..AudioSource::new("hum.wav")
            },
        ));
        let zap = world.spawn((Transform::default(), AudioSource::new("zap.wav")));
        let mut audio = AudioEngine::new();
        let mut player = AudioSourcePlayer::new();

        let msgs = player.update(&world, &mut audio, Some(&dir), []);
        assert!(msgs.is_empty(), "{msgs:?}");
        assert!(player.is_playing(hum));
        assert!(!player.is_playing(zap));

        player.update(&world, &mut audio, Some(&dir), [AudioCommand::Play(zap)]);
        assert!(player.is_playing(zap));
        // Playing again restarts instead of layering.
        player.update(&world, &mut audio, Some(&dir), [AudioCommand::Play(zap)]);
        assert_eq!(player.voice_count(), 2);

        // The 0.1 s zap ends; the looping hum keeps going.
        for _ in 0..30 {
            audio.pump_frame(1.0 / 60.0);
            player.update(&world, &mut audio, Some(&dir), []);
        }
        assert!(!player.is_playing(zap));
        assert!(player.is_playing(hum));

        player.update(&world, &mut audio, Some(&dir), [AudioCommand::Stop(hum)]);
        assert!(!player.is_playing(hum));
        player.update(&world, &mut audio, Some(&dir), [AudioCommand::Play(hum)]);
        world.despawn(hum).unwrap();
        player.update(&world, &mut audio, Some(&dir), []);
        assert_eq!(player.voice_count(), 0, "despawned entities fall silent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_shots_play_and_missing_clips_are_reported_once() {
        let dir = temp_dir("oneshot");
        write_wav(&dir, "hit.wav", 0.2);
        let world = SceneWorld::new();
        let mut audio = AudioEngine::new();
        let mut player = AudioSourcePlayer::new();
        let shot = |path: &str| AudioCommand::OneShot {
            path: path.into(),
            volume: 0.5,
            pitch: 1.2,
            position: Some(Vec3::new(3.0, 0.0, 0.0)),
        };
        let msgs = player.update(&world, &mut audio, Some(&dir), [shot("hit.wav")]);
        assert!(msgs.is_empty(), "{msgs:?}");
        assert_eq!(player.voice_count(), 1);
        let msgs = player.update(&world, &mut audio, Some(&dir), [shot("nope.ogg")]);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("nope.ogg"), "{msgs:?}");
        let msgs = player.update(&world, &mut audio, Some(&dir), [shot("nope.ogg")]);
        assert!(msgs.is_empty(), "reported once: {msgs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spatial_sources_fade_with_distance_and_pan_toward_their_side() {
        let camera = Camera3D {
            position: Vec3::ZERO,
            target: Vec3::NEG_Z,
            ..Camera3D::default()
        };
        let l = Listener::of(&camera);
        let (near, pan_right) = l.mix(Vec3::new(2.0, 0.0, 0.0), 20.0);
        let (far, pan_left) = l.mix(Vec3::new(-15.0, 0.0, 0.0), 20.0);
        let (silent, _) = l.mix(Vec3::new(0.0, 0.0, -25.0), 20.0);
        assert!(near > far && far > 0.0, "{near} {far}");
        assert!(silent.abs() < 1e-6);
        assert!(pan_right > 0.5 && pan_left < -0.5, "{pan_right} {pan_left}");
        let (_, ahead) = l.mix(Vec3::new(0.0, 0.0, -5.0), 20.0);
        assert!(ahead.abs() < 1e-4);
    }

    #[test]
    fn clips_resolve_against_the_root() {
        let root = Path::new("/games/fps");
        assert_eq!(
            resolve_clip(Some(root), "sounds/jump.ogg"),
            root.join("sounds").join("jump.ogg")
        );
        assert_eq!(resolve_clip(None, "a.ogg"), PathBuf::from("a.ogg"));
    }
}
