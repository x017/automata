//! The Project panel: a file browser over the project directory.
//!
//! Double-click opens: scenes load, scripts open in the code editor,
//! prefabs are instantiated, folders are entered. Right-click offers the
//! same plus creation of scripts and folders.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::menus::{Action, DialogKind, MenuItem, UiAction};
use super::{CursorHint, Intent, PlayState, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::Document;
use crate::theme::{color, layout};

/// One file or folder in the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    name: String,
    path: PathBuf,
    dir: bool,
}

/// Project panel state.
#[derive(Debug, Default)]
pub struct ProjectState {
    /// The folder shown (inside the project root); `None` = the root.
    dir: Option<PathBuf>,
    items: Vec<Item>,
    listed: Option<(PathBuf, Instant)>,
    scroll: f32,
    selected: Option<PathBuf>,
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
    /// Anything else.
    Other,
}

/// Classifies a path by extension.
#[must_use]
pub fn asset_kind(path: &Path, dir: bool) -> AssetKind {
    if dir {
        return AssetKind::Folder;
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

impl Ui {
    /// The folder the Project panel shows.
    pub(super) fn project_dir(&self) -> PathBuf {
        self.project
            .dir
            .clone()
            .unwrap_or_else(|| self.project_root.clone())
    }

    /// Draws the Project tab body.
    #[allow(clippy::too_many_lines)]
    pub(super) fn project_body(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        let dir = self.project_dir();
        let stale = self
            .project
            .listed
            .as_ref()
            .is_none_or(|(d, t)| *d != dir || t.elapsed() > Duration::from_secs(1));
        if stale {
            self.project.items = list(&dir);
            self.project.listed = Some((dir.clone(), Instant::now()));
        }
        let editing = self.play == PlayState::Editing;

        // Breadcrumb bar.
        let bar_h = 26.0;
        canvas.fill_rect(rect.x, rect.y, rect.w, bar_h, color::background());
        let mut x = rect.x + 8.0;
        let root = self.project_root.clone();
        let rel = dir
            .strip_prefix(&root)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut crumbs = vec![(
            root.file_name().map_or_else(
                || "Project".to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            root.clone(),
        )];
        let mut acc = root.clone();
        for part in rel.components() {
            acc = acc.join(part);
            crumbs.push((part.as_os_str().to_string_lossy().into_owned(), acc.clone()));
        }
        let mut go = None;
        for (i, (name, path)) in crumbs.iter().enumerate() {
            if i > 0 {
                canvas.text(x, rect.y + 6.0, layout::SMALL_SIZE, "›", color::text_dim());
                x += 12.0;
            }
            let tw = canvas.measure_text(layout::SMALL_SIZE, name) + 8.0;
            let r = Rect::new(x - 4.0, rect.y + 3.0, tw, bar_h - 6.0);
            if self.hot(r) {
                canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::hover());
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    go = Some(path.clone());
                }
            }
            canvas.text(
                x,
                rect.y + 6.0,
                layout::SMALL_SIZE,
                name,
                color::accent_soft(),
            );
            x += tw;
        }
        // Toolbar buttons (right).
        let mut bx = rect.x + rect.w - 8.0;
        for (label, action) in [("New Folder", 0), ("New Script", 1)] {
            let bw = canvas.measure_text(layout::SMALL_SIZE, label) + 16.0;
            bx -= bw + 4.0;
            let r = Rect::new(bx, rect.y + 3.0, bw, bar_h - 6.0);
            if self.button(canvas, r, label, super::ButtonStyle::Normal) {
                match action {
                    0 => self.out.push(Intent::NewFolder(
                        dir.join(unique_child(&dir, "New Folder")),
                    )),
                    _ => self.open_dialog(DialogKind::NewScript, doc),
                }
            }
        }

        // Listing (grid of tiles).
        let body = Rect::new(rect.x, rect.y + bar_h, rect.w, (rect.h - bar_h).max(0.0));
        let tile_w = 112.0;
        let tile_h = 74.0;
        let cols = ((body.w - 8.0) / tile_w).floor().max(1.0) as usize;
        let rows = self.project.items.len().div_ceil(cols);
        let content_h = rows as f32 * tile_h + 8.0;
        if self.hot(body) && self.p.scroll != 0.0 {
            self.project.scroll -= self.p.scroll * 40.0;
        }
        self.project.scroll = self
            .project
            .scroll
            .clamp(0.0, (content_h - body.h).max(0.0));
        let prev = canvas.set_clip(Some((body.x, body.y, body.w, body.h)));
        let saved_clip = self.clip;
        self.clip = (body.y, body.y + body.h);
        let mut clicked_item = false;
        let items = self.project.items.clone();
        for (i, item) in items.iter().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let t = Rect::new(
                body.x + 6.0 + c as f32 * tile_w,
                body.y + 6.0 - self.project.scroll + r as f32 * tile_h,
                tile_w - 8.0,
                tile_h - 6.0,
            );
            if t.y + t.h < body.y || t.y > body.y + body.h {
                continue;
            }
            let selected = self.project.selected.as_ref() == Some(&item.path);
            let hot = self.hot(t);
            if selected {
                canvas.fill_rounded(t.x, t.y, t.w, t.h, 5.0, color::select());
            } else if hot {
                canvas.fill_rounded(t.x, t.y, t.w, t.h, 5.0, color::hover());
            }
            let kind = asset_kind(&item.path, item.dir);
            draw_asset_icon(canvas, kind, t.x + t.w / 2.0, t.y + 22.0);
            let tw = canvas
                .measure_text(layout::SMALL_SIZE, &item.name)
                .min(t.w - 8.0);
            canvas.text_clipped(
                t.x + (t.w - tw) / 2.0,
                t.y + t.h - 20.0,
                layout::SMALL_SIZE,
                &item.name,
                color::text(),
                t.w - 8.0,
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    clicked_item = true;
                    self.project.selected = Some(item.path.clone());
                    if self.p.double_click {
                        if item.dir {
                            go = Some(item.path.clone());
                        } else if let Some(intent) = open_intent(&item.path, false) {
                            self.out.push(intent);
                        }
                    }
                }
                if self.p.right_pressed {
                    clicked_item = true;
                    self.project.selected = Some(item.path.clone());
                    let mut menu = Vec::new();
                    match kind {
                        AssetKind::Scene => {
                            menu.push(item_menu("Open Scene", open_intent(&item.path, false)));
                        }
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
                        AssetKind::Folder | AssetKind::Other => {}
                    }
                    if !menu.is_empty() {
                        menu.push(MenuItem {
                            label: "-".into(),
                            shortcut: "",
                            action: None,
                            enabled: false,
                        });
                    }
                    menu.extend(self.project_create_items());
                    self.open_menu(self.p.x, self.p.y, menu);
                }
            }
        }
        if items.is_empty() {
            canvas.text(
                body.x + 12.0,
                body.y + 12.0,
                layout::SMALL_SIZE,
                "Empty folder",
                color::text_dim(),
            );
        }
        if !clicked_item && self.hot(body) {
            if self.p.pressed {
                self.project.selected = None;
            }
            if self.p.right_pressed {
                let menu = self.project_create_items();
                self.open_menu(self.p.x, self.p.y, menu);
            }
        }
        self.clip = saved_clip;
        canvas.restore_clip(prev);
        if let Some(path) = go {
            self.project.dir = (path != self.project_root).then_some(path);
            self.project.scroll = 0.0;
            self.project.listed = None;
        }
    }

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

