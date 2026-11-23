//! The command palette (Ctrl+Shift+P): one search box over every menu
//! command, every entity in the scene, every script in the project and the
//! Settings pages.

use std::path::{Path, PathBuf};

use super::menus::{Action, BAR};
use super::settings::SettingsPage;
use super::{CursorHint, Intent, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::theme::{color, layout};

/// Rows shown at once.
const ROWS: usize = 11;
const ROW_H: f32 = 30.0;
const WIDTH: f32 = 640.0;

/// What choosing a palette entry does.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteAction {
    /// A menu command.
    Menu(Action),
    /// Select (and frame) an entity.
    Entity(u64),
    /// Open a script in the editor.
    Script(PathBuf),
    /// Open a Settings page.
    Settings(SettingsPage),
    /// Add a model to the scene (`target: None`) or show it on an entity.
    Model {
        /// The model file.
        path: PathBuf,
        /// The entity to show it on.
        target: Option<u64>,
    },
    /// Add a sound to the scene (`target: None`) or play it from an entity.
    Sound {
        /// The sound file.
        path: PathBuf,
        /// The entity whose Audio Source gets it.
        target: Option<u64>,
    },
    /// Start a spring chain at a bone of a model, or put a collider on it.
    SpringBone {
        /// The model file (its import settings get the chain).
        model: PathBuf,
        /// The bone's name.
        bone: String,
        /// Chain or collider.
        pick: super::springs::SpringPick,
    },
}

/// One searchable entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteItem {
    /// Main text.
    pub label: String,
    /// Category tag ("File", "Entity", "Script", …).
    pub group: &'static str,
    /// Right-aligned hint (shortcut or path).
    pub hint: String,
    /// What it does.
    pub action: PaletteAction,
}

/// Keys the palette understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKey {
    /// Previous entry.
    Up,
    /// Next entry.
    Down,
    /// Run the selected entry.
    Enter,
    /// Delete the last query character.
    Backspace,
    /// Close.
    Escape,
}

/// The open palette.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    /// The search text.
    pub query: String,
    selected: usize,
    items: Vec<PaletteItem>,
    /// Go to File mode (Ctrl+E): scripts only.
    pub files_only: bool,
    /// Search box hint replacing the default one (file pickers).
    pub placeholder: Option<&'static str>,
}

impl Palette {
    /// Entries matching the query, best first.
    #[must_use]
    pub fn matches(&self) -> Vec<&PaletteItem> {
        let mut scored: Vec<(i32, usize, &PaletteItem)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                // The label matters most ("sa" → Save, not "Script app.py");
                // the group only helps queries like "file save".
                let label = fuzzy_score(&self.query, &item.label).map(|s| {
                    let prefix = item
                        .label
                        .to_lowercase()
                        .starts_with(&self.query.trim().to_lowercase());
                    s + if prefix { 20 } else { 0 }
                });
                let grouped = || {
                    fuzzy_score(&self.query, &format!("{} {}", item.group, item.label))
                        .map(|s| s - 12)
                };
                label.or_else(grouped).map(|s| (s, i, item))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, item)| item).collect()
    }
}

/// Subsequence fuzzy match: higher is better; `None` when `query` doesn't
/// match. Consecutive runs, word starts and early matches score higher.
#[must_use]
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut prev_match = false;
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && c.to_lowercase().eq(std::iter::once(q[qi])) {
            let word_start = i == 0 || !t[i - 1].is_alphanumeric();
            score += 1 + if prev_match { 5 } else { 0 } + if word_start { 8 } else { 0 };
            score -= i32::try_from(i.min(40)).unwrap_or(40) / 8;
            qi += 1;
            prev_match = true;
        } else {
            prev_match = false;
        }
    }
    (qi == q.len()).then_some(score)
}

/// Python files under `root` (skipping hidden, virtualenv and build
/// directories), at most `limit`.
fn project_scripts(root: &Path, limit: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, limit: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if out.len() >= limit {
                return;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "venv" | "target" | "node_modules" | "__pycache__" | "vendor"
                )
            {
                continue;
            }
            if path.is_dir() {
                if depth < 5 {
                    walk(&path, depth + 1, out, limit);
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("py"))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out, limit);
    out
}

