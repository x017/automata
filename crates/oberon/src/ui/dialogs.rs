//! Modal dialogs: the in-app file browser (Open / Save As / prefabs /
//! scripts), the unsaved-changes prompt, and the help screens.

use std::path::{Path, PathBuf};

use super::menus::DialogKind;
use super::{ButtonStyle, CursorHint, EditTarget, Intent, Rect, TextEdit, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::theme::{color, layout};

/// One row of the file browser.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    dir: bool,
}

/// An open modal dialog.
#[derive(Debug, Clone)]
pub enum Dialog {
    /// A file browser.
    File(FileDialog),
    /// "Save changes?" before an action that discards the scene.
    Unsaved {
        /// What to do after saving or discarding.
        then: Box<Intent>,
        /// Whether the scene has unsaved changes (else only scripts do).
        scene: bool,
    },
    /// Read-only text (shortcuts, about).
    Message {
        /// Title line.
        title: &'static str,
        /// Body lines.
        lines: Vec<String>,
    },
}

/// File browser state.
#[derive(Debug, Clone)]
pub struct FileDialog {
    kind: DialogKind,
    dir: PathBuf,
    entries: Vec<Entry>,
    scroll: f32,
    error: Option<String>,
    /// A path the user must confirm overwriting (second click on Save).
    overwrite: Option<PathBuf>,
}

