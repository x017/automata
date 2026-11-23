//! Per-model import settings: fix-ups applied every time a model loads.
//!
//! Models often arrive at the wrong size, facing the wrong way, or with the
//! origin somewhere odd. Instead of correcting every entity that uses one,
//! the fix is stored once beside the model, in `<model>.import.ron`
//! (for example `tree.glb.import.ron`; not plain `.import`, which Godot
//! kits already ship), and baked into the geometry when it loads:
//!
//! ```ron
//! (scale: 0.01, rotation: (0.0, 180.0, 0.0), pivot: BottomCenter)
//! ```
//!
//! Order: scale, then rotation (degrees, X then Y then Z), then the pivot
//! move. The file is plain text, ships with exports like any project file,
//! and editing it reloads the model like editing the model itself.

use std::path::{Path, PathBuf};

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::ModelPart;

/// Where a model's origin ends up after import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Pivot {
    /// Keep the file's own origin.
    #[default]
    File,
    /// The centre of the bottom face of the bounding box: models stand on
    /// the ground where they're placed.
    BottomCenter,
    /// The centre of the bounding box.
    Center,
}

impl Pivot {
    /// Every option, in menu order.
    pub const ALL: [Self; 3] = [Self::File, Self::BottomCenter, Self::Center];

    /// A short label for menus.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "File origin",
            Self::BottomCenter => "Bottom centre",
            Self::Center => "Centre",
        }
    }
}

/// How a model file is adjusted when it loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportSettings {
    /// Uniform scale (`0.01` turns centimetres into metres).
    pub scale: f32,
    /// Rotation in degrees about X, Y, then Z.
    pub rotation: [f32; 3],
    /// Where the origin goes.
    pub pivot: Pivot,
    /// Bones that swing on their own (see [`crate::springs`]).
    #[serde(skip_serializing_if = "crate::springs::Springs::is_unset")]
    pub springs: crate::springs::Springs,
    /// Animations kept in other files, added to this model's clips (see
    /// [`ClipFile`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<ClipFile>,
    /// Named moments in clips that scripts hear about (see [`EventMark`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventMark>,
}

/// An animation event: when `clip` passes `time` seconds, scripts hear
/// `name` (`on_animation_event`, and the channel
/// `<entity>:animation:<name>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventMark {
    /// The clip's exact name (borrowed clips too: `"emotes:Wave"`).
    pub clip: String,
    /// Seconds into the clip.
    pub time: f32,
    /// The event's name (`"footstep"`).
    pub name: String,
}

impl EventMark {
    /// `name` at `time` seconds into `clip`.
    #[must_use]
    pub fn new(clip: impl Into<String>, time: f32, name: impl Into<String>) -> Self {
        Self {
            clip: clip.into(),
            time,
            name: name.into(),
        }
    }
}

/// A file whose animations a model borrows: a glTF made for the same
/// skeleton (same bone names), like the one-animation-per-file downloads
/// from Mixamo. Its clips are matched to the model's bones by name.
///
/// Its clips are named after the file: a file with one clip gives a clip
/// called after the file (`run.glb` gives `run`); a file with several gives
/// `file:Clip` (`sword.glb` with Slash and Block gives `sword:Slash` and
/// `sword:Block`). Names are exact, like every clip name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipFile {
    /// The file, relative to the model's folder.
    pub file: String,
}

impl ClipFile {
    /// A clip file at `file` (relative to the model's folder).
    #[must_use]
    pub fn new(file: impl Into<String>) -> Self {
        Self { file: file.into() }
    }

    /// What clips from this file are called: `stem` for a file with one
    /// clip, `stem:clip` otherwise.
    #[must_use]
    pub fn clip_names(&self, clips: &[&str]) -> Vec<String> {
        let path = std::path::Path::new(&self.file);
        let stem = path
            .file_stem()
            .map_or_else(|| self.file.clone(), |s| s.to_string_lossy().into_owned());
        match clips {
            [_] => vec![stem],
            _ => clips.iter().map(|c| format!("{stem}:{c}")).collect(),
        }
    }