impl Ui {
    /// Whether the command palette is open.
    #[must_use]
    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    /// Opens the command palette.
    pub fn open_palette(&mut self, doc: &Document) {
        let mut items = Vec::new();
        for (i, title) in BAR.iter().enumerate() {
            for item in self.bar_items(i, doc) {
                let Some(action) = item.action.clone() else {
                    continue;
                };
                if !item.enabled || item.label == "-" {
                    continue;
                }
                items.push(PaletteItem {
                    label: item.label.trim_end_matches('…').to_string(),
                    group: title,
                    hint: item.shortcut.to_string(),
                    action: PaletteAction::Menu(action),
                });
            }
        }
        for page in [
            SettingsPage::Appearance,
            SettingsPage::Editor,
            SettingsPage::Python,
            SettingsPage::Files,
        ] {
            items.push(PaletteItem {
                label: format!("Settings: {}", settings_title(page)),
                group: "Settings",
                hint: if page == SettingsPage::Appearance {
                    "Ctrl+,".into()
                } else {
                    String::new()
                },
                action: PaletteAction::Settings(page),
            });
        }
        for e in &doc.entities {
            items.push(PaletteItem {
                label: e.name().to_string(),
                group: "Entity",
                hint: format!("#{}", e.id),
                action: PaletteAction::Entity(e.id),
            });
        }
        let root = self.project_root.clone();
        for path in project_scripts(&root, 400) {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            items.push(PaletteItem {
                label: name,
                group: "Script",
                hint: rel,
                action: PaletteAction::Script(path),
            });
        }
        self.menu = None;
        self.picker = None;
        items.push(PaletteItem {
            label: format!("Settings: {}", settings_title(SettingsPage::Build)),
            group: "Settings",
            hint: String::new(),
            action: PaletteAction::Menu(Action::Ui(super::menus::UiAction::BuildSettings)),
        });
        self.palette = Some(Palette {
            query: String::new(),
            selected: 0,
            items,
            files_only: false,
            placeholder: None,
        });
    }

    /// Go to File (Ctrl+E, VS Code's Quick Open): open scripts first in
    /// most-recently-used order, then every script in the project.
    pub fn open_quick_open(&mut self) {
        let root = self.project_root.clone();
        let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
        let name = |p: &Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let mut items: Vec<PaletteItem> = Vec::new();
        let active = self.code.active;
        for i in self.code.mru_order().into_iter().flatten() {
            // The active file is last in VS Code's list; skip it here.
            if Some(i) == active {
                continue;
            }
            if let Some(t) = self.code.tabs.get(i) {
                items.push(PaletteItem {
                    label: name(&t.path),
                    group: "Open",
                    hint: rel(&t.path),
                    action: PaletteAction::Script(t.path.clone()),
                });
            }
        }
        let open: std::collections::HashSet<PathBuf> =
            self.code.tabs.iter().map(|t| t.path.clone()).collect();
        for path in project_scripts(&root, 2000) {
            if open.contains(&path) && Some(&path) != self.code.current().map(|t| &t.path) {
                continue;
            }
            items.push(PaletteItem {
                label: name(&path),
                group: "Script",
                hint: rel(&path),
                action: PaletteAction::Script(path),
            });
        }
        self.menu = None;
        self.picker = None;
        self.palette = Some(Palette {
            query: String::new(),
            selected: 0,
            items,
            files_only: true,
            placeholder: None,
        });
    }

    /// Types into the palette's search box.
    pub fn palette_text(&mut self, text: &str) {
        if let Some(p) = &mut self.palette {
            p.query.extend(text.chars().filter(|c| !c.is_control()));
            p.selected = 0;
        }
    }

