//! The Files panel: the project directory as a tree, at the bottom of the
//! left sidebar under the Hierarchy.
//!
//! Click a folder to expand or collapse it. Double-click a file to open it:
//! scenes load, scripts open in the code editor, prefabs are instantiated,
//! models are added to the scene. Drag a model onto the scene view to place
//! it there, or onto the Inspector to use it on the selected entity.
//! Right-click offers the same plus creation of scripts and folders in that
//! folder, and Rename / Duplicate / Delete / Copy Path / Show in File
//! Manager. After a click the panel takes the keyboard: arrows move, Enter
//! opens, F2 renames in place, Delete asks and moves to the Trash, Ctrl+D
//! duplicates. Drag a file onto a folder to move it there (scene references
//! follow), or a script onto the Inspector to attach it. The open scene and
//! the active script are highlighted.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::menus::{Action, DialogKind, MenuItem, UiAction};
use super::{ButtonStyle, CursorHint, Icon, Intent, PlayState, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::theme::{color, layout};

/// Row height in the tree.
const ROW_H: f32 = 22.0;
/// Indent per folder level.
const INDENT: f32 = 14.0;
/// How long a folder listing is trusted before it's re-read.
const RELIST: Duration = Duration::from_secs(1);

/// One file or folder in a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    name: String,
    path: PathBuf,
    dir: bool,
}

/// One visible row of the tree.
#[derive(Debug, Clone)]
struct Row {
    item: Item,
    depth: usize,
}

/// Files panel state.
#[derive(Debug, Default)]
pub struct ProjectState {
    /// Expanded folders.
    expanded: BTreeSet<PathBuf>,
    /// Cached listings: folder → (items, when read).
    listings: HashMap<PathBuf, (Vec<Item>, Instant)>,
    scroll: f32,
    pub(super) selected: Option<PathBuf>,
    /// The panel was clicked last, so it takes F2 / Delete / arrow keys.
    focused: bool,
    /// The file or folder being renamed in place.
    renaming: Option<PathBuf>,
    /// The folder a file dragged over the panel would move into.
    drop_dir: Option<PathBuf>,
}

/// A key for the focused Files panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesKey {
    /// Select the row above.
    Up,
    /// Select the row below.
    Down,
    /// Collapse the folder, or select its parent.
    Left,
    /// Expand the folder.
    Right,
    /// Open the file / toggle the folder.
    Open,
    /// Rename in place.
    Rename,
    /// Delete (asks first).
    Delete,
    /// Copy next to itself.
    Duplicate,
    /// Copy the path to the clipboard.
    CopyPath,
}

/// A Files panel command on the selected file (context menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCommand {
    /// Rename in place.
    Rename,
    /// Delete (asks first).
    Delete,
    /// Copy the absolute path.
    CopyPath,
    /// Copy the path relative to the project.
    CopyRelativePath,
}

/// A file dragged out of the Files panel.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDrag {
    /// The file.
    pub path: PathBuf,
    /// Where the press happened (the drag starts after a few pixels).
    pub origin: (f32, f32),
    /// Whether the pointer moved far enough to count as a drag.
    pub active: bool,
}

/// What kind of asset a file is (drives icon, color, and open action).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// A folder.
    Folder,
    /// A `.ron` scene.
    Scene,
    /// A `.prefab`.
    Prefab,
    /// A `.py` script.
    Script,
    /// A 3D model (`.glb`, `.gltf`, `.obj`).
    Model,
    /// A sound (`.ogg`, `.wav`, `.mp3`, `.flac`).
    Audio,
    /// Anything else.
    Other,
}

/// Classifies a path by extension.
#[must_use]
pub fn asset_kind(path: &Path, dir: bool) -> AssetKind {
    if dir {
        return AssetKind::Folder;
    }
    // A model's import settings (`tree.glb.import.ron`) aren't a scene.
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".import.ron"))
    {
        return AssetKind::Other;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("ron") => AssetKind::Scene,
        Some("prefab") => AssetKind::Prefab,
        Some("py") => AssetKind::Script,
        Some("glb" | "gltf" | "obj") => AssetKind::Model,
        Some("ogg" | "wav" | "mp3" | "flac") => AssetKind::Audio,
        _ => AssetKind::Other,
    }
}

/// The intent that "opening" an asset performs, if any.
#[must_use]
pub fn open_intent(path: &Path, dir: bool) -> Option<Intent> {
    match asset_kind(path, dir) {
        AssetKind::Scene => Some(Intent::Open(path.to_path_buf())),
        AssetKind::Prefab => Some(Intent::InstantiatePrefab(path.to_path_buf())),
        AssetKind::Script => Some(Intent::OpenScript(path.to_path_buf())),
        AssetKind::Model => Some(Intent::AddModel {
            path: path.to_path_buf(),
            at: None,
        }),
        AssetKind::Audio => Some(Intent::PreviewSound(path.to_path_buf())),
        AssetKind::Folder | AssetKind::Other => None,
    }
}

