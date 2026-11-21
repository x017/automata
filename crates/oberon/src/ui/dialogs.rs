//! Modal dialogs: the in-app file browser (Open / Save As / prefabs /
//! scripts), the unsaved-changes prompt, and the help screens.

use std::path::{Path, PathBuf};

use super::menus::DialogKind;
use super::{ButtonStyle, EditTarget, Intent, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::theme::{color, layout};

/// An open modal dialog.
#[derive(Debug, Clone)]
pub enum Dialog {
    /// A file browser.
    File(Box<super::file_browser::FileDialog>),
    /// "Save changes?" before an action that discards the scene.
    Unsaved {
        /// What to do after saving or discarding.
        then: Box<Intent>,
        /// Whether the scene has unsaved changes (else only scripts do).
        scene: bool,
    },
    /// "Move this to the Trash?" before deleting a file or folder.
    ConfirmDelete(PathBuf),
    /// The Settings dialog.
    Settings(super::settings::SettingsPage),
    /// Read-only text (shortcuts, about).
    Message {
        /// Title line.
        title: &'static str,
        /// Body lines.
        lines: Vec<String>,
    },
}

/// The keyboard shortcut reference.
#[must_use]
pub fn shortcut_lines() -> Vec<String> {
    [
        "Scene view",
        "  Click select · Drag / RMB orbit · MMB pan · Wheel zoom",
        "  F / double-click frame selection · Right-click context menu",
        "  W / E / R  move / rotate / scale tool · Arrows, PgUp/PgDn nudge",
        "Editing",
        "  Ctrl+Z / Ctrl+Y undo / redo · Ctrl+C / Ctrl+V copy / paste entity",
        "  Ctrl+D duplicate · Del delete · F2 rename · Esc deselect",
        "  Inspector numbers: drag to scrub (Shift fine, Ctrl coarse), click to type",
        "  Number fields accept math: 2*pi, 90/4, 1.5+2",
        "Files",
        "  Ctrl+N new · Ctrl+O open · Ctrl+S save · Ctrl+Shift+S save as · Ctrl+Q quit",
        "  File browser: type to filter · ↑↓ select · Enter choose · Tab complete",
        "  Backspace (empty name) up a folder · Alt+← / Alt+→ back / forward · F5 refresh",
        "  Files panel: F2 rename · Del delete · Ctrl+D duplicate · Enter open · drag into folders",
        "Play mode",
        "  Ctrl+P play / stop · WASD / arrows move · Space jump · Shift sprint",
        "Build",
        "  Ctrl+Shift+B build the game (Build ▸ Build Settings picks platforms)",
        "Code editor",
        "  Ctrl+S save · Ctrl+F find · Ctrl+H replace · Ctrl+G go to line",
        "  Ctrl+/ comment · Tab / Shift+Tab indent · Ctrl+Space complete",
        "  F12 go to definition · Shift+Alt+F format · Ctrl+E go to file",
        "  Ctrl+D add next match · Ctrl+Shift+L all matches · Alt+click add caret",
        "  Ctrl+Alt+↑/↓ caret above / below · Alt+↑/↓ move line · Shift+Alt+↑/↓ copy line",
        "Tabs",
        "  Ctrl+Tab recent tabs (hold Ctrl) · Ctrl+PgUp/PgDn previous / next · Alt+1…9 go to tab",
        "  Ctrl+Shift+PgUp/PgDn move tab · Ctrl+W / Ctrl+F4 close · Ctrl+Shift+T reopen",
        "  Ctrl+K W close all · Ctrl+K U close saved · Middle-click close · Drag to reorder",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

impl Ui {
    /// Opens a dialog of `kind`.
    pub(super) fn open_dialog(&mut self, kind: DialogKind, doc: &Document) {
        self.menu = None;
        self.picker = None;
        self.edit = None;
        match kind {
            DialogKind::Shortcuts => {
                self.dialog = Some(Dialog::Message {
                    title: "Keyboard Shortcuts",
                    lines: shortcut_lines(),
                });
            }
            DialogKind::About => {
                self.dialog = Some(Dialog::Message {
                    title: "About Oberon",
                    lines: vec![
                        "Oberon — the AUTOMATA scene editor and IDE.".into(),
                        "Native Rust: winit + tiny-skia UI, wgpu viewport, rapier physics.".into(),
                        "Python gameplay scripts run in a bridged interpreter during Play.".into(),
                        String::new(),
                        "Font: Fira Code (SIL Open Font License 1.1).".into(),
                    ],
                });
            }
            _ => self.open_file_dialog(kind, Some(doc)),
        }
    }

    /// Asks whether to save before running `then` (which discards the scene).
    pub fn ask_unsaved(&mut self, then: Intent, scene: bool) {
        self.menu = None;
        self.picker = None;
        self.dialog = Some(Dialog::Unsaved {
            then: Box::new(then),
            scene,
        });
    }

    /// Asks before deleting `path` (Delete in the Files panel).
    pub fn ask_delete(&mut self, path: PathBuf) {
        self.menu = None;
        self.picker = None;
        self.edit = None;
        self.dialog = Some(Dialog::ConfirmDelete(path));
    }

    /// Enter in a dialog without a text field: confirms a deletion,
    /// otherwise closes the dialog.
    pub fn dialog_enter(&mut self) {
        if let Some(Dialog::ConfirmDelete(path)) = self.dialog.take() {
            self.out.push(Intent::DeleteFile(path));
            return;
        }
        self.close_dialog();
    }

    /// Whether a modal dialog is open.
    #[must_use]
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Closes the open dialog (Escape).
    pub fn close_dialog(&mut self) {
        if self.close_file_dialog() {
            return;
        }
        self.dialog = None;
        if self
            .edit
            .as_ref()
            .is_some_and(|e| e.target == EditTarget::DialogName)
        {
            self.edit = None;
        }
    }

    /// The dialog's rectangle for a `w`×`h` window.
    pub(super) fn dialog_rect(&self, w: f32, h: f32) -> Option<Rect> {
        let (dw, dh): (f32, f32) = match self.dialog.as_ref()? {
            Dialog::File(_) => (880.0, 600.0),
            Dialog::Unsaved { .. } => (440.0, 150.0),
            Dialog::ConfirmDelete(_) => (480.0, 160.0),
            Dialog::Settings(_) => (820.0, 600.0),
            Dialog::Message { lines, .. } => (640.0, 90.0 + lines.len() as f32 * 19.0),
        };
        let dw = dw.min(w - 20.0);
        let dh = dh.min(h - 20.0);
        Some(Rect::new((w - dw) / 2.0, (h - dh) / 2.0, dw, dh))
    }

    /// Draws the modal dialog over everything and handles its input.
    #[allow(clippy::too_many_lines)]
    pub(super) fn draw_dialog(&mut self, canvas: &mut Canvas, w: f32, h: f32) {
        let Some(r) = self.dialog_rect(w, h) else {
            return;
        };
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        canvas.fill_rect(0.0, 0.0, w, h, tiny_skia::Color::from_rgba8(0, 0, 0, 110));
        canvas.fill_rounded(r.x + 4.0, r.y + 6.0, r.w, r.h, 10.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 10.0, color::surface());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 10.0, color::border(), 1.0);
        let raw = self.raw;
        // Dialog widgets read the raw pointer (everything else is masked).
        let saved_p = self.p;
        self.p = raw;
        let saved_clip = self.clip;
        self.clip = (0.0, f32::INFINITY);
        match dialog {
            Dialog::Settings(page) => self.draw_settings(canvas, r, page),
            Dialog::Message { title, lines } => {
                canvas.text(r.x + 18.0, r.y + 16.0, 15.0, title, color::accent_soft());
                let mut y = r.y + 46.0;
                for line in &lines {
                    let c = if line.starts_with("  ") {
                        color::text()
                    } else {
                        color::accent_soft()
                    };
                    canvas.text_clipped(r.x + 18.0, y, layout::FONT_SIZE, line, c, r.w - 36.0);
                    y += 19.0;
                }
                let ok = Rect::new(r.x + r.w - 96.0, r.y + r.h - 38.0, 80.0, 26.0);
                if self.button(canvas, ok, "Close", ButtonStyle::Accent) {
                    self.dialog = None;
                }
            }
            Dialog::Unsaved { then, scene } => {
                let scripts = if matches!(*then, Intent::Quit) {
                    self.code.dirty_count()
                } else {
                    0
                };
                let what = match (scene, scripts) {
                    (true, 0) => "Save the scene before continuing?".to_string(),
                    (true, n) => format!("Save the scene and {n} script(s) before quitting?"),
                    (false, n) => format!("Save {n} modified script(s) before quitting?"),
                };
                canvas.text(
                    r.x + 18.0,
                    r.y + 18.0,
                    15.0,
                    "Unsaved changes",
                    color::accent_soft(),
                );
                canvas.text(
                    r.x + 18.0,
                    r.y + 50.0,
                    layout::FONT_SIZE,
                    &what,
                    color::text(),
                );
                let by = r.y + r.h - 42.0;
                if self.button(
                    canvas,
                    Rect::new(r.x + 18.0, by, 90.0, 28.0),
                    "Cancel",
                    ButtonStyle::Normal,
                ) {
                    self.dialog = None;
                }
                if self.button(
                    canvas,
                    Rect::new(r.x + r.w - 236.0, by, 110.0, 28.0),
                    "Don't Save",
                    ButtonStyle::Normal,
                ) {
                    self.dialog = None;
                    self.out.push(Intent::Force(then.clone()));
                }
                if self.button(
                    canvas,
                    Rect::new(r.x + r.w - 118.0, by, 100.0, 28.0),
                    "Save",
                    ButtonStyle::Accent,
                ) {
                    self.dialog = None;
                    if scripts > 0 {
                        self.code.save_all();
                        self.drain_code_events();
                    }
                    if scene {
                        self.out.push(Intent::Save);
                    }
                    if self.code.dirty_count() == 0 || scripts == 0 {
                        self.out.push(Intent::Force(then));
                    }
                }
            }
            Dialog::File(_) => self.draw_file_dialog(canvas, r),
            Dialog::ConfirmDelete(path) => self.draw_confirm_delete(canvas, r, &path),
        }
        self.p = saved_p;
        self.clip = saved_clip;
    }
}

impl Ui {
    fn draw_confirm_delete(&mut self, canvas: &mut Canvas, r: Rect, path: &Path) {
        let name = crate::files::display_name(path);
        let what = if path.is_dir() { "folder" } else { "file" };
        let title = if crate::files::deletes_to_trash() {
            format!("Move the {what} \"{name}\" to the Trash?")
        } else {
            format!("Delete the {what} \"{name}\"?")
        };
        canvas.text_clipped(
            r.x + 18.0,
            r.y + 18.0,
            15.0,
            &title,
            color::accent_soft(),
            r.w - 36.0,
        );
        let mut body = if crate::files::deletes_to_trash() {
            "You can restore it from the Trash later.".to_string()
        } else {
            "This can't be undone.".to_string()
        };
        if path.is_dir() {
            body = format!("Everything inside goes too. {body}");
        }
        canvas.text_clipped(
            r.x + 18.0,
            r.y + 52.0,
            layout::FONT_SIZE,
            &body,
            color::text(),
            r.w - 36.0,
        );
        let by = r.y + r.h - 42.0;
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 290.0, by, 100.0, 28.0),
            "Cancel",
            ButtonStyle::Normal,
        ) {
            self.dialog = None;
        }
        let label = crate::files::delete_label();
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 182.0, by, 164.0, 28.0),
            label,
            ButtonStyle::Accent,
        ) {
            self.dialog = None;
            self.out.push(Intent::DeleteFile(path.to_path_buf()));
        }
    }
}

/// A file-name-safe version of an entity name.
#[must_use]
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "object".into()
    } else {
        s
    }
}