    /// Handles a palette key.
    pub fn palette_key(&mut self, key: PaletteKey, doc: &Document) {
        let Some(p) = &mut self.palette else { return };
        let n = p.matches().len();
        match key {
            PaletteKey::Up if n > 0 => p.selected = (p.selected + n - 1) % n,
            PaletteKey::Down if n > 0 => p.selected = (p.selected + 1) % n,
            PaletteKey::Backspace => {
                p.query.pop();
                p.selected = 0;
            }
            PaletteKey::Escape => self.palette = None,
            PaletteKey::Enter => {
                let chosen = p.matches().get(p.selected).map(|i| i.action.clone());
                self.palette = None;
                if let Some(action) = chosen {
                    self.run_palette(action, doc);
                }
            }
            PaletteKey::Up | PaletteKey::Down => {}
        }
    }

    fn run_palette(&mut self, action: PaletteAction, doc: &Document) {
        match action {
            PaletteAction::Menu(Action::Intent(i)) => self.out.push(i),
            PaletteAction::Menu(Action::Ui(a)) => self.run_action(a, doc),
            PaletteAction::Entity(id) => {
                self.code.show_scene();
                self.out.push(Intent::Select(Some(id)));
                self.run_action(super::menus::UiAction::Focus(id), doc);
            }
            PaletteAction::Script(path) => self.out.push(Intent::OpenScript(path)),
            PaletteAction::Settings(page) => {
                self.settings_page = page;
                self.open_settings();
            }
            PaletteAction::Model { path, target } => self.out.push(match target {
                Some(id) => Intent::SetModel { id, path },
                None => Intent::AddModel { path, at: None },
            }),
            PaletteAction::Sound { path, target } => self.out.push(match target {
                Some(id) => Intent::SetAudioClip { id, path },
                None => Intent::AddSound { path, at: None },
            }),
            PaletteAction::SpringBone { model, bone, pick } => {
                self.out.push(super::springs::add_bone(model, bone, pick));
            }
        }
    }

    /// The bone picker: `rig`'s bones, likely ones first, to start a
    /// spring chain at one or put a collider on one (`pick`).
    pub fn open_bone_picker(
        &mut self,
        model: &Path,
        rig: &automata_assets::Rig,
        pick: super::springs::SpringPick,
    ) {
        use super::springs::SpringPick;
        self.menu = None;
        self.picker = None;
        self.palette = Some(Palette {
            query: String::new(),
            selected: 0,
            items: super::springs::bone_items(model, rig, pick),
            files_only: true,
            placeholder: Some(match pick {
                SpringPick::Chain => "Choose the first bone of the chain…",
                SpringPick::Collider => "Choose a body bone for the collider…",
            }),
        });
    }

    /// The sound picker: the project's sound files by name, plus "Import
    /// from file…". Picks a clip for a new entity (`target: None`) or for
    /// entity `target`'s Audio Source.
    pub fn open_sound_picker(&mut self, target: Option<u64>) {
        let root = self.project_root.clone();
        let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
        let mut items: Vec<PaletteItem> = crate::sounds::project_sounds(&root, 2000)
            .into_iter()
            .map(|path| PaletteItem {
                label: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                group: "Sound",
                hint: rel(&path),
                action: PaletteAction::Sound { path, target },
            })
            .collect();
        items.push(PaletteItem {
            label: "Import from file…".into(),
            group: "Import",
            hint: "copies it into sounds/".into(),
            action: PaletteAction::Menu(Action::Ui(super::menus::UiAction::Dialog(
                super::menus::DialogKind::ImportSound(target),
            ))),
        });
        self.menu = None;
        self.picker = None;
        self.palette = Some(Palette {
            query: String::new(),
            selected: 0,
            items,
            files_only: true,
            placeholder: Some(if target.is_some() {
                "Choose a sound for this entity…"
            } else {
                "Add a sound to the scene…"
            }),
        });
    }

