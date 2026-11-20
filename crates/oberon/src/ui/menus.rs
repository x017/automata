//! Menus: the menu bar, popup/context menus, and dropdowns.
//!
//! A [`Menu`] is a snapshot of items built when it opens; choosing an item
//! either emits an [`Intent`] for the app or runs a UI-local [`UiAction`].

use automata_core::EntityData;

use super::{ButtonStyle, CursorHint, Intent, PlayState, Rect, Ui};
use crate::canvas::Canvas;
use crate::doc::{ComponentKind, CreateKind, Document};
use crate::theme::{color, layout};
use crate::view::GizmoMode;

/// Height of the menu bar.
pub const MENUBAR_H: f32 = 26.0;

/// The menu bar's top-level menus.
pub const BAR: [&str; 6] = ["File", "Edit", "GameObject", "Component", "Window", "Help"];

/// A dockable panel that the Window menu can show or hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Entity list (left).
    Hierarchy,
    /// Property editor (right).
    Inspector,
    /// Console / Project dock (bottom).
    Bottom,
}

/// Modal dialogs the UI can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// Open a scene file.
    OpenScene,
    /// Save the scene under a new name.
    SaveSceneAs,
    /// Save an entity as a prefab file.
    SavePrefab(u64),
    /// Instantiate a prefab file.
    InstantiatePrefab,
    /// Open a Python script in the code editor.
    OpenScript,
    /// Create a new Python script file.
    NewScript,
    /// Attach a script (existing or new) to an entity.
    AttachScript(u64),
    /// Keyboard shortcut reference.
    Shortcuts,
    /// About Oberon.
    About,
}

/// A UI-local action a menu item can trigger.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiAction {
    /// Start renaming an entity.
    Rename(u64),
    /// Frame an entity in the scene view.
    Focus(u64),
    /// Copy a component's values to the component clipboard.
    CopyComponent(u64, ComponentKind),
    /// Copy an entity to the entity clipboard.
    CopyEntity(u64),
    /// Select a transform tool.
    Gizmo(GizmoMode),
    /// Show or hide a panel.
    TogglePanel(Panel),
    /// Restore default panel sizes and visibility.
    ResetLayout,
    /// Open a dialog.
    Dialog(DialogKind),
    /// Open a color picker for an entity's color channels.
    PickColor(u64, super::picker::ColorTarget),
    /// Show the Console tab in the bottom dock.
    ShowConsole,
    /// Show the Project tab in the bottom dock.
    ShowProject,
    /// Open the Settings dialog.
    Settings,
    /// Open the command palette.
    CommandPalette,
}

/// What a menu item does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Ask the app to do something.
    Intent(Intent),
    /// Handle inside the UI.
    Ui(UiAction),
}

/// One menu row (or a separator).
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// Row text; `"-"` draws a separator.
    pub label: String,
    /// Shortcut hint, right-aligned.
    pub shortcut: &'static str,
    /// What choosing it does (`None` = header / separator).
    pub action: Option<Action>,
    /// Greyed out when false.
    pub enabled: bool,
}

impl MenuItem {
    fn new(
        label: impl Into<String>,
        shortcut: &'static str,
        action: Action,
        enabled: bool,
    ) -> Self {
        Self {
            label: label.into(),
            shortcut,
            action: Some(action),
            enabled,
        }
    }

    fn intent(label: impl Into<String>, shortcut: &'static str, i: Intent, enabled: bool) -> Self {
        Self::new(label, shortcut, Action::Intent(i), enabled)
    }

    fn ui(label: impl Into<String>, shortcut: &'static str, a: UiAction, enabled: bool) -> Self {
        Self::new(label, shortcut, Action::Ui(a), enabled)
    }

    fn separator() -> Self {
        Self {
            label: "-".into(),
            shortcut: "",
            action: None,
            enabled: false,
        }
    }

    fn is_separator(&self) -> bool {
        self.label == "-" && self.action.is_none()
    }
}

/// An open popup menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// Anchor (top-left) in logical pixels.
    pub x: f32,
    /// Anchor top.
    pub y: f32,
    /// The rows.
    pub items: Vec<MenuItem>,
    /// Which menu-bar entry opened it, if any.
    pub bar: Option<usize>,
}