fn list(dir: &Path) -> Vec<Item> {
    let mut items: Vec<Item> = std::fs::read_dir(dir)
        .map(|read| {
            read.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let skip = name.starts_with('.')
                        || matches!(name.as_str(), "target" | "__pycache__" | "node_modules");
                    (!skip).then(|| Item {
                        dir: e.file_type().is_ok_and(|t| t.is_dir()),
                        path: e.path(),
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    items.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    items
}

/// Same file, comparing canonical paths when both exist.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

impl ProjectState {
    /// The listing of `dir`, re-read when older than [`RELIST`].
    fn listing(&mut self, dir: &Path) -> Vec<Item> {
        let fresh = self
            .listings
            .get(dir)
            .is_some_and(|(_, t)| t.elapsed() < RELIST);
        if !fresh {
            self.listings
                .insert(dir.to_path_buf(), (list(dir), Instant::now()));
        }
        self.listings[dir].0.clone()
    }

    /// The visible rows: a depth-first walk through expanded folders.
    fn rows(&mut self, root: &Path) -> Vec<Row> {
        fn walk(state: &mut ProjectState, dir: &Path, depth: usize, out: &mut Vec<Row>) {
            for item in state.listing(dir) {
                let open = item.dir && state.expanded.contains(&item.path);
                let path = item.path.clone();
                out.push(Row { item, depth });
                if open && depth < 32 {
                    walk(state, &path, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, root, 0, &mut out);
        out
    }

    /// Forgets cached listings so the next frame re-reads the disk.
    fn invalidate(&mut self) {
        self.listings.clear();
    }
}

impl Ui {
    /// The folder new files go into: the selected folder, the selected
    /// file's folder, or the project root.
    pub(super) fn project_dir(&self) -> PathBuf {
        match &self.project.selected {
            Some(p) if p.is_dir() => p.clone(),
            Some(p) => p
                .parent()
                .filter(|d| d.starts_with(&self.project_root))
                .map_or_else(|| self.project_root.clone(), Path::to_path_buf),
            None => self.project_root.clone(),
        }
    }

    /// Expands every folder from the project root down to `path` and
    /// selects it, so it's visible in the Files panel.
    pub fn reveal_in_files(&mut self, path: &Path) {
        let Ok(rel) = path.strip_prefix(&self.project_root) else {
            return;
        };
        let mut acc = self.project_root.clone();
        let parts: Vec<_> = rel.components().collect();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            acc.push(part);
            self.project.expanded.insert(acc.clone());
        }
        self.project.selected = Some(path.to_path_buf());
        self.panels.show_files = true;
    }

    /// Draws the Files panel.
    #[allow(clippy::too_many_lines)]
    pub(super) fn files_panel(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x + rect.w - 1.0, rect.y, 1.0, rect.h, color::border());
        if self.panels.show_hierarchy {
            canvas.fill_rect(rect.x, rect.y, rect.w, 1.0, color::border());
        }
        let root = self.project_root.clone();
        let editing = self.play == PlayState::Editing;
        let open_scene = doc.path.clone();
        let open_script = self
            .code
            .active
            .and_then(|i| self.code.tabs.get(i))
            .map(|t| t.path.clone());

        let top = rect.y + layout::TITLEBAR_H;
        let list = Rect::new(rect.x, top, rect.w, (rect.h - layout::TITLEBAR_H).max(0.0));
        self.last_files_h = rect.h;
        // Focus follows clicks: inside takes the keyboard, outside gives it
        // back.
        if self.p.pressed || self.p.right_pressed {
            self.project.focused = self.hot(rect);
        }
        let dragging = self
            .file_drag
            .as_ref()
            .filter(|d| d.active)
            .map(|d| d.path.clone());
        self.project.drop_dir = None;
        let renaming = self.project.renaming.clone().filter(|_| {
            self.edit
                .as_ref()
                .is_some_and(|e| e.target == super::EditTarget::FileRename)
        });
        if renaming.is_none() {
            self.project.renaming = None;
        }
        let rows = self.project.rows(&root);
        let content_h = rows.len() as f32 * ROW_H + 8.0;
        if self.hot(list) && self.p.scroll != 0.0 {
            self.project.scroll -= self.p.scroll * ROW_H * 2.0;
        }
        self.project.scroll = self
            .project
            .scroll
            .clamp(0.0, (content_h - list.h).max(0.0));

        let prev = canvas.set_clip(Some((list.x, list.y, list.w, list.h)));
        let saved_clip = self.clip;
        self.clip = (list.y, list.y + list.h);
        let mut clicked_row = false;
        let mut toggle = None;
        let mut y = top + 4.0 - self.project.scroll;
        for row in &rows {
            let r = Rect::new(rect.x + 4.0, y, rect.w - 9.0, ROW_H);
            y += ROW_H;
            if !self.row_visible(r) {
                continue;
            }
            let item = &row.item;
            let kind = asset_kind(&item.path, item.dir);
            let selected = self.project.selected.as_ref() == Some(&item.path);
            let open = !item.dir
                && [&open_scene, &open_script]
                    .into_iter()
                    .flatten()
                    .any(|p| same_file(p, &item.path));
            let hot = self.hot(r);
            if selected {
                canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::select());
            } else if hot {
                canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::hover());
            }
            // Indent guides keep deep trees readable.
            for d in 0..row.depth {
                let gx = r.x + 10.0 + d as f32 * INDENT;
                canvas.fill_rect(gx, r.y, 1.0, r.h, color::border());
            }
            let x = r.x + 4.0 + row.depth as f32 * INDENT;
            if item.dir {
                draw_chevron(
                    canvas,
                    x + 6.0,
                    r.y + r.h / 2.0,
                    self.project.expanded.contains(&item.path),
                );
            }
            draw_tree_icon(canvas, kind, x + 21.0, r.y + r.h / 2.0);
            let tx = x + 32.0;
            // A file dragged over a row moves into that folder (a file row:
            // its folder).
            if let Some(dragged) = dragging.as_ref().filter(|_| hot) {
                let into = if item.dir {
                    item.path.clone()
                } else {
                    item.path
                        .parent()
                        .map_or_else(|| root.clone(), Path::to_path_buf)
                };
                if dragged.parent() != Some(into.as_path()) && dragged != &into {
                    canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.5);
                    self.project.drop_dir = Some(into);
                }
            }
            if renaming.as_ref() == Some(&item.path) {
                let field = Rect::new(tx - 4.0, r.y + 1.0, (r.x + r.w - tx).max(40.0), r.h - 2.0);
                canvas.fill_rounded(
                    field.x,
                    field.y,
                    field.w,
                    field.h,
                    3.0,
                    color::viewport_bg(),
                );
                canvas.stroke_rounded(
                    field.x,
                    field.y,
                    field.w,
                    field.h,
                    3.0,
                    color::accent(),
                    1.0,
                );
                self.draw_edit_text(canvas, field, tx);
                let (_, ext) = crate::files::split_name(&item.path);
                let typed = self.edit.as_ref().map_or("", |e| e.buffer.as_str());
                if !ext.is_empty() && Path::new(typed).extension().is_none() {
                    let tw = canvas.measure_text(layout::FONT_SIZE, typed);
                    canvas.text(
                        tx + tw + 3.0,
                        r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
                        layout::FONT_SIZE,
                        &ext,
                        color::text_faint(),
                    );
                }
                continue;
            }
            canvas.text_clipped(
                tx,
                r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
                layout::FONT_SIZE,
                &item.name,
                if selected || open {
                    color::text()
                } else {
                    color::text_dim()
                },
                (r.x + r.w - tx - 14.0).max(0.0),
            );
            if open {
                canvas.fill_circle(r.x + r.w - 8.0, r.y + r.h / 2.0, 2.5, color::accent());
            }
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    clicked_row = true;
                    self.project.selected = Some(item.path.clone());
                    if !item.dir && editing && !self.p.double_click {
                        self.file_drag = Some(FileDrag {
                            path: item.path.clone(),
                            origin: (self.p.x, self.p.y),
                            active: false,
                        });
                    }
                    if item.dir {
                        toggle = Some(item.path.clone());
                    } else if self.p.double_click {
                        if let Some(intent) = open_intent(&item.path, false) {
                            self.out.push(intent);
                        }
                    }
                }
                if self.p.right_pressed {
                    clicked_row = true;
                    self.project.selected = Some(item.path.clone());
                    let mut menu = self.file_menu(doc, item, kind, editing);
                    menu.extend(self.file_ops_items(item, editing));
                    self.open_menu(self.p.x, self.p.y, menu);
                }
            }
        }
        if rows.is_empty() {
            canvas.text(
                rect.x + layout::PAD,
                top + 10.0,
                layout::SMALL_SIZE,
                "No files yet. Right-click to add a script.",
                color::text_dim(),
            );
        }
        // Dropping on empty space moves to the project folder.
        if let Some(dragged) = dragging.as_ref() {
            if self.project.drop_dir.is_none()
                && self.hot(list)
                && dragged.parent() != Some(root.as_path())
                && y < list.y + list.h
                && self.p.y > y
            {
                canvas.stroke_rounded(
                    list.x + 2.0,
                    list.y + 2.0,
                    list.w - 5.0,
                    list.h - 4.0,
                    4.0,
                    color::accent(),
                    1.5,
                );
                self.project.drop_dir = Some(root.clone());
            }
        }
        if !clicked_row && self.hot(list) {
            if self.p.pressed {
                self.project.selected = None;
            }
            if self.p.right_pressed {
                self.project.selected = None;
                let menu = self.project_create_items();
                self.open_menu(self.p.x, self.p.y, menu);
            }
        }
        self.clip = saved_clip;
        canvas.restore_clip(prev);
        if let Some(path) = toggle {
            if !self.project.expanded.remove(&path) {
                self.project.expanded.insert(path);
            }
        }

        // Title bar: "Files", the project folder, and quick actions.
        self.panel_title(canvas, rect, "Files");
        let title_w = canvas.measure_text(layout::TITLE_SIZE, "Files");
        let name = root.file_name().map_or_else(
            || root.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let bw = layout::TITLEBAR_H - 8.0;
        let name_x = rect.x + layout::PAD + title_w + 8.0;
        canvas.text_clipped(
            name_x,
            rect.y + (layout::TITLEBAR_H - layout::TITLE_SIZE) / 2.0 - 1.0,
            layout::TITLE_SIZE,
            &name,
            color::text_faint(),
            (rect.x + rect.w - name_x - 2.0 * bw - 16.0).max(0.0),
        );
        let add = Rect::new(rect.x + rect.w - layout::TITLEBAR_H, rect.y + 4.0, bw, bw);
        self.tip(add, "New script or folder");
        if self.icon_button(canvas, add, Icon::Plus, ButtonStyle::Ghost) {
            let items = self.project_create_items();
            self.open_menu(add.x, add.y + add.h + 2.0, items);
        }
        let collapse = Rect::new(add.x - bw - 2.0, add.y, bw, bw);
        self.tip(collapse, "Collapse all folders");
        let any_open = !self.project.expanded.is_empty();
        let style = if any_open {
            ButtonStyle::Ghost
        } else {
            ButtonStyle::Disabled
        };
        if self.button(canvas, collapse, "", style) {
            self.project.expanded.clear();
            self.project.scroll = 0.0;
        }
        draw_collapse_icon(
            canvas,
            collapse.x + collapse.w / 2.0,
            collapse.y + collapse.h / 2.0,
            if !any_open {
                color::text_faint()
            } else if self.hot(collapse) {
                color::text()
            } else {
                color::text_dim()
            },
        );
    }

    /// The right-click menu for one file or folder.
    /// The context menu of an asset file (the Assets tab's tiles).
    pub(super) fn asset_menu(&self, doc: &Document, path: &Path, editing: bool) -> Vec<MenuItem> {
        let item = Item {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_path_buf(),
            dir: false,
        };
        self.file_menu(doc, &item, asset_kind(path, false), editing)
    }

    fn file_menu(
        &self,
        doc: &Document,
        item: &Item,
        kind: AssetKind,
        editing: bool,
    ) -> Vec<MenuItem> {
        let mut menu = Vec::new();
        match kind {
            AssetKind::Scene => menu.push(item_menu("Open Scene", open_intent(&item.path, false))),
            AssetKind::Prefab => {
                menu.push(item_menu("Instantiate", open_intent(&item.path, false)));
            }
            AssetKind::Script => {
                menu.push(item_menu("Edit Script", open_intent(&item.path, false)));
                if let Some(i) = doc.selected_index() {
                    menu.push(MenuItem {
                        label: format!("Attach to {}", doc.entities[i].name()),
                        shortcut: "",
                        action: Some(Action::Intent(Intent::AttachScript {
                            id: doc.entities[i].id,
                            path: item.path.clone(),
                        })),
                        enabled: editing,
                    });
                }
            }
            AssetKind::Model => {
                menu.push(MenuItem {
                    label: "Add to Scene".into(),
                    shortcut: "",
                    action: open_intent(&item.path, false).map(Action::Intent),
                    enabled: editing,
                });
                if let Some(i) = doc.selected_index() {
                    menu.push(MenuItem {
                        label: format!("Use on {}", doc.entities[i].name()),
                        shortcut: "",
                        action: Some(Action::Intent(Intent::SetModel {
                            id: doc.entities[i].id,
                            path: item.path.clone(),
                        })),
                        enabled: editing,
                    });
                }
            }
            AssetKind::Audio => {
                menu.push(item_menu("Preview", open_intent(&item.path, false)));
                menu.push(MenuItem {
                    label: "Add to Scene".into(),
                    shortcut: "",
                    action: Some(Action::Intent(Intent::AddSound {
                        path: item.path.clone(),
                        at: None,
                    })),
                    enabled: editing,
                });
                if let Some(i) = doc.selected_index() {
                    menu.push(MenuItem {
                        label: format!("Play from {}", doc.entities[i].name()),
                        shortcut: "",
                        action: Some(Action::Intent(Intent::SetAudioClip {
                            id: doc.entities[i].id,
                            path: item.path.clone(),
                        })),
                        enabled: editing,
                    });
                }
            }
            AssetKind::Folder | AssetKind::Other => {}
        }
        if !menu.is_empty() {
            menu.push(MenuItem::separator());
        }
        menu.extend(self.project_create_items());
        menu
    }

    /// Rename / Duplicate / Delete / Copy Path / Show in File Manager for a
    /// tree row (acting on the selection, which the right-click set).
    fn file_ops_items(&self, item: &Item, editing: bool) -> Vec<MenuItem> {
        let file = |label: &str, shortcut: &'static str, cmd: FileCommand, on: bool| {
            MenuItem::new(label, shortcut, Action::Ui(UiAction::File(cmd)), on)
        };
        vec![
            MenuItem::separator(),
            file("Rename…", "F2", FileCommand::Rename, editing),
            MenuItem::new(
                "Duplicate",
                "Ctrl+D",
                Action::Intent(Intent::DuplicateFile(item.path.clone())),
                true,
            ),
            file(
                crate::files::delete_label(),
                "Del",
                FileCommand::Delete,
                true,
            ),
            MenuItem::separator(),
            file("Copy Path", "Ctrl+Shift+C", FileCommand::CopyPath, true),
            file(
                "Copy Relative Path",
                "",
                FileCommand::CopyRelativePath,
                true,
            ),
            MenuItem::new(
                "Show in File Manager",
                "",
                Action::Intent(Intent::RevealFile(item.path.clone())),
                true,
            ),
        ]
    }

    /// The folder a file dragged over the Files panel would move into.
    pub(super) fn project_drop_dir(&self) -> Option<PathBuf> {
        self.project.drop_dir.clone()
    }

    /// Whether the Files panel has the keyboard.
    #[must_use]
    pub fn files_focused(&self) -> bool {
        self.project.focused
            && self.panels.show_files
            && self.edit.is_none()
            && self.dialog.is_none()
            && self.menu.is_none()
            && !self.code.has_focus()
    }

    /// Runs a context-menu command on the selected file.
    pub(super) fn file_command(&mut self, cmd: FileCommand) {
        let Some(path) = self.project.selected.clone() else {
            return;
        };
        match cmd {
            FileCommand::Rename => self.begin_file_rename(&path),
            FileCommand::Delete => self.ask_delete(path),
            FileCommand::CopyPath => self.code.clipboard.set(&path.display().to_string()),
            FileCommand::CopyRelativePath => {
                let rel = path.strip_prefix(&self.project_root).unwrap_or(&path);
                let text = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                self.code.clipboard.set(&text);
            }
        }
    }

    /// Starts renaming `path` in place: the name without its extension is
    /// selected, so typing replaces it and the extension stays.
    pub fn begin_file_rename(&mut self, path: &Path) {
        if path == self.project_root {
            return;
        }
        let (stem, _) = crate::files::split_name(path);
        self.project.renaming = Some(path.to_path_buf());
        self.project.selected = Some(path.to_path_buf());
        self.edit = Some(super::TextEdit {
            id: 0,
            target: super::EditTarget::FileRename,
            buffer: stem,
            rect: Rect::default(),
            fresh: true,
            attr: None,
        });
    }

    /// Commits an in-place rename typed as `text`. A name without an
    /// extension keeps the file's own.
    pub(super) fn commit_file_rename(&mut self, text: &str) -> Option<Intent> {
        let from = self.project.renaming.take()?;
        let name = text.trim();
        if let Err(e) = crate::files::check_name(name) {
            self.log(super::LogLevel::Warn, format!("Rename: {e}"));
            return None;
        }
        let (stem, ext) = crate::files::split_name(&from);
        let new_name = if !ext.is_empty() && Path::new(name).extension().is_none() {
            format!("{name}{ext}")
        } else {
            name.to_string()
        };
        if new_name == format!("{stem}{ext}") {
            return None;
        }
        let to = from.with_file_name(new_name);
        Some(Intent::MoveFile { from, to })
    }

    /// Handles a key while the Files panel has focus.
    pub fn files_key(&mut self, key: FilesKey, editing: bool) -> Option<Intent> {
        let root = self.project_root.clone();
        let rows = self.project.rows(&root);
        let pos = self
            .project
            .selected
            .as_ref()
            .and_then(|s| rows.iter().position(|r| &r.item.path == s));
        let current = pos.map(|i| rows[i].item.clone());
        match key {
            FilesKey::Up | FilesKey::Down => {
                if rows.is_empty() {
                    return None;
                }
                let next = match (key, pos) {
                    (FilesKey::Down, Some(i)) => (i + 1).min(rows.len() - 1),
                    (FilesKey::Up, Some(i)) => i.saturating_sub(1),
                    (FilesKey::Down, None) => 0,
                    _ => rows.len() - 1,
                };
                self.project.selected = Some(rows[next].item.path.clone());
                self.reveal_row(next);
                None
            }
            FilesKey::Left => {
                let item = current?;
                if item.dir && self.project.expanded.remove(&item.path) {
                    return None;
                }
                let parent = item.path.parent()?.to_path_buf();
                if parent.starts_with(&root) && parent != root {
                    self.project.expanded.remove(&parent);
                    if let Some(i) = rows.iter().position(|r| r.item.path == parent) {
                        self.reveal_row(i);
                    }
                    self.project.selected = Some(parent);
                }
                None
            }
            FilesKey::Right => {
                let item = current?;
                if item.dir {
                    self.project.expanded.insert(item.path);
                }
                None
            }
            FilesKey::Open => {
                let item = current?;
                if item.dir {
                    if !self.project.expanded.remove(&item.path) {
                        self.project.expanded.insert(item.path);
                    }
                    return None;
                }
                open_intent(&item.path, false)
            }
            FilesKey::Rename => {
                if editing {
                    self.begin_file_rename(&current?.path);
                }
                None
            }
            FilesKey::Delete => {
                self.ask_delete(current?.path);
                None
            }
            FilesKey::Duplicate => Some(Intent::DuplicateFile(current?.path)),
            FilesKey::CopyPath => {
                self.code
                    .clipboard
                    .set(&current?.path.display().to_string());
                None
            }
        }
    }

    /// Scrolls the tree so row `i` is visible.
    fn reveal_row(&mut self, i: usize) {
        let top = i as f32 * ROW_H;
        let view = (self.last_files_h - layout::TITLEBAR_H - 8.0).max(ROW_H);
        if top < self.project.scroll {
            self.project.scroll = top;
        } else if top + ROW_H > self.project.scroll + view {
            self.project.scroll = top + ROW_H - view;
        }
    }

    /// "New Script…" and "New Folder" in [`Ui::project_dir`].
    fn project_create_items(&self) -> Vec<MenuItem> {
        let dir = self.project_dir();
        vec![
            MenuItem {
                label: "New Script…".into(),
                shortcut: "",
                action: Some(Action::Ui(UiAction::Dialog(DialogKind::NewScript))),
                enabled: true,
            },
            MenuItem {
                label: "New Folder".into(),
                shortcut: "",
                action: Some(Action::Intent(Intent::NewFolder(
                    dir.join(unique_child(&dir, "New Folder")),
                ))),
                enabled: true,
            },
        ]
    }

    /// Called after the app handles an intent that changes files on disk:
    /// re-reads listings and reveals what was created.
    pub fn files_changed(&mut self, created: Option<&Path>) {
        self.project.invalidate();
        self.assets.invalidate();
        if let Some(path) = created {
            self.reveal_in_files(path);
        }
    }
}