    /// The model picker: the project's models (`.glb`, `.gltf`, `.obj`) by
    /// name, plus "Import from file…" for one outside the project. Picks a
    /// model for a new entity (`target: None`) or for entity `target`.
    pub fn open_model_picker(&mut self, target: Option<u64>) {
        let root = self.project_root.clone();
        let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
        let mut items: Vec<PaletteItem> = crate::models::project_models(&root, 2000)
            .into_iter()
            .map(|path| PaletteItem {
                label: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                group: "Model",
                hint: rel(&path),
                action: PaletteAction::Model { path, target },
            })
            .collect();
        items.push(PaletteItem {
            label: "Import from file…".into(),
            group: "Import",
            hint: "copies it into models/".into(),
            action: PaletteAction::Menu(Action::Ui(super::menus::UiAction::Dialog(
                super::menus::DialogKind::ImportModel(target),
            ))),
        });
        self.menu = None;
        self.picker = None;
        self.palette = Some(Palette {
            query: String::new(),
            selected: 0,
            items,
            files_only: true,
            placeholder: Some(if target.is_some() {
                "Choose a model for this entity…"
            } else {
                "Add a model to the scene…"
            }),
        });
    }

    /// The palette's rectangle for a `w`-wide window.
    pub(super) fn palette_rect(&self, w: f32) -> Option<Rect> {
        let p = self.palette.as_ref()?;
        let rows = p.matches().len().clamp(1, ROWS);
        let width = WIDTH.min(w - 40.0);
        Some(Rect::new(
            (w - width) / 2.0,
            56.0,
            width,
            52.0 + rows as f32 * ROW_H + 30.0,
        ))
    }

    /// Draws the palette (and handles its mouse input).
    pub(super) fn draw_palette(&mut self, canvas: &mut Canvas, doc: &Document, w: f32) {
        let Some(r) = self.palette_rect(w) else {
            return;
        };
        let Some(p) = self.palette.clone() else {
            return;
        };
        canvas.fill_rounded(r.x + 3.0, r.y + 8.0, r.w, r.h, 10.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 10.0, color::surface());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 10.0, color::border(), 1.0);

        // Search row.
        let sx = r.x + 22.0;
        let sy = r.y + 26.0;
        canvas.stroke_circle(sx, sy - 1.0, 5.5, color::text_dim(), 1.4);
        canvas.line(
            sx + 4.0,
            sy + 3.0,
            sx + 8.0,
            sy + 7.0,
            color::text_dim(),
            1.4,
        );
        let tx = r.x + 40.0;
        if p.query.is_empty() {
            canvas.text(
                tx,
                r.y + 17.0,
                14.0,
                if let Some(hint) = p.placeholder {
                    hint
                } else if p.files_only {
                    "Go to script by name…"
                } else {
                    "Search commands, entities, scripts, settings…"
                },
                color::text_faint(),
            );
        }
        let qw = canvas.text(tx, r.y + 17.0, 14.0, &p.query, color::text());
        canvas.fill_rect(tx + qw + 1.0, r.y + 16.0, 1.5, 19.0, color::accent());
        canvas.line(r.x, r.y + 50.0, r.x + r.w, r.y + 50.0, color::border(), 1.0);