const SEP_H: f32 = 9.0;

impl Menu {
    fn height(&self) -> f32 {
        self.items
            .iter()
            .map(|i| {
                if i.is_separator() {
                    SEP_H
                } else {
                    layout::ROW_H
                }
            })
            .sum::<f32>()
            + 8.0
    }

    /// The menu's on-screen rectangle (kept inside a `w`×`h` window).
    #[must_use]
    pub fn rect(&self, canvas: &mut Canvas, w: f32, h: f32) -> Rect {
        let widest = self
            .items
            .iter()
            .map(|i| {
                canvas.measure_text(layout::FONT_SIZE, &i.label)
                    + canvas.measure_text(layout::SMALL_SIZE, i.shortcut)
            })
            .fold(0.0f32, f32::max);
        let mw = (widest + 64.0).max(layout::MENU_W).min(w - 8.0);
        let mh = self.height().min(h - 8.0);
        let x = self.x.min(w - mw - 4.0).max(0.0);
        let y = if self.y + mh > h {
            (h - mh - 4.0).max(0.0)
        } else {
            self.y
        };
        Rect::new(x, y, mw, mh)
    }
}

impl Ui {
    /// Opens a popup menu at `(x, y)`.
    pub(super) fn open_menu(&mut self, x: f32, y: f32, items: Vec<MenuItem>) {
        self.menu = Some(Menu {
            x,
            y,
            items,
            bar: None,
        });
    }