fn item_menu(label: &str, intent: Option<Intent>) -> MenuItem {
    MenuItem {
        label: label.into(),
        shortcut: "",
        enabled: intent.is_some(),
        action: intent.map(Action::Intent),
    }
}

/// `base`, or `base 2`, `base 3`, … whichever does not exist in `dir`.
#[must_use]
pub fn unique_child(dir: &Path, base: &str) -> String {
    if !dir.join(base).exists() {
        return base.to_string();
    }
    (2..1000)
        .map(|i| format!("{base} {i}"))
        .find(|n| !dir.join(n).exists())
        .unwrap_or_else(|| base.to_string())
}

/// A small disclosure triangle: pointing right (closed) or down (open).
fn draw_chevron(canvas: &mut Canvas, cx: f32, cy: f32, open: bool) {
    let c = color::text_dim();
    if open {
        canvas.fill_polygon(
            &[(cx - 3.5, cy - 2.0), (cx + 3.5, cy - 2.0), (cx, cy + 2.5)],
            c,
        );
    } else {
        canvas.fill_polygon(
            &[(cx - 2.0, cy - 3.5), (cx + 2.5, cy), (cx - 2.0, cy + 3.5)],
            c,
        );
    }
}

/// "Collapse all": two chevrons pointing at a bar.
fn draw_collapse_icon(canvas: &mut Canvas, cx: f32, cy: f32, c: tiny_skia::Color) {
    canvas.polyline(
        &[(cx - 4.0, cy - 6.0), (cx, cy - 2.5), (cx + 4.0, cy - 6.0)],
        c,
        1.3,
    );
    canvas.polyline(
        &[(cx - 4.0, cy + 6.0), (cx, cy + 2.5), (cx + 4.0, cy + 6.0)],
        c,
        1.3,
    );
    canvas.fill_rect(cx - 5.0, cy - 0.5, 10.0, 1.2, c);
}