    /// Where the file is, for a model at `model`.
    #[must_use]
    pub fn path(&self, model: &std::path::Path) -> std::path::PathBuf {
        model
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(&self.file)
    }
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            rotation: [0.0; 3],
            pivot: Pivot::File,
            springs: crate::springs::Springs::default(),
            clips: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl ImportSettings {
    /// Whether these settings change nothing (no file needed).
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.keeps_geometry()
            && self.springs.is_unset()
            && self.clips.is_empty()
            && self.events.is_empty()
    }

    /// Whether the scale, rotation and pivot leave the geometry as it is.
    #[must_use]
    pub fn keeps_geometry(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-6
            && self.rotation.iter().all(|r| r.abs() < 1e-6)
            && self.pivot == Pivot::File
    }

    /// Bakes the settings into `parts`, returning the transform applied
    /// (rigs apply it after posing).
    pub(crate) fn apply(&self, parts: &mut [ModelPart]) -> Mat4 {
        if self.keeps_geometry() {
            return Mat4::IDENTITY;
        }
        let scale = if self.scale.is_finite() && self.scale.abs() > 1e-6 {
            self.scale
        } else {
            1.0
        };
        let [rx, ry, rz] = self.rotation.map(f32::to_radians);
        let rot = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
        for part in parts.iter_mut() {
            for v in &mut part.mesh.vertices {
                v.position = (rot * (Vec3::from(v.position) * scale)).to_array();
                v.normal = (rot * Vec3::from(v.normal) * scale.signum()).to_array();
            }
            if scale < 0.0 {
                // A mirror turns faces inside out; swap the winding back.
                for t in part.mesh.indices.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }
        }
        let transform = Mat4::from_quat(rot) * Mat4::from_scale(Vec3::splat(scale));
        if self.pivot == Pivot::File {
            return transform;
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in parts.iter().flat_map(|p| &p.mesh.vertices) {
            lo = lo.min(Vec3::from(v.position));
            hi = hi.max(Vec3::from(v.position));
        }
        if !lo.is_finite() {
            return transform;
        }
        let center = (lo + hi) * 0.5;
        let origin = match self.pivot {
            Pivot::File => Vec3::ZERO,
            Pivot::BottomCenter => Vec3::new(center.x, lo.y, center.z),
            Pivot::Center => center,
        };
        for v in parts.iter_mut().flat_map(|p| &mut p.mesh.vertices) {
            v.position = (Vec3::from(v.position) - origin).to_array();
        }
        Mat4::from_translation(-origin) * transform
    }
}

/// The settings file of a model: `<model>.import.ron`.
#[must_use]
pub fn settings_path(model: &Path) -> PathBuf {
    let mut name = model.as_os_str().to_owned();
    name.push(".import.ron");
    PathBuf::from(name)
}

/// The settings saved for `model` (defaults when there is no file or it
/// can't be read; a broken file is logged).
#[must_use]
pub fn read_settings(model: &Path) -> ImportSettings {
    let path = settings_path(model);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ImportSettings::default();
    };
    ron::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!("{}: {e} (ignored)", path.display());
        ImportSettings::default()
    })
}