    /// Items for the menu bar entry `index`.
    pub(super) fn bar_items(&self, index: usize, doc: &Document) -> Vec<MenuItem> {
        let editing = self.play == PlayState::Editing;
        let selected = doc.selected_index().map(|i| doc.entities[i].id);
        let has_sel = selected.is_some() && editing;
        let sel = selected.unwrap_or(0);
        match index {
            0 => {
                let mut items = vec![
                    MenuItem::intent("New Scene", "Ctrl+N", Intent::New, editing),
                    MenuItem::ui(
                        "Open Scene…",
                        "Ctrl+O",
                        UiAction::Dialog(DialogKind::OpenScene),
                        editing,
                    ),
                    MenuItem::intent("Save", "Ctrl+S", Intent::Save, true),
                    MenuItem::ui(
                        "Save As…",
                        "Ctrl+Shift+S",
                        UiAction::Dialog(DialogKind::SaveSceneAs),
                        true,
                    ),
                    MenuItem::separator(),
                    MenuItem::ui(
                        "New Script…",
                        "",
                        UiAction::Dialog(DialogKind::NewScript),
                        true,
                    ),
                    MenuItem::ui(
                        "Open Script…",
                        "",
                        UiAction::Dialog(DialogKind::OpenScript),
                        true,
                    ),
                    MenuItem::separator(),
                    MenuItem::ui(
                        "Save Selection as Prefab…",
                        "",
                        UiAction::Dialog(DialogKind::SavePrefab(sel)),
                        has_sel,
                    ),
                    MenuItem::ui(
                        "Instantiate Prefab…",
                        "",
                        UiAction::Dialog(DialogKind::InstantiatePrefab),
                        editing,
                    ),
                ];
                if !self.recent.is_empty() {
                    items.push(MenuItem::separator());
                    for path in self.recent.iter().take(8) {
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |n| n.to_string_lossy().into(),
                        );
                        items.push(MenuItem::intent(
                            format!("Recent: {name}"),
                            "",
                            Intent::Open(path.clone()),
                            editing,
                        ));
                    }
                }
                items.push(MenuItem::separator());
                items.push(MenuItem::intent("Quit", "Ctrl+Q", Intent::Quit, true));
                items
            }
            1 => vec![
                MenuItem::intent("Undo", "Ctrl+Z", Intent::Undo, editing && doc.can_undo()),
                MenuItem::intent("Redo", "Ctrl+Y", Intent::Redo, editing && doc.can_redo()),
                MenuItem::separator(),
                MenuItem::ui("Copy", "Ctrl+C", UiAction::CopyEntity(sel), has_sel),
                MenuItem::intent(
                    "Paste",
                    "Ctrl+V",
                    Intent::PasteEntities(self.entity_clipboard.clone()),
                    editing && !self.entity_clipboard.is_empty(),
                ),
                MenuItem::intent("Duplicate", "Ctrl+D", Intent::Duplicate(sel), has_sel),
                MenuItem::intent("Delete", "Del", Intent::Delete(sel), has_sel),
                MenuItem::ui("Rename", "F2", UiAction::Rename(sel), has_sel),
                MenuItem::separator(),
                MenuItem::ui(
                    "Frame Selected",
                    "F",
                    UiAction::Focus(sel),
                    selected.is_some(),
                ),
                MenuItem::intent("Deselect", "Esc", Intent::Select(None), selected.is_some()),
                MenuItem::separator(),
                MenuItem::ui("Move Tool", "W", UiAction::Gizmo(GizmoMode::Move), true),
                MenuItem::ui("Rotate Tool", "E", UiAction::Gizmo(GizmoMode::Rotate), true),
                MenuItem::ui("Scale Tool", "R", UiAction::Gizmo(GizmoMode::Scale), true),
                MenuItem::separator(),
                MenuItem::intent(
                    if editing { "Play" } else { "Stop" },
                    "Ctrl+P",
                    if editing { Intent::Play } else { Intent::Stop },
                    true,
                ),
                MenuItem::intent("Pause", "", Intent::Pause, !editing),
                MenuItem::separator(),
                MenuItem::ui(
                    "Command Palette…",
                    "Ctrl+Shift+P",
                    UiAction::CommandPalette,
                    true,
                ),
                MenuItem::ui("Settings…", "Ctrl+,", UiAction::Settings, true),
            ],
            2 => CreateKind::ALL
                .iter()
                .map(|k| MenuItem::intent(k.label(), "", Intent::Create(*k), editing))
                .collect(),
            3 => match doc.selected_index() {
                Some(i) => {
                    let mut items: Vec<MenuItem> = ComponentKind::ALL
                        .iter()
                        .map(|k| {
                            let present = k.present_in(&doc.entities[i].data);
                            let label = if present {
                                format!("{} ✓", k.label())
                            } else {
                                k.label().to_string()
                            };
                            MenuItem::intent(
                                label,
                                "",
                                Intent::AddComponent(sel, *k),
                                editing && !present,
                            )
                        })
                        .collect();
                    items.push(MenuItem::separator());
                    items.push(self.script_menu_item(sel, doc.entities[i].data.script.is_some()));
                    items
                }
                None => vec![MenuItem {
                    label: "Select an entity first".into(),
                    shortcut: "",
                    action: None,
                    enabled: false,
                }],
            },
            4 => {
                let check =
                    |on: bool, name: &str| format!("{}{name}", if on { "✓ " } else { "   " });
                vec![
                    MenuItem::ui(
                        check(self.panels.show_hierarchy, "Hierarchy"),
                        "",
                        UiAction::TogglePanel(Panel::Hierarchy),
                        true,
                    ),
                    MenuItem::ui(
                        check(self.panels.show_inspector, "Inspector"),
                        "",
                        UiAction::TogglePanel(Panel::Inspector),
                        true,
                    ),
                    MenuItem::ui(
                        check(self.panels.show_bottom, "Console / Project"),
                        "",
                        UiAction::TogglePanel(Panel::Bottom),
                        true,
                    ),
                    MenuItem::separator(),
                    MenuItem::ui("Console", "", UiAction::ShowConsole, true),
                    MenuItem::ui("Project", "", UiAction::ShowProject, true),
                    MenuItem::separator(),
                    MenuItem::ui("Reset Layout", "", UiAction::ResetLayout, true),
                ]
            }
            _ => vec![
                MenuItem::ui(
                    "Keyboard Shortcuts",
                    "F1",
                    UiAction::Dialog(DialogKind::Shortcuts),
                    true,
                ),
                MenuItem::ui(
                    "About Oberon",
                    "",
                    UiAction::Dialog(DialogKind::About),
                    true,
                ),
            ],
        }
    }

    /// Context menu for an entity (Hierarchy row or scene-view object).
    pub(super) fn entity_menu(&self, doc: &Document, id: u64) -> Vec<MenuItem> {
        let editing = self.play == PlayState::Editing;
        let mut items = vec![
            MenuItem::ui("Rename", "F2", UiAction::Rename(id), editing),
            MenuItem::intent("Duplicate", "Ctrl+D", Intent::Duplicate(id), editing),
            MenuItem::ui("Copy", "Ctrl+C", UiAction::CopyEntity(id), true),
            MenuItem::intent(
                "Paste",
                "Ctrl+V",
                Intent::PasteEntities(self.entity_clipboard.clone()),
                editing && !self.entity_clipboard.is_empty(),
            ),
            MenuItem::intent("Delete", "Del", Intent::Delete(id), editing),
            MenuItem::separator(),
            MenuItem::ui("Frame in Scene", "F", UiAction::Focus(id), true),
            MenuItem::ui(
                "Save as Prefab…",
                "",
                UiAction::Dialog(DialogKind::SavePrefab(id)),
                editing,
            ),
        ];
        if let Some(i) = doc.index_of(id) {
            let d = &doc.entities[i].data;
            match &d.script {
                Some(script) => items.push(MenuItem::intent(
                    "Edit Script",
                    "",
                    Intent::OpenScript(doc.resolve(&script.path)),
                    true,
                )),
                None => items.push(MenuItem::ui(
                    "Attach Script…",
                    "",
                    UiAction::Dialog(DialogKind::AttachScript(id)),
                    editing,
                )),
            }
        }
        items.push(MenuItem::separator());
        items.extend(self.create_items());
        items
    }

    /// The "Create …" items (shared by context menus and + Create).
    pub(super) fn create_items(&self) -> Vec<MenuItem> {
        let editing = self.play == PlayState::Editing;
        CreateKind::ALL
            .iter()
            .map(|k| {
                MenuItem::intent(
                    format!("Create {}", k.label()),
                    "",
                    Intent::Create(*k),
                    editing,
                )
            })
            .collect()
    }

    /// The ⋮ menu of a component header.
    pub(super) fn component_menu(&self, id: u64, kind: Option<ComponentKind>) -> Vec<MenuItem> {
        let editing = self.play == PlayState::Editing;
        let mut items = Vec::new();
        match kind {
            None => {
                items.push(MenuItem::intent(
                    "Reset",
                    "",
                    Intent::ResetTransform(id),
                    editing,
                ));
            }
            Some(kind) => {
                items.push(MenuItem::intent(
                    "Reset",
                    "",
                    Intent::ResetComponent(id, kind),
                    editing,
                ));
                items.push(MenuItem::ui(
                    "Copy Component",
                    "",
                    UiAction::CopyComponent(id, kind),
                    true,
                ));
                let paste = self
                    .component_clipboard
                    .as_ref()
                    .filter(|(k, _)| *k == kind)
                    .map(|(_, data)| data.clone());
                items.push(MenuItem::intent(
                    "Paste Component Values",
                    "",
                    Intent::PasteComponent {
                        id,
                        kind,
                        source: Box::new(paste.clone().unwrap_or_default()),
                    },
                    editing && paste.is_some(),
                ));
                items.push(MenuItem::separator());
                items.push(MenuItem::intent(
                    "Remove Component",
                    "",
                    Intent::RemoveComponent(id, kind),
                    editing,
                ));
            }
        }
        items
    }

    /// Runs a UI-local action.
    pub(super) fn run_action(&mut self, action: UiAction, doc: &Document) {
        match action {
            UiAction::Rename(id) => {
                if self.play == PlayState::Editing {
                    self.out.push(Intent::Select(Some(id)));
                    self.begin_rename_id(doc, id);
                }
            }
            UiAction::Focus(id) => {
                if let Some(i) = doc.index_of(id) {
                    self.focus_entity(&doc.entities[i]);
                }
            }
            UiAction::CopyComponent(id, kind) => {
                if let Some(i) = doc.index_of(id) {
                    self.component_clipboard = Some((kind, doc.entities[i].data.clone()));
                    self.log(super::LogLevel::Info, format!("Copied {}", kind.label()));
                }
            }
            UiAction::CopyEntity(id) => self.copy_entity(doc, id),
            UiAction::Gizmo(mode) => self.gizmo = mode,
            UiAction::TogglePanel(panel) => {
                let p = &mut self.panels;
                match panel {
                    Panel::Hierarchy => p.show_hierarchy = !p.show_hierarchy,
                    Panel::Inspector => p.show_inspector = !p.show_inspector,
                    Panel::Bottom => p.show_bottom = !p.show_bottom,
                }
            }
            UiAction::ResetLayout => self.panels = super::PanelSizes::default(),
            UiAction::Dialog(kind) => self.open_dialog(kind, doc),
            UiAction::PickColor(id, target) => self.open_picker(doc, id, target),
            UiAction::ShowConsole => {
                self.panels.show_bottom = true;
                self.bottom_tab = super::BottomTab::Console;
            }
            UiAction::ShowProject => {
                self.panels.show_bottom = true;
                self.bottom_tab = super::BottomTab::Project;
            }
            UiAction::Settings => self.open_settings(),
            UiAction::CommandPalette => self.open_palette(doc),
        }
    }

    /// Copies an entity to the entity clipboard.
    pub(super) fn copy_entity(&mut self, doc: &Document, id: u64) {
        if let Some(i) = doc.index_of(id) {
            let data: EntityData = doc.entities[i].data.clone();
            self.entity_clipboard = vec![data];
            self.log(
                super::LogLevel::Info,
                format!("Copied {}", doc.entities[i].name()),
            );
        }
    }

    /// Draws the menu bar along the top edge.
    pub(super) fn menu_bar(&mut self, canvas: &mut Canvas, doc: &Document, w: f32) {
        canvas.fill_rect(0.0, 0.0, w, MENUBAR_H, color::background());
        canvas.fill_rect(0.0, MENUBAR_H - 1.0, w, 1.0, color::border());
        let cy = MENUBAR_H / 2.0;
        draw_logo(canvas, 17.0, cy, 15.0, color::accent());
        canvas.text(36.0, cy - 7.5, 13.0, "OBERON", color::text());
        let mut x = 98.0;
        let open_bar = self.menu.as_ref().and_then(|m| m.bar);
        for (i, title) in BAR.iter().enumerate() {
            let tw = canvas.measure_text(layout::FONT_SIZE, title) + 18.0;
            let r = Rect::new(x, 2.0, tw, MENUBAR_H - 4.0);
            let over = r.contains(self.raw.x, self.raw.y);
            let open = open_bar == Some(i);
            if open || over {
                canvas.fill_rounded(
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    4.0,
                    if open {
                        color::select()
                    } else {
                        color::hover()
                    },
                );
            }
            canvas.text(x + 9.0, cy - 7.5, layout::FONT_SIZE, title, color::text());
            if over {
                self.cursor = CursorHint::Pointer;
                // Click toggles; hovering moves between open bar menus.
                let switch = open_bar.is_some() && open_bar != Some(i);
                if (self.raw.pressed && !open) || switch {
                    let items = self.bar_items(i, doc);
                    self.menu = Some(Menu {
                        x: r.x,
                        y: MENUBAR_H,
                        items,
                        bar: Some(i),
                    });
                    self.bar_click_consumed = true;
                } else if self.raw.pressed && open {
                    self.menu = None;
                    self.bar_click_consumed = true;
                }
            }
            x += tw + 2.0;
        }
        // Right side: the scene's file name and unsaved marker.
        let name = doc.path.as_ref().map_or_else(
            || "Untitled scene".to_string(),
            |p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            },
        );
        let label = if doc.dirty() {
            format!("{name} •")
        } else {
            name
        };
        let tw = canvas.measure_text(layout::SMALL_SIZE, &label);
        if w - tw - 16.0 > x + 20.0 {
            canvas.text(
                w - tw - 12.0,
                cy - 7.0,
                layout::SMALL_SIZE,
                &label,
                color::text_dim(),
            );
        }
    }

    /// Draws the open popup menu (if any) and handles its clicks.
    pub(super) fn draw_menu(&mut self, canvas: &mut Canvas, doc: &Document, w: f32, h: f32) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let r = menu.rect(canvas, w, h);
        canvas.fill_rounded(r.x + 3.0, r.y + 4.0, r.w, r.h, 6.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 6.0, color::raised());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 6.0, color::border(), 1.0);
        let raw = self.raw;
        let mut chosen = None;
        let mut y = r.y + 4.0;
        for item in &menu.items {
            if item.is_separator() {
                canvas.fill_rect(r.x + 8.0, y + SEP_H / 2.0, r.w - 16.0, 1.0, color::border());
                y += SEP_H;
                continue;
            }
            let row = Rect::new(r.x + 4.0, y, r.w - 8.0, layout::ROW_H);
            let hot = item.enabled && row.contains(raw.x, raw.y);
            if hot {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 4.0, color::select());
                self.cursor = CursorHint::Pointer;
            }
            let fg = if item.enabled {
                color::text()
            } else {
                color::border()
            };
            canvas.text_clipped(
                row.x + 10.0,
                row.y + 5.0,
                layout::FONT_SIZE,
                &item.label,
                fg,
                row.w - 20.0,
            );
            if !item.shortcut.is_empty() {
                let sw = canvas.measure_text(layout::SMALL_SIZE, item.shortcut);
                canvas.text(
                    row.x + row.w - sw - 10.0,
                    row.y + 6.0,
                    layout::SMALL_SIZE,
                    item.shortcut,
                    color::text_dim(),
                );
            }
            if hot && raw.pressed {
                chosen.clone_from(&item.action);
            }
            y += layout::ROW_H;
        }
        if let Some(action) = chosen {
            self.menu = None;
            match action {
                Action::Intent(intent) => self.out.push(intent),
                Action::Ui(action) => self.run_action(action, doc),
            }
        }
    }

    /// A small "⋮" button; returns true when clicked.
    pub(super) fn kebab_button(&mut self, canvas: &mut Canvas, r: Rect) -> bool {
        let clicked = self.button(canvas, r, "", ButtonStyle::Normal);
        let cx = r.x + r.w / 2.0;
        for i in 0..3 {
            canvas.fill_circle(
                cx,
                r.y + r.h / 2.0 - 4.0 + i as f32 * 4.0,
                1.3,
                color::text(),
            );
        }
        clicked
    }
}

