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

use glam::{EulerRot, Quat, Vec3};
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportSettings {
    /// Uniform scale (`0.01` turns centimetres into metres).
    pub scale: f32,
    /// Rotation in degrees about X, Y, then Z.
    pub rotation: [f32; 3],
    /// Where the origin goes.
    pub pivot: Pivot,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            rotation: [0.0; 3],
            pivot: Pivot::File,
        }
    }
}

impl ImportSettings {
    /// Whether these settings change nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-6
            && self.rotation.iter().all(|r| r.abs() < 1e-6)
            && self.pivot == Pivot::File
    }

    /// Bakes the settings into `parts`.
    pub(crate) fn apply(&self, parts: &mut [ModelPart]) {
        if self.is_identity() {
            return;
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
        if self.pivot == Pivot::File {
            return;
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in parts.iter().flat_map(|p| &p.mesh.vertices) {
            lo = lo.min(Vec3::from(v.position));
            hi = hi.max(Vec3::from(v.position));
        }
        if !lo.is_finite() {
            return;
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
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn scale_rotation_and_pivot_bake_into_the_geometry() {
        let mut parts = [part()];
        ImportSettings {
            scale: 2.0,
            rotation: [0.0, 180.0, 0.0],
            pivot: Pivot::BottomCenter,
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