/// Saves the settings for `model`. Default settings delete the file, so
/// untouched models have no sidecar. The model reloads on the next
/// [`crate::refresh`].
///
/// # Errors
///
/// Fails when the file can't be written or removed.
pub fn write_settings(model: &Path, settings: &ImportSettings) -> std::io::Result<()> {
    let path = settings_path(model);
    if settings.is_identity() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let text = ron::ser::to_string_pretty(settings, ron::ser::PrettyConfig::default())
        .map_err(std::io::Error::other)?;
    std::fs::write(path, text + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_renderer::{MeshData, Vertex};

    fn part() -> ModelPart {
        // A unit triangle standing on x 0..1, y 2..3.
        let verts = vec![
            Vertex::new([0.0, 2.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
            Vertex::new([1.0, 2.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0]),
            Vertex::new([0.0, 3.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0]),
        ];
        ModelPart {
            name: "p".into(),
            mesh: MeshData::new(verts, vec![0, 1, 2]),
            color: [1.0; 4],
            texture: None,
            pixelated: false,
            skin: None,
            surface: crate::PartSurface::default(),
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn clip_files_name_their_clips_after_the_file() {
        let one = ClipFile::new("anims/Run Fast.glb");
        assert_eq!(one.clip_names(&["mixamo.com"]), ["Run Fast"]);
        let many = ClipFile::new("sword.glb");
        assert_eq!(
            many.clip_names(&["Slash", "Block"]),
            ["sword:Slash", "sword:Block"]
        );
        assert_eq!(
            one.path(std::path::Path::new("models/hero.glb")),
            std::path::Path::new("models/anims/Run Fast.glb")
        );
        assert!(!ImportSettings {
            clips: vec![one],
            ..ImportSettings::default()
        }
        .is_identity());
    }

    #[test]
    fn scale_rotation_and_pivot_bake_into_the_geometry() {
        let mut parts = [part()];
        ImportSettings {
            scale: 2.0,
            rotation: [0.0, 180.0, 0.0],
            pivot: Pivot::BottomCenter,
            ..ImportSettings::default()
        }
        .apply(&mut parts);
        let v = &parts[0].mesh.vertices;
        // Scaled ×2 → x 0..2, y 4..6; turned 180° → x -2..0, normal -Z;
        // pivot → x centred, lowest point at y = 0.
        assert!(close(v[0].position, [1.0, 0.0, 0.0]), "{:?}", v[0].position);
        assert!(
            close(v[1].position, [-1.0, 0.0, 0.0]),
            "{:?}",
            v[1].position
        );
        assert!(close(v[2].position, [1.0, 2.0, 0.0]), "{:?}", v[2].position);
        assert!(close(v[0].normal, [0.0, 0.0, -1.0]));
    }

    #[test]
    fn centre_pivot_and_mirroring_keep_faces_outward() {
        let mut parts = [part()];
        ImportSettings {
            scale: -1.0,
            pivot: Pivot::Center,
            ..ImportSettings::default()
        }
        .apply(&mut parts);
        assert_eq!(parts[0].mesh.indices, [0, 2, 1]);
        let ys: Vec<f32> = parts[0]
            .mesh
            .vertices
            .iter()
            .map(|v| v.position[1])
            .collect();
        assert!(ys.iter().all(|y| y.abs() <= 0.5 + 1e-5), "{ys:?}");
    }

    #[test]
    fn rigs_apply_the_settings_after_posing() {
        let dir = std::env::temp_dir().join(format!("import-rig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("skin.gltf");
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/SimpleSkin.gltf");
        std::fs::copy(data, &model).unwrap();
        let settings = ImportSettings {
            scale: -2.0,
            rotation: [0.0, 90.0, 0.0],
            pivot: Pivot::BottomCenter,
            ..ImportSettings::default()
        };
        write_settings(&model, &settings).unwrap();
        let loaded = crate::load_file(&model).unwrap();
        let rig = loaded.rig.as_ref().unwrap();
        // Skinning the bind pose with the rest palette lands exactly on the
        // baked (scaled, turned, re-pivoted) rest mesh.
        let part = &loaded.parts[0];
        let posed = part.skin.as_ref().unwrap().posed(&rig.rest_palette());
        for (a, b) in posed.iter().zip(&part.mesh.vertices) {
            assert!(Vec3::from(a.position).abs_diff_eq(Vec3::from(b.position), 1e-4));
            assert!(Vec3::from(a.normal).abs_diff_eq(Vec3::from(b.normal), 1e-4));
        }
        assert!(loaded.min.y.abs() < 1e-5, "bottom-centre pivot");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_files_round_trip_and_defaults_remove_them() {
        let dir = std::env::temp_dir().join(format!("import-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("tree.glb");
        assert_eq!(settings_path(&model), dir.join("tree.glb.import.ron"));
        assert_eq!(read_settings(&model), ImportSettings::default());
        let s = ImportSettings {
            scale: 0.01,
            rotation: [90.0, 0.0, 0.0],
            pivot: Pivot::Center,
            springs: crate::springs::Springs {
                chains: vec![crate::springs::SpringChain::new("Tail")],
                colliders: vec![],
            },
            clips: vec![ClipFile::new("anims/run.glb")],
            events: vec![EventMark::new("Run", 0.3, "footstep")],
        };
        write_settings(&model, &s).unwrap();
        assert_eq!(read_settings(&model), s);
        // Partial files fill in defaults.
        std::fs::write(settings_path(&model), "(scale: 3.0)").unwrap();
        assert!((read_settings(&model).scale - 3.0).abs() < 1e-6);
        assert_eq!(read_settings(&model).pivot, Pivot::File);
        write_settings(&model, &ImportSettings::default()).unwrap();
        assert!(!settings_path(&model).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