/// The AUTOMATA mark: a sharp "A" crossed by an orbital ring, with a
/// four-point star. `h` is the letter height, centred on `(cx, cy)`.
pub(super) fn draw_logo(canvas: &mut Canvas, cx: f32, cy: f32, h: f32, c: tiny_skia::Color) {
    let half = h / 2.0;
    let leg = h * 0.40;
    let w = (h * 0.15).max(1.6);
    canvas.polyline(
        &[
            (cx - leg, cy + half),
            (cx, cy - half + w * 0.4),
            (cx + leg, cy + half),
        ],
        c,
        w,
    );
    // The orbit: a tilted ellipse arc sweeping in front of the letter.
    let (rx, ry, tilt) = (h * 0.78, h * 0.2, -0.36_f32);
    let (st, ct) = tilt.sin_cos();
    let pts: Vec<(f32, f32)> = (0..=28)
        .map(|i| {
            let a = 0.35 + i as f32 / 28.0 * 4.4;
            let (x, y) = (rx * a.cos(), ry * a.sin());
            (cx + x * ct - y * st, cy + h * 0.1 + x * st + y * ct)
        })
        .collect();
    canvas.polyline(&pts, c, (h * 0.07).max(1.0));
    // The star.
    let (sx, sy, r) = (cx + h * 0.72, cy - h * 0.34, h * 0.16);
    let k = r * 0.28;
    canvas.fill_polygon(
        &[
            (sx, sy - r),
            (sx + k, sy - k),
            (sx + r, sy),
            (sx + k, sy + k),
            (sx, sy + r),
            (sx - k, sy + k),
            (sx - r, sy),
            (sx - k, sy - k),
        ],
        c,
    );
}