/// A 14 px icon for an asset kind, centred on `(cx, cy)`. Hue only where it
/// carries meaning: scripts green (Python), scenes and prefabs paper white.
pub fn draw_tree_icon(canvas: &mut Canvas, kind: AssetKind, cx: f32, cy: f32) {
    match kind {
        AssetKind::Folder => super::draw_small_file_icon(canvas, cx, cy, true),
        AssetKind::Scene => {
            let c = color::accent_soft();
            canvas.fill_polygon(
                &[
                    (cx, cy - 6.0),
                    (cx + 6.0, cy),
                    (cx, cy + 6.0),
                    (cx - 6.0, cy),
                ],
                c,
            );
            canvas.fill_polygon(
                &[
                    (cx, cy - 2.5),
                    (cx + 2.5, cy),
                    (cx, cy + 2.5),
                    (cx - 2.5, cy),
                ],
                color::surface(),
            );
        }
        AssetKind::Prefab => {
            canvas.stroke_rounded(
                cx - 5.5,
                cy - 5.5,
                11.0,
                11.0,
                2.0,
                color::accent_soft(),
                1.3,
            );
            canvas.fill_rounded(cx - 2.0, cy - 2.0, 4.0, 4.0, 1.0, color::accent_soft());
        }
        AssetKind::Script => {
            canvas.stroke_rounded(cx - 5.0, cy - 6.5, 10.0, 13.0, 2.0, color::axis_y(), 1.2);
            canvas.fill_rect(cx - 2.5, cy - 2.5, 5.0, 1.2, color::axis_y());
            canvas.fill_rect(cx - 2.5, cy + 0.5, 3.5, 1.2, color::axis_y());
        }
        AssetKind::Model => draw_model_icon(canvas, cx, cy, color::accent_soft()),
        AssetKind::Audio => draw_sound_icon(canvas, cx, cy, color::accent_soft()),
        AssetKind::Other => super::draw_small_file_icon(canvas, cx, cy, false),
    }
}