impl FileDialog {
    fn title(&self) -> &'static str {
        match self.kind {
            DialogKind::OpenScene => "Open Scene",
            DialogKind::SaveSceneAs => "Save Scene As",
            DialogKind::SavePrefab(_) => "Save Prefab",
            DialogKind::InstantiatePrefab => "Instantiate Prefab",
            DialogKind::OpenScript => "Open Script",
            DialogKind::NewScript => "New Script",
            DialogKind::AttachScript(_) => "Attach Script (pick or name a new .py)",
            DialogKind::Shortcuts | DialogKind::About => "",
        }
    }

    fn extension(&self) -> &'static str {
        match self.kind {
            DialogKind::SavePrefab(_) | DialogKind::InstantiatePrefab => "prefab",
            DialogKind::OpenScript | DialogKind::NewScript | DialogKind::AttachScript(_) => "py",
            _ => "ron",
        }
    }

    fn saves(&self) -> bool {
        matches!(
            self.kind,
            DialogKind::SaveSceneAs | DialogKind::SavePrefab(_) | DialogKind::NewScript
        )
    }

    fn action_label(&self) -> &'static str {
        match self.kind {
            DialogKind::OpenScene | DialogKind::OpenScript => "Open",
            DialogKind::InstantiatePrefab => "Instantiate",
            DialogKind::NewScript => "Create",
            DialogKind::AttachScript(_) => "Attach",
            _ => "Save",
        }
    }

    fn refresh(&mut self) {
        let ext = self.extension();
        let mut entries = Vec::new();
        match std::fs::read_dir(&self.dir) {
            Ok(read) => {
                for e in read.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let dir = e.file_type().is_ok_and(|t| t.is_dir());
                    let matches = Path::new(&name)
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case(ext));
                    if dir || matches {
                        entries.push(Entry { name, dir });
                    }
                }
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", self.dir.display())),
        }
        entries.sort_by(|a, b| {
            b.dir
                .cmp(&a.dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.entries = entries;
        self.scroll = 0.0;
    }

    /// Resolves the typed name to a full path with the right extension.
    fn target(&self, name: &str) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut path = if Path::new(name).is_absolute() {
            PathBuf::from(name)
        } else {
            self.dir.join(name)
        };
        if path.extension().is_none() {
            path.set_extension(self.extension());
        }
        Some(path)
    }
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
        "Play mode",
        "  Ctrl+P play / stop · WASD / arrows move · Space jump · Shift sprint",
        "Code editor",
        "  Ctrl+S save · Ctrl+F find · Ctrl+H replace · Ctrl+G go to line",
        "  Ctrl+/ comment · Tab / Shift+Tab indent · Ctrl+Space complete",
        "  F12 go to definition · Shift+Alt+F format · Ctrl+W close tab",
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
            _ => {
                let in_panel = self.project_dir();
                let mut dir = if matches!(kind, DialogKind::NewScript | DialogKind::OpenScript) {
                    in_panel.clone()
                } else {
                    self.project_root.clone()
                };
                let sub = match kind {
                    DialogKind::SavePrefab(_) | DialogKind::InstantiatePrefab => Some("prefabs"),
                    DialogKind::NewScript
                    | DialogKind::OpenScript
                    | DialogKind::AttachScript(_) => Some("scripts"),
                    _ => None,
                };
                if let Some(sub) = sub {
                    if dir == self.project_root && dir.join(sub).is_dir() {
                        dir = dir.join(sub);
                    }
                }
                let mut fd = FileDialog {
                    kind,
                    dir,
                    entries: Vec::new(),
                    scroll: 0.0,
                    error: None,
                    overwrite: None,
                };
                fd.refresh();
                let suggestion = match kind {
                    DialogKind::SaveSceneAs => {
                        doc.path.as_ref().and_then(|p| p.file_name()).map_or_else(
                            || "scene.ron".to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        )
                    }
                    DialogKind::SavePrefab(id) => doc.index_of(id).map_or_else(
                        || "object.prefab".to_string(),
                        |i| format!("{}.prefab", sanitize(doc.entities[i].name())),
                    ),
                    DialogKind::NewScript => "new_script.py".to_string(),
                    DialogKind::AttachScript(id) => doc.index_of(id).map_or_else(
                        || "script.py".to_string(),
                        |i| format!("{}.py", sanitize(doc.entities[i].name()).to_lowercase()),
                    ),
                    _ => String::new(),
                };
                self.dialog = Some(Dialog::File(fd));
                self.edit = Some(TextEdit {
                    id: 0,
                    target: EditTarget::DialogName,
                    buffer: suggestion,
                    rect: Rect::default(),
                    fresh: false,
                    attr: None,
                });
            }
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

    /// Whether a modal dialog is open.
    #[must_use]
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Closes the open dialog (Escape).
    pub fn close_dialog(&mut self) {
        self.dialog = None;
        if self
            .edit
            .as_ref()
            .is_some_and(|e| e.target == EditTarget::DialogName)
        {
            self.edit = None;
        }
    }

    /// Submits the file dialog with `name` typed in its field. Returns the
    /// resulting intent, or `None` if the dialog stays open.
    pub(super) fn submit_dialog(&mut self, name: &str) -> Option<Intent> {
        let Some(Dialog::File(fd)) = &mut self.dialog else {
            return None;
        };
        // A directory name navigates instead of submitting.
        let candidate = fd.dir.join(name.trim());
        if !name.trim().is_empty() && candidate.is_dir() {
            fd.dir = candidate;
            fd.refresh();
            self.reopen_name_field(String::new());
            return None;
        }
        let Some(path) = fd.target(name) else {
            fd.error = Some("Type a file name".into());
            self.reopen_name_field(name.to_string());
            return None;
        };
        let exists = path.exists();
        if fd.saves()
            && exists
            && fd.overwrite.as_ref() != Some(&path)
            && fd.kind != DialogKind::NewScript
        {
            fd.error = Some(format!(
                "{} exists. Press {} again to replace it.",
                path.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                fd.action_label()
            ));
            fd.overwrite = Some(path);
            self.reopen_name_field(name.to_string());
            return None;
        }
        let needs_existing = matches!(
            fd.kind,
            DialogKind::OpenScene | DialogKind::InstantiatePrefab | DialogKind::OpenScript
        );
        if needs_existing && !exists {
            fd.error = Some(format!("{} not found", path.display()));
            self.reopen_name_field(name.to_string());
            return None;
        }
        let intent = match fd.kind {
            DialogKind::OpenScene => Intent::Open(path),
            DialogKind::SaveSceneAs => Intent::SaveAs(path),
            DialogKind::SavePrefab(id) => Intent::SavePrefab { id, path },
            DialogKind::InstantiatePrefab => Intent::InstantiatePrefab(path),
            DialogKind::OpenScript => Intent::OpenScript(path),
            DialogKind::NewScript => Intent::NewScript(path),
            DialogKind::AttachScript(id) => Intent::AttachScript { id, path },
            DialogKind::Shortcuts | DialogKind::About => return None,
        };
        self.dialog = None;
        Some(intent)
    }

    fn reopen_name_field(&mut self, buffer: String) {
        self.edit = Some(TextEdit {
            id: 0,
            target: EditTarget::DialogName,
            buffer,
            rect: Rect::default(),
            fresh: false,
            attr: None,
        });
    }

    /// The dialog's rectangle for a `w`×`h` window.
    pub(super) fn dialog_rect(&self, w: f32, h: f32) -> Option<Rect> {
        let (dw, dh): (f32, f32) = match self.dialog.as_ref()? {
            Dialog::File(_) => (620.0, 460.0),
            Dialog::Unsaved { .. } => (440.0, 150.0),
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
            Dialog::File(fd) => self.draw_file_dialog(canvas, r, fd),
        }
        self.p = saved_p;
        self.clip = saved_clip;
    }

    #[allow(clippy::too_many_lines)]
    fn draw_file_dialog(&mut self, canvas: &mut Canvas, r: Rect, mut fd: FileDialog) {
        let raw = self.raw;
        canvas.text(
            r.x + 18.0,
            r.y + 14.0,
            15.0,
            fd.title(),
            color::accent_soft(),
        );
        // Path bar with an "up" button.
        let up = Rect::new(r.x + 18.0, r.y + 42.0, 34.0, 24.0);
        let mut navigate: Option<PathBuf> = None;
        if self.button(canvas, up, "↑", ButtonStyle::Normal) {
            navigate = fd.dir.parent().map(Path::to_path_buf);
        }
        let home = Rect::new(r.x + 56.0, r.y + 42.0, 70.0, 24.0);
        if self.button(canvas, home, "Project", ButtonStyle::Normal) {
            navigate = Some(self.project_root.clone());
        }
        let bar = Rect::new(r.x + 132.0, r.y + 42.0, r.w - 150.0, 24.0);
        canvas.fill_rounded(bar.x, bar.y, bar.w, bar.h, 4.0, color::field());
        canvas.text_clipped(
            bar.x + 8.0,
            bar.y + 5.0,
            layout::SMALL_SIZE,
            &fd.dir.display().to_string(),
            color::text_dim(),
            bar.w - 16.0,
        );

        // Listing.
        let list = Rect::new(r.x + 18.0, r.y + 74.0, r.w - 36.0, r.h - 74.0 - 92.0);
        canvas.fill_rounded(list.x, list.y, list.w, list.h, 4.0, color::field());
        canvas.stroke_rounded(list.x, list.y, list.w, list.h, 4.0, color::border(), 1.0);
        if list.contains(raw.x, raw.y) && raw.scroll != 0.0 {
            fd.scroll -= raw.scroll * 40.0;
        }
        let row_h = 22.0;
        let max = (fd.entries.len() as f32 * row_h - list.h + 8.0).max(0.0);
        fd.scroll = fd.scroll.clamp(0.0, max);
        let prev_clip = canvas.set_clip(Some((list.x, list.y + 1.0, list.w, list.h - 2.0)));
        let current_name = self
            .edit
            .as_ref()
            .filter(|e| e.target == EditTarget::DialogName)
            .map(|e| e.buffer.clone())
            .unwrap_or_default();
        let mut set_name = None;
        let mut submit = None;
        let mut y = list.y + 4.0 - fd.scroll;
        for e in &fd.entries {
            let row = Rect::new(list.x + 4.0, y, list.w - 8.0, row_h);
            if row.y + row.h > list.y && row.y < list.y + list.h {
                let hot = row.contains(raw.x, raw.y) && list.contains(raw.x, raw.y);
                let selected = !e.dir && e.name == current_name;
                if selected {
                    canvas.fill_rect(row.x, row.y, row.w, row.h, color::select());
                } else if hot {
                    canvas.fill_rect(row.x, row.y, row.w, row.h, color::hover());
                }
                let (icon, c) = if e.dir {
                    ("▸", color::warn())
                } else {
                    ("•", color::accent_soft())
                };
                canvas.text(row.x + 8.0, row.y + 4.0, layout::FONT_SIZE, icon, c);
                canvas.text(
                    row.x + 26.0,
                    row.y + 4.0,
                    layout::FONT_SIZE,
                    &e.name,
                    color::text(),
                );
                if hot {
                    self.cursor = CursorHint::Pointer;
                    if raw.pressed {
                        if e.dir {
                            navigate = Some(fd.dir.join(&e.name));
                        } else if raw.double_click {
                            submit = Some(e.name.clone());
                        } else {
                            set_name = Some(e.name.clone());
                        }
                    }
                }
            }
            y += row_h;
        }
        if fd.entries.is_empty() {
            canvas.text(
                list.x + 12.0,
                list.y + 12.0,
                layout::SMALL_SIZE,
                &format!("No .{} files here", fd.extension()),
                color::text_dim(),
            );
        }
        canvas.restore_clip(prev_clip);

        // Name field.
        let fy = list.y + list.h + 12.0;
        canvas.text(
            r.x + 18.0,
            fy + 4.0,
            layout::FONT_SIZE,
            "Name",
            color::text_dim(),
        );
        let field = Rect::new(r.x + 70.0, fy, r.w - 88.0, 24.0);
        canvas.fill_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            4.0,
            color::viewport_bg(),
        );
        canvas.stroke_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            4.0,
            color::accent(),
            1.0,
        );
        if self
            .edit
            .as_ref()
            .is_some_and(|e| e.target == EditTarget::DialogName)
        {
            self.draw_edit_text(canvas, field, field.x + 6.0);
        } else {
            self.reopen_name_field(current_name.clone());
        }
        if let Some(err) = &fd.error {
            canvas.text_clipped(
                r.x + 18.0,
                fy + 32.0,
                layout::SMALL_SIZE,
                err,
                color::warn(),
                r.w - 240.0,
            );
        } else {
            canvas.text(
                r.x + 18.0,
                fy + 32.0,
                layout::SMALL_SIZE,
                &format!(
                    "*.{}  ·  double-click to choose, Enter to confirm",
                    fd.extension()
                ),
                color::text_dim(),
            );
        }
        let by = r.y + r.h - 38.0;
        let cancel = Rect::new(r.x + r.w - 214.0, by, 90.0, 26.0);
        if self.button(canvas, cancel, "Cancel", ButtonStyle::Normal) {
            self.close_dialog();
            return;
        }
        let ok = Rect::new(r.x + r.w - 116.0, by, 98.0, 26.0);
        if self.button(canvas, ok, fd.action_label(), ButtonStyle::Accent) {
            submit = Some(current_name);
        }

        if let Some(dir) = navigate {
            fd.dir = dir;
            fd.refresh();
            fd.overwrite = None;
        }
        self.dialog = Some(Dialog::File(fd));
        if let Some(name) = set_name {
            self.reopen_name_field(name);
        }
        if let Some(name) = submit {
            if let Some(intent) = self.submit_dialog(&name) {
                self.edit = None;
                self.out.push(intent);
            }
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
