//! Sound files in the editor: importing them into the project, listing
//! them, and summarizing a clip (length and waveform) for the Inspector.
//!
//! Sounds work like models: importing copies the file into the project's
//! `sounds/` folder so scenes keep a short relative path and exports carry
//! it along; files already inside the project are used where they are.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use automata_audio::ClipInfo;
use automata_core::audio_source::is_audio_path;

use crate::models::{free_name, is_inside, project_files};

/// The project folder sounds are imported into.
pub const SOUNDS_DIR: &str = "sounds";

/// Sound files under `root`, at most `limit`, sorted by path.
#[must_use]
pub fn project_sounds(root: &Path, limit: usize) -> Vec<PathBuf> {
    project_files(root, limit, is_audio_path)
}

/// Copies a sound into `project/sounds/` and returns where it landed (a
/// file already inside the project is returned unchanged).
///
/// # Errors
///
/// Fails when `src` isn't a sound file or copying fails.
pub fn import_sound(src: &Path, project: &Path) -> Result<PathBuf, String> {
    if !is_audio_path(src) {
        return Err(format!(
            "{} isn't a sound (use .ogg, .wav, .mp3 or .flac)",
            src.display()
        ));
    }
    if !src.is_file() {
        return Err(format!("{} doesn't exist", src.display()));
    }
    if is_inside(src, project) {
        return Ok(src.to_path_buf());
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("the sound has no file name")?;
    let dir = project.join(SOUNDS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = free_name(&dir, &name);
    std::fs::copy(src, &dest).map_err(|e| format!("copying {}: {e}", src.display()))?;
    Ok(dest)
}

/// An entity name for a sound file (`door_open.ogg` → `Door open`).
#[must_use]
pub fn entity_name(path: &Path) -> String {
    crate::models::entity_name(path)
}

/// One line for the Inspector: `0.42 s · 44.1 kHz`.
#[must_use]
pub fn describe(info: &ClipInfo) -> String {
    format!(
        "{:.2} s · {:.1} kHz",
        info.duration,
        f64::from(info.sample_rate) / 1000.0
    )
}

/// Decoded clip summaries, refreshed when a file changes on disk (the
/// Inspector asks every frame; decoding happens once per version).
#[derive(Debug, Default)]
pub struct ClipCache {
    entries: HashMap<PathBuf, (Option<SystemTime>, Result<ClipInfo, String>)>,
}

impl ClipCache {
    /// The summary of the sound file at `path`.
    pub fn get(&mut self, path: &Path) -> &Result<ClipInfo, String> {
        let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let stale = self.entries.get(path).is_none_or(|(s, _)| *s != stamp);
        if stale {
            let info = if stamp.is_none() {
                Err("File not found".to_string())
            } else {
                automata_audio::probe(&path.to_string_lossy()).map_err(|e| match e {
                    automata_audio::AudioError::Load { source, .. } => {
                        format!("Can't decode: {source}")
                    }
                    other => other.to_string(),
                })
            };
            self.entries.insert(path.to_path_buf(), (stamp, info));
        }
        &self.entries[path].1
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn imports_into_sounds_and_keeps_project_files_in_place() {
        let base = std::env::temp_dir().join(format!("oberon_sounds_{}", std::process::id()));
        let project = base.join("game");
        let outside = base.join("downloads");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let src = outside.join("Bell.OGG");
        std::fs::write(&src, b"not really ogg").unwrap();

        let dest = import_sound(&src, &project).unwrap();
        assert_eq!(dest, project.join("sounds").join("Bell.OGG"));
        // Again: a free name, never an overwrite.
        let again = import_sound(&src, &project).unwrap();
        assert_eq!(again, project.join("sounds").join("Bell 2.OGG"));
        // Inside the project: used where it is.
        assert_eq!(import_sound(&dest, &project).unwrap(), dest);
        assert!(import_sound(&outside.join("x.txt"), &project).is_err());

        let listed = project_sounds(&project, 10);
        assert_eq!(listed.len(), 2);

        // A file that isn't really audio reports a decode error, cached.
        let mut cache = ClipCache::default();
        assert!(cache.get(&dest).is_err());
        assert_eq!(
            cache
                .get(&project.join("missing.wav"))
                .as_ref()
                .unwrap_err(),
            "File not found"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn summarizes_a_real_clip() {
        let dir = std::env::temp_dir().join(format!("oberon_clip_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("tone.wav");
        write_test_wav(&wav, 22_050, 0.5);
        let mut cache = ClipCache::default();
        let info = cache.get(&wav).clone().unwrap();
        assert!((info.duration - 0.5).abs() < 0.01, "{}", info.duration);
        assert_eq!(info.sample_rate, 22_050);
        assert!(info.peaks.iter().any(|&p| p > 0.4), "{:?}", info.peaks);
        assert_eq!(describe(&info), "0.50 s · 22.1 kHz");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A mono 16-bit sine at half volume.
    pub(crate) fn write_test_wav(path: &Path, rate: u32, seconds: f32) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (rate as f32 * seconds) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + n * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(n * 2).to_le_bytes());
        for i in 0..n {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let v = ((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin()
                * 0.5
                * f32::from(i16::MAX)) as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }
}