/// Draws a 28px icon for an asset kind centered at `(cx, cy)`.
pub fn draw_asset_icon(canvas: &mut Canvas, kind: AssetKind, cx: f32, cy: f32) {
    match kind {
        AssetKind::Folder => {
            let c = color::warn();
            canvas.fill_rounded(cx - 16.0, cy - 10.0, 14.0, 6.0, 2.0, c);
            canvas.fill_rounded(cx - 16.0, cy - 7.0, 32.0, 21.0, 3.0, c);
        }
        AssetKind::Scene => {
            let c = color::accent();
            canvas.fill_polygon(
                &[
                    (cx, cy - 14.0),
                    (cx + 14.0, cy),
                    (cx, cy + 14.0),
                    (cx - 14.0, cy),
                ],
                c,
            );
            canvas.fill_polygon(
                &[
                    (cx, cy - 7.0),
                    (cx + 7.0, cy),
                    (cx, cy + 7.0),
                    (cx - 7.0, cy),
                ],
                color::surface(),
            );
        }
        AssetKind::Prefab => {
            let c = color::accent_soft();
            canvas.fill_rounded(cx - 12.0, cy - 12.0, 24.0, 24.0, 4.0, c);
            canvas.fill_rounded(cx - 6.0, cy - 6.0, 12.0, 12.0, 2.0, color::surface());
        }
        AssetKind::Script => {
            canvas.fill_rounded(cx - 12.0, cy - 15.0, 24.0, 30.0, 3.0, color::raised());
            canvas.stroke_rounded(cx - 12.0, cy - 15.0, 24.0, 30.0, 3.0, color::axis_y(), 1.5);
            canvas.text(cx - 8.0, cy - 7.0, 11.0, "py", color::axis_y());
        }
        AssetKind::Other => {
            canvas.fill_rounded(cx - 11.0, cy - 14.0, 22.0, 28.0, 3.0, color::raised());
            canvas.stroke_rounded(cx - 11.0, cy - 14.0, 22.0, 28.0, 3.0, color::border(), 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
