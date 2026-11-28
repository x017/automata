//! "Clips from file…": a model's import settings can borrow the animations
//! of other glTF files made for the same skeleton (one Mixamo download per
//! move, say). This is the Clip files block under a rigged model and the
//! file picker behind its button.

use std::path::{Component, Path, PathBuf};

use automata_assets::import::{read_settings, ClipFile};
use automata_assets::Rig;

use super::palette::{PaletteAction, PaletteItem};
use super::{ButtonStyle, Icon, Intent, Rect, Ui};
use crate::canvas::Canvas;
use crate::theme::{color, layout};

/// `file` as written in `model`'s import settings: relative to the model's
/// folder, with `/` separators and `..` to climb out of it.
#[must_use]
pub fn relative_file(model: &Path, file: &Path) -> String {
    let dir = model.parent().unwrap_or_else(|| Path::new(""));
    let from: Vec<Component<'_>> = dir.components().collect();
    let to: Vec<Component<'_>> = file.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); from.len() - shared];
    parts.extend(
        to[shared..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Whether `path` is a glTF file (the only kind that holds animations).
fn is_gltf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
}

/// The palette entries for picking a clip file for `model`: the project's
/// glTF files, minus the model itself and the files it already uses.
#[must_use]
pub fn file_items(root: &Path, model: &Path) -> Vec<PaletteItem> {
    let used: Vec<PathBuf> = read_settings(model)
        .clips
        .iter()
        .map(|c| c.path(model))
        .collect();
    crate::models::project_files(root, 2000, is_gltf)
        .into_iter()
        .filter(|p| p != model && !used.contains(p))
        .map(|path| PaletteItem {
            label: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            group: "Model",
            hint: path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string(),
            action: PaletteAction::ClipFile {
                model: model.to_path_buf(),
                file: path,
            },
        })
        .collect()
}

/// Adds `file`'s animations to `model`'s clips.
#[must_use]
pub fn add_file(model: PathBuf, file: &Path) -> Intent {
    let mut settings = read_settings(&model);
    let rel = relative_file(&model, file);
    if !settings.clips.iter().any(|c| c.file == rel) {
        settings.clips.push(ClipFile::new(rel));
    }
    Intent::SetImport { model, settings }
}

/// The clips of `rig` that came from `source`.
fn clips_from<'r>(rig: &'r Rig, source: &ClipFile) -> Vec<&'r str> {
    let stem = source.clip_names(&["only"]).remove(0);
    let prefix = format!("{stem}:");
    rig.clips
        .iter()
        .map(|c| c.name.as_str())
        .filter(|n| *n == stem || n.starts_with(&prefix))
        .collect()
}

impl Ui {
    /// The Clip files rows under a rigged model's import settings: each
    /// borrowed file with the clips it gave, and a button to add one.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn clip_files_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        mut y: f32,
        w: f32,
        file: &Path,
        rig: &Rig,
        editing: bool,
    ) -> f32 {
        let settings = read_settings(file);
        let fx = x + layout::LABEL_W;
        let fw = w - layout::LABEL_W;
        let (normal, ghost) = if editing {
            (ButtonStyle::Normal, ButtonStyle::Ghost)
        } else {
            (ButtonStyle::Disabled, ButtonStyle::Disabled)
        };

        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            canvas.text(
                x,
                y + 3.0,
                layout::SMALL_SIZE,
                "Clip files",
                color::text_dim(),
            );
            let add = Rect::new(fx, y, fw, layout::FIELD_H);
            self.tip(
                add,
                "Use the animations of another glTF made for this skeleton",
            );
            if self.button(canvas, add, "+ Clips from file…", normal) && editing {
                self.open_clip_file_picker(file);
            }
        }
        y += layout::ROW_STEP;
        if settings.clips.is_empty() {
            if self.row_visible(Rect::new(x, y, w, 16.0)) {
                canvas.text_clipped(
                    fx,
                    y,
                    layout::SMALL_SIZE,
                    "Animations kept in other files",
                    color::text_faint(),
                    fw,
                );
            }
            return y + 20.0;
        }

        let mut remove = None;
        for (i, source) in settings.clips.iter().enumerate() {
            let names = clips_from(rig, source);
            if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
                let ok = !names.is_empty();
                let tint = if ok { color::text() } else { color::danger() };
                canvas.text_clipped(x, y + 3.0, layout::FONT_SIZE, &source.file, tint, w - 30.0);
                let label = Rect::new(x, y, w - 30.0, layout::FIELD_H);
                if ok {
                    self.tip(label, &source.file);
                } else {
                    self.tip(
                        label,
                        "No clips: the file is missing, has no animations, or \
                         its bones have other names",
                    );
                }
                let close = Rect::new(x + w - 22.0, y + 1.0, 22.0, 18.0);
                self.tip(close, "Remove");
                if self.icon_button(canvas, close, Icon::Close, ghost) && editing {
                    remove = Some(i);
                }
            }
            y += layout::ROW_STEP;
            if !names.is_empty() {
                let line = names.join(", ");
                if self.row_visible(Rect::new(x, y, w, 16.0)) {
                    canvas.text_clipped(fx, y, layout::SMALL_SIZE, &line, color::text_faint(), fw);
                    self.tip(Rect::new(fx, y, fw, 16.0), &line);
                }
                y += 20.0;
            }
        }
        if let Some(i) = remove {
            let mut s = settings.clone();
            s.clips.remove(i);
            self.out.push(Intent::SetImport {
                model: file.to_path_buf(),
                settings: s,
            });
        }
        y + 4.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_written_relative_to_the_model() {
        let model = Path::new("/game/models/hero.glb");
        assert_eq!(
            relative_file(model, Path::new("/game/models/run.glb")),
            "run.glb"
        );
        assert_eq!(
            relative_file(model, Path::new("/game/models/anims/Run Fast.glb")),
            "anims/Run Fast.glb"
        );
        assert_eq!(
            relative_file(model, Path::new("/game/anims/sword.gltf")),
            "../anims/sword.gltf"
        );
    }

    #[test]
    fn the_picker_lists_other_gltf_files_and_adding_one_is_idempotent() {
        let root = std::env::temp_dir().join(format!("clip-picker-{}", std::process::id()));
        std::fs::create_dir_all(root.join("models/anims")).unwrap();
        for f in [
            "models/hero.glb",
            "models/anims/run.glb",
            "models/anims/jump.gltf",
            "models/tree.obj",
        ] {
            std::fs::write(root.join(f), b"").unwrap();
        }
        let model = root.join("models/hero.glb");
        let labels = |items: Vec<PaletteItem>| -> Vec<String> {
            items.into_iter().map(|i| i.label).collect()
        };
        assert_eq!(labels(file_items(&root, &model)), ["jump.gltf", "run.glb"]);

        let Intent::SetImport { settings, .. } =
            add_file(model.clone(), &root.join("models/anims/run.glb"))
        else {
            panic!("adds through the import settings");
        };
        assert_eq!(settings.clips, [ClipFile::new("anims/run.glb")]);
        automata_assets::import::write_settings(&model, &settings).unwrap();
        // Used files leave the list; adding one twice keeps one entry.
        assert_eq!(labels(file_items(&root, &model)), ["jump.gltf"]);
        let Intent::SetImport { settings, .. } =
            add_file(model.clone(), &root.join("models/anims/run.glb"))
        else {
            panic!();
        };
        assert_eq!(settings.clips.len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