        // Results.
        let matches = p.matches();
        let raw = self.raw;
        if matches.is_empty() {
            canvas.text(
                r.x + 22.0,
                r.y + 60.0,
                layout::FONT_SIZE,
                "No matches",
                color::text_dim(),
            );
        }
        let start = p.selected.saturating_sub(ROWS - 1);
        let mut run = None;
        for (k, item) in matches.iter().enumerate().skip(start).take(ROWS) {
            let row = Rect::new(
                r.x + 6.0,
                r.y + 54.0 + (k - start) as f32 * ROW_H,
                r.w - 12.0,
                ROW_H - 2.0,
            );
            let hot = row.contains(raw.x, raw.y);
            if k == p.selected {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 6.0, color::select());
            } else if hot {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 6.0, color::hover());
            }
            let tag_w = canvas.measure_text(layout::SMALL_SIZE, item.group) + 14.0;
            canvas.fill_rounded(
                row.x + 12.0,
                row.y + 6.0,
                tag_w,
                row.h - 12.0,
                4.0,
                color::raised(),
            );
            canvas.text(
                row.x + 19.0,
                row.y + 7.5,
                layout::SMALL_SIZE,
                item.group,
                color::text_dim(),
            );
            // Labels share one column whatever the tag's width.
            let col = canvas.measure_text(layout::SMALL_SIZE, "Settings") + 14.0;
            let lx = row.x + 12.0 + col + 12.0;
            let hint_w = canvas
                .measure_text(layout::SMALL_SIZE, &item.hint)
                .min(row.w * 0.4);
            canvas.text_clipped(
                lx,
                row.y + 6.0,
                layout::FONT_SIZE,
                &item.label,
                color::text(),
                row.x + row.w - hint_w - lx - 20.0,
            );
            canvas.text_clipped(
                row.x + row.w - hint_w - 12.0,
                row.y + 8.0,
                layout::SMALL_SIZE,
                &item.hint,
                color::text_faint(),
                hint_w,
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if raw.pressed {
                    run = Some(item.action.clone());
                }
            }
        }
        let footer = "↑↓ navigate   Enter run   Esc close";
        canvas.text(
            r.x + 22.0,
            r.y + r.h - 22.0,
            layout::SMALL_SIZE,
            footer,
            color::text_faint(),
        );
        let count = format!("{} results", matches.len());
        let cw = canvas.measure_text(layout::SMALL_SIZE, &count);
        canvas.text(
            r.x + r.w - cw - 22.0,
            r.y + r.h - 22.0,
            layout::SMALL_SIZE,
            &count,
            color::text_faint(),
        );
        if let Some(action) = run {
            self.palette = None;
            self.run_palette(action, doc);
        }
    }
}

fn settings_title(page: SettingsPage) -> &'static str {
    match page {
        SettingsPage::Appearance => "Appearance",
        SettingsPage::Editor => "Script Editor",
        SettingsPage::Python => "Python",
        SettingsPage::Files => "Files",
        SettingsPage::Build => "Build",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_word_starts_and_runs() {
        assert!(fuzzy_score("ns", "File New Scene").is_some());
        assert!(fuzzy_score("xyz", "File New Scene").is_none());
        let a = fuzzy_score("save", "File Save Scene").unwrap();
        let b = fuzzy_score("save", "Settings: Files and autosave").unwrap();
        assert!(a > b, "{a} vs {b}");
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn palette_filters_and_runs_commands() {
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.open_palette(&doc);
        assert!(ui.palette_open());
        let all = ui.palette.as_ref().unwrap().matches().len();
        assert!(all > 10, "{all}");

        // Entities are listed and selecting one selects it in the scene.
        ui.palette_text("crate 2");
        let first = ui.palette.as_ref().unwrap().matches()[0].label.clone();
        assert!(first.contains("Crate 2"), "{first}");
        ui.palette_key(PaletteKey::Enter, &doc);
        assert!(!ui.palette_open());
        let id = doc
            .entities
            .iter()
            .find(|e| e.name() == "Crate 2")
            .unwrap()
            .id;
        assert!(ui.out.contains(&Intent::Select(Some(id))), "{:?}", ui.out);

        // Settings pages open the Settings dialog.
        ui.open_palette(&doc);
        ui.palette_text("script editor");
        ui.palette_key(PaletteKey::Enter, &doc);
        assert!(matches!(
            ui.dialog,
            Some(super::super::dialogs::Dialog::Settings(
                SettingsPage::Editor
            ))
        ));

        // Escape and Backspace.
        ui.dialog = None;
        ui.open_palette(&doc);
        ui.palette_text("zz");
        ui.palette_key(PaletteKey::Backspace, &doc);
        assert_eq!(ui.palette.as_ref().unwrap().query, "z");
        ui.palette_key(PaletteKey::Escape, &doc);
        assert!(!ui.palette_open());

        // Labels outrank group-name matches; groups still work.
        ui.open_palette(&doc);
        ui.palette_text("sa");
        assert_eq!(ui.palette.as_ref().unwrap().matches()[0].label, "Save");
        ui.palette_key(PaletteKey::Escape, &doc);
        ui.open_palette(&doc);
        ui.palette_text("file save as");
        assert_eq!(ui.palette.as_ref().unwrap().matches()[0].label, "Save As");
    }
}