/// A speaker with one sound wave: the sound file icon.
pub fn draw_sound_icon(canvas: &mut Canvas, cx: f32, cy: f32, c: tiny_skia::Color) {
    canvas.fill_polygon(
        &[
            (cx - 6.0, cy - 2.0),
            (cx - 3.5, cy - 2.0),
            (cx + 0.5, cy - 5.5),
            (cx + 0.5, cy + 5.5),
            (cx - 3.5, cy + 2.0),
            (cx - 6.0, cy + 2.0),
        ],
        c,
    );
    canvas.polyline(
        &[
            (cx + 3.0, cy - 3.5),
            (cx + 4.6, cy - 1.5),
            (cx + 5.0, cy),
            (cx + 4.6, cy + 1.5),
            (cx + 3.0, cy + 3.5),
        ],
        c,
        1.3,
    );
}

/// A wireframe cube in two-point view: the model file icon.
pub fn draw_model_icon(canvas: &mut Canvas, cx: f32, cy: f32, c: tiny_skia::Color) {
    let top = (cx, cy - 6.0);
    let left = (cx - 5.5, cy - 3.0);
    let right = (cx + 5.5, cy - 3.0);
    let mid = (cx, cy);
    canvas.polyline(&[left, top, right, mid, left], c, 1.2);
    canvas.polyline(
        &[
            left,
            (cx - 5.5, cy + 3.0),
            (cx, cy + 6.0),
            (cx + 5.5, cy + 3.0),
            right,
        ],
        c,
        1.2,
    );
    canvas.polyline(&[mid, (cx, cy + 6.0)], c, 1.2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Pointer;

    #[test]
    fn assets_open_with_the_right_action() {
        assert_eq!(
            asset_kind(Path::new("a/level.ron"), false),
            AssetKind::Scene
        );
        assert_eq!(
            asset_kind(Path::new("enemy.PREFAB"), false),
            AssetKind::Prefab
        );
        assert_eq!(asset_kind(Path::new("player.py"), false), AssetKind::Script);
        assert_eq!(asset_kind(Path::new("tree.GLB"), false), AssetKind::Model);
        assert!(matches!(
            open_intent(Path::new("rock.obj"), false),
            Some(Intent::AddModel { at: None, .. })
        ));
        assert_eq!(asset_kind(Path::new("scripts"), true), AssetKind::Folder);
        assert!(matches!(
            open_intent(Path::new("x.py"), false),
            Some(Intent::OpenScript(_))
        ));
        assert!(matches!(
            open_intent(Path::new("x.prefab"), false),
            Some(Intent::InstantiatePrefab(_))
        ));
        assert!(open_intent(Path::new("readme.md"), false).is_none());
    }

    /// A throwaway project: `level.ron`, `scripts/coin.py`, `scripts/ai/`.
    fn project(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("oberon-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("scripts/ai")).unwrap();
        std::fs::write(root.join("level.ron"), "()").unwrap();
        std::fs::write(root.join("scripts/coin.py"), "").unwrap();
        std::fs::write(root.join(".hidden"), "").unwrap();
        root
    }

    fn press(
        ui: &mut Ui,
        c: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        double: bool,
    ) -> Vec<Intent> {
        let mut out = ui.frame(
            c,
            doc,
            Pointer {
                x,
                y,
                down: true,
                pressed: true,
                double_click: double,
                ..Pointer::default()
            },
            None,
        );
        out.extend(ui.frame(
            c,
            doc,
            Pointer {
                x,
                y,
                ..Pointer::default()
            },
            None,
        ));
        out
    }

    /// The centre of visible tree row `i` in a 1600×950 window.
    fn row_center(i: usize) -> (f32, f32) {
        let files = super::super::Layout::compute(1600.0, 950.0).files;
        (
            files.x + 60.0,
            files.y + layout::TITLEBAR_H + 4.0 + (i as f32 + 0.5) * ROW_H,
        )
    }

    #[test]
    fn the_files_panel_sits_under_the_hierarchy() {
        let lay = super::super::Layout::compute(1600.0, 950.0);
        assert_eq!(lay.files.x, lay.hierarchy.x);
        assert_eq!(lay.files.w, lay.hierarchy.w);
        assert!((lay.files.y - (lay.hierarchy.y + lay.hierarchy.h)).abs() < 0.5);
        assert!((lay.files.y + lay.files.h - lay.status.y).abs() < 0.5);
        assert!((lay.files.h - layout::FILES_H).abs() < 0.5);

        // Hiding the hierarchy gives Files the whole sidebar, and back.
        let mut panels = super::super::PanelSizes {
            show_hierarchy: false,
            ..Default::default()
        };
        let lay = super::super::Layout::with_panels(1600.0, 950.0, &panels);
        assert_eq!(lay.hierarchy.h, 0.0);
        assert!((lay.files.y - lay.menubar.h).abs() < 0.5);
        panels.show_hierarchy = true;
        panels.show_files = false;
        let lay = super::super::Layout::with_panels(1600.0, 950.0, &panels);
        assert_eq!(lay.files.h, 0.0);
        assert!((lay.hierarchy.y + lay.hierarchy.h - lay.status.y).abs() < 0.5);
        panels.show_hierarchy = false;
        let lay = super::super::Layout::with_panels(1600.0, 950.0, &panels);
        assert_eq!(lay.viewport.x, 0.0, "both hidden: no sidebar");
    }

    #[test]
    fn folders_expand_in_place_and_files_open_on_double_click() {
        let root = project("tree");
        let mut c = Canvas::new(1600, 950);
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root.clone_from(&root);
        ui.frame(&mut c, &doc, Pointer::default(), None);
        // Folders first, dotfiles hidden: [scripts/, level.ron].
        let names: Vec<String> = ui
            .project
            .rows(&root)
            .into_iter()
            .map(|r| r.item.name)
            .collect();
        assert_eq!(names, ["scripts", "level.ron"]);

        let (x, y) = row_center(0);
        press(&mut ui, &mut c, &doc, x, y, false);
        let rows = ui.project.rows(&root);
        let names: Vec<(&str, usize)> = rows
            .iter()
            .map(|r| (r.item.name.as_str(), r.depth))
            .collect();
        assert_eq!(
            names,
            [("scripts", 0), ("ai", 1), ("coin.py", 1), ("level.ron", 0)]
        );

        // New files go into the selected folder.
        assert_eq!(ui.project_dir(), root.join("scripts"));

        let (x, y) = row_center(2);
        let out = press(&mut ui, &mut c, &doc, x, y, true);
        assert!(
            out.iter()
                .any(|i| matches!(i, Intent::OpenScript(p) if p.ends_with("scripts/coin.py"))),
            "{out:?}"
        );
        let (x, y) = row_center(3);
        let out = press(&mut ui, &mut c, &doc, x, y, true);
        assert!(out
            .iter()
            .any(|i| matches!(i, Intent::Open(p) if p.ends_with("level.ron"))));

        // Clicking the folder again collapses it.
        let (x, y) = row_center(0);
        press(&mut ui, &mut c, &doc, x, y, false);
        assert_eq!(ui.project.rows(&root).len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn revealing_a_new_file_expands_its_folders() {
        let root = project("reveal");
        let mut ui = Ui::new();
        ui.project_root.clone_from(&root);
        ui.panels.show_files = false;
        let created = root.join("scripts/ai/brain.py");
        std::fs::write(&created, "").unwrap();
        ui.files_changed(Some(&created));
        assert!(ui.panels.show_files);
        let rows = ui.project.rows(&root);
        assert!(rows.iter().any(|r| r.item.path == created && r.depth == 2));
        assert_eq!(ui.project.selected.as_deref(), Some(created.as_path()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dragging_the_divider_resizes_files_against_the_hierarchy() {
        let mut c = Canvas::new(1600, 950);
        let doc = Document::starter();
        let mut ui = Ui::new();
        let files = super::super::Layout::compute(1600.0, 950.0).files;
        let (x, y) = (files.x + 100.0, files.y);
        let at = |y: f32, down: bool, pressed: bool| Pointer {
            x,
            y,
            down,
            pressed,
            ..Pointer::default()
        };
        ui.frame(&mut c, &doc, at(y, true, true), None);
        ui.frame(&mut c, &doc, at(y - 100.0, true, false), None);
        ui.frame(&mut c, &doc, at(y - 100.0, false, false), None);
        assert!(
            (ui.panels.files_h - (files.h + 100.0)).abs() < 1.0,
            "{}",
            ui.panels.files_h
        );
    }

    #[test]
    fn the_focused_panel_renames_duplicates_and_deletes_by_keyboard() {
        let root = project("keys");
        let mut c = Canvas::new(1600, 950);
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root.clone_from(&root);
        ui.frame(&mut c, &doc, Pointer::default(), None);
        // A click in the panel gives it the keyboard; arrows walk rows.
        let (x, y) = row_center(1);
        press(&mut ui, &mut c, &doc, x, y, false);
        assert!(ui.files_focused());
        assert_eq!(ui.project.selected, Some(root.join("level.ron")));
        ui.files_key(FilesKey::Up, true);
        assert_eq!(ui.project.selected, Some(root.join("scripts")));
        ui.files_key(FilesKey::Right, true);
        ui.files_key(FilesKey::Down, true);
        ui.files_key(FilesKey::Down, true);
        assert_eq!(ui.project.selected, Some(root.join("scripts/coin.py")));
        assert!(matches!(
            ui.files_key(FilesKey::Open, true),
            Some(Intent::OpenScript(p)) if p.ends_with("coin.py")
        ));

        // F2 renames in place: the stem is selected, the extension kept.
        ui.files_key(FilesKey::Rename, true);
        assert_eq!(ui.edit.as_ref().map(|e| e.buffer.as_str()), Some("coin"));
        assert!(!ui.files_focused(), "the rename field has the keyboard");
        ui.type_text("gem");
        let intent = ui.edit_key(crate::ui::EditKey::Enter);
        assert_eq!(
            intent,
            Some(Intent::MoveFile {
                from: root.join("scripts/coin.py"),
                to: root.join("scripts/gem.py"),
            })
        );

        assert_eq!(
            ui.files_key(FilesKey::Duplicate, true),
            Some(Intent::DuplicateFile(root.join("scripts/coin.py")))
        );
        // Delete asks first; Enter confirms.
        assert_eq!(ui.files_key(FilesKey::Delete, true), None);
        assert!(ui.dialog_open());
        ui.dialog_enter();
        let out = ui.frame(&mut c, &doc, Pointer::default(), None);
        assert!(
            out.contains(&Intent::DeleteFile(root.join("scripts/coin.py"))),
            "{out:?}"
        );

        // A click elsewhere gives the keyboard back.
        press(&mut ui, &mut c, &doc, 800.0, 400.0, false);
        assert!(!ui.files_focused());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_context_menu_offers_file_operations() {
        let root = project("menu");
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root.clone_from(&root);
        let item = Item {
            name: "level.ron".into(),
            path: root.join("level.ron"),
            dir: false,
        };
        let labels: Vec<String> = ui
            .file_ops_items(&item, true)
            .into_iter()
            .map(|m| m.label)
            .collect();
        for want in ["Rename…", "Duplicate", "Copy Path", "Show in File Manager"] {
            assert!(labels.iter().any(|l| l == want), "{labels:?}");
        }
        assert!(labels.iter().any(|l| l == crate::files::delete_label()));
        let _ = doc;
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dragging_a_file_onto_a_folder_moves_it() {
        let root = project("drag");
        let mut c = Canvas::new(1600, 950);
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root.clone_from(&root);
        ui.frame(&mut c, &doc, Pointer::default(), None);
        // Drag level.ron (row 1) onto scripts/ (row 0).
        let (x, y) = row_center(1);
        let down = |x: f32, y: f32, pressed: bool| Pointer {
            x,
            y,
            down: true,
            pressed,
            ..Pointer::default()
        };
        ui.frame(&mut c, &doc, down(x, y, true), None);
        let (tx, ty) = row_center(0);
        ui.frame(&mut c, &doc, down(tx, ty - 2.0, false), None);
        ui.frame(&mut c, &doc, down(tx, ty, false), None);
        assert_eq!(ui.project_drop_dir(), Some(root.join("scripts")));
        let out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: tx,
                y: ty,
                ..Pointer::default()
            },
            None,
        );
        assert!(
            out.contains(&Intent::MoveFile {
                from: root.join("level.ron"),
                to: root.join("scripts/level.ron"),
            }),
            "{out:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
