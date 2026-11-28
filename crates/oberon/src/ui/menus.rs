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
pub const MENUBAR_H: f32 = 40.0;

/// The menu bar's top-level menus.
pub const BAR: [&str; 7] = [
    "File",
    "Edit",
    "GameObject",
    "Component",
    "Window",
    "Build",
    "Help",
];

/// A dockable panel that the Window menu can show or hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Entity list (left sidebar, top).
    Hierarchy,
    /// Project file tree (left sidebar, bottom).
    Files,
    /// Property editor (right).
    Inspector,
    /// Console dock (bottom).
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
    /// Create a new UI document (`ui/<name>.ui.ron`).
    NewUiScreen,
    /// Create a new material file (`materials/<name>.mat.ron`).
    NewMaterial,
    /// Pick the material file for an entity's Mesh Renderer.
    ChooseMaterial(u64),
    /// Pick (or name a new) animation graph for an entity's Animation.
    ChooseAnimGraph(u64),
    /// Pick the image for a texture slot of the material being edited
    /// (index into [`automata_assets::MapSlot::ALL`]).
    ChooseMaterialMap(usize),
    /// Attach a script (existing or new) to an entity.
    AttachScript(u64),
    /// Import a model file (from anywhere) into the scene, or onto an
    /// entity.
    ImportModel(Option<u64>),
    /// Import a sound file (from anywhere) into the scene, or onto an
    /// entity's Audio Source.
    ImportSound(Option<u64>),
    /// Import a font file (from anywhere) into the project's `fonts/`.
    ImportFont,
    /// Pick a greyscale image for an entity's Terrain heights.
    ImportHeightmap(u64),
    /// Settings ▸ Build: pick the start scene.
    BuildScene,
    /// Settings ▸ Build: pick the output folder.
    BuildOutput,
    /// Settings ▸ Build: pick the Android release key.
    BuildKey,
    /// Settings ▸ Python: pick the interpreter.
    PythonInterpreter,
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
    /// Show (or hide) the Assets tab of the bottom dock.
    ShowAssets,
    /// Show (or hide) the Profiler tab of the bottom dock.
    ShowProfiler,
    /// Turn gizmo snapping on or off.
    ToggleSnap,
    /// Switch the scene view between 3D and the flat 2D view.
    Toggle2D,
    /// Open Duplicate Along for an entity.
    DuplicateAlong(u64),
    /// Show the Files panel in the left sidebar.
    ShowFiles,
    /// Open the Settings dialog.
    Settings,
    /// Open the command palette.
    CommandPalette,
    /// Open Go to File (scripts by name).
    QuickOpen,
    /// Open Settings on the Build page.
    BuildSettings,
    /// A script tab's context menu command.
    Tab(TabCommand),
    /// Pick a model from the project: for a new entity (`None`) or to show
    /// on an entity.
    PickModel(Option<u64>),
    /// Pick a sound from the project: for a new entity (`None`) or for an
    /// entity's Audio Source.
    PickSound(Option<u64>),
    /// A Files panel command on the selected file.
    File(super::project_panel::FileCommand),
    /// A UI editor command.
    UiDoc(super::ui_editor::UiDocAction),
    /// An animation graph editor command.
    Anim(super::anim_editor::AnimAction),
}

/// Commands from a script tab's context menu (VS Code's set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCommand {
    /// Close this tab.
    Close(usize),
    /// Close every other tab.
    CloseOthers(usize),
    /// Close the tabs to the right of this one.
    CloseRight(usize),
    /// Close the tabs without unsaved changes.
    CloseSaved,
    /// Close every tab.
    CloseAll,
    /// Copy the file's absolute path.
    CopyPath(usize),
    /// Copy the path relative to the project.
    CopyRelativePath(usize),
    /// Show the file in the Files panel.
    Reveal(usize),
    /// Reopen the most recently closed tab.
    ReopenClosed,
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
    pub(super) fn new(
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

    pub(super) fn ui(
        label: impl Into<String>,
        shortcut: &'static str,
        a: UiAction,
        enabled: bool,
    ) -> Self {
        Self::new(label, shortcut, Action::Ui(a), enabled)
    }

    pub(super) fn separator() -> Self {
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
    /// How many columns the menu needs to fit a window `h` tall.
    fn columns(&self, h: f32) -> usize {
        let room = (h - 16.0).max(layout::ROW_H * 4.0);
        let mut columns = 1;
        let mut y = 0.0;
        for i in &self.items {
            let ih = if i.is_separator() {
                SEP_H
            } else {
                layout::ROW_H
            };
            if y + ih > room {
                columns += 1;
                y = 0.0;
            }
            y += ih;
        }
        columns
    }

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
        // Too tall for the window: the items flow into more columns.
        let columns = self.columns(h);
        #[allow(clippy::cast_precision_loss)] // a few columns
        let mw = ((widest + 64.0).max(layout::MENU_W) * columns as f32).min(w - 8.0);
        let mh = if columns > 1 {
            h - 8.0
        } else {
            self.height().min(h - 8.0)
        };
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
        // A long menu pushed up to fit the window can land under the
        // pointer: the press that opened it mustn't also pick an item.
        self.menu_fresh = true;
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
                        "New UI Screen…",
                        "",
                        UiAction::Dialog(DialogKind::NewUiScreen),
                        true,
                    ),
                    MenuItem::ui(
                        "New Material…",
                        "",
                        UiAction::Dialog(DialogKind::NewMaterial),
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
                MenuItem::ui(
                    "Duplicate Along…",
                    "Ctrl+Shift+D",
                    UiAction::DuplicateAlong(sel),
                    has_sel,
                ),
                MenuItem::intent("Delete", "Del", Intent::Delete(sel), has_sel),
                MenuItem::ui("Rename", "F2", UiAction::Rename(sel), has_sel),
                MenuItem::separator(),
                MenuItem::intent(
                    "Select All",
                    "Ctrl+A",
                    Intent::SelectMany(doc.entities.iter().map(|e| e.id).collect()),
                    !doc.entities.is_empty(),
                ),
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
                MenuItem::ui(
                    format!(
                        "{}Snap",
                        if self.prefs.snap.enabled {
                            "✓ "
                        } else {
                            "   "
                        }
                    ),
                    "",
                    UiAction::ToggleSnap,
                    true,
                ),
                MenuItem::ui(
                    format!("{}2D View", if self.camera.two_d { "✓ " } else { "   " }),
                    "2",
                    UiAction::Toggle2D,
                    true,
                ),
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
                MenuItem::ui("Go to File…", "Ctrl+E", UiAction::QuickOpen, true),
                MenuItem::ui("Settings…", "Ctrl+,", UiAction::Settings, true),
            ],
            2 => {
                let mut items: Vec<MenuItem> = CreateKind::ALL
                    .iter()
                    .map(|k| MenuItem::intent(k.label(), "", Intent::Create(*k), editing))
                    .collect();
                items.push(MenuItem::separator());
                items.push(MenuItem::ui(
                    "Model…",
                    "",
                    UiAction::PickModel(None),
                    editing,
                ));
                items.push(MenuItem::ui(
                    "Import Model…",
                    "",
                    UiAction::Dialog(DialogKind::ImportModel(None)),
                    editing,
                ));
                items.push(MenuItem::ui(
                    "Sound…",
                    "",
                    UiAction::PickSound(None),
                    editing,
                ));
                items.push(MenuItem::ui(
                    "Import Sound…",
                    "",
                    UiAction::Dialog(DialogKind::ImportSound(None)),
                    editing,
                ));
                items.push(MenuItem::ui(
                    "Import Font…",
                    "",
                    UiAction::Dialog(DialogKind::ImportFont),
                    true,
                ));
                items
            }
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
                        check(self.panels.show_files, "Files"),
                        "",
                        UiAction::TogglePanel(Panel::Files),
                        true,
                    ),
                    MenuItem::ui(
                        check(self.panels.show_inspector, "Inspector"),
                        "",
                        UiAction::TogglePanel(Panel::Inspector),
                        true,
                    ),
                    MenuItem::ui(
                        check(self.panels.show_bottom, "Console"),
                        "",
                        UiAction::TogglePanel(Panel::Bottom),
                        true,
                    ),
                    MenuItem::ui(
                        check(
                            self.panels.show_bottom && self.bottom_tab == super::BottomTab::Assets,
                            "Assets",
                        ),
                        "",
                        UiAction::ShowAssets,
                        true,
                    ),
                    MenuItem::ui(
                        check(
                            self.panels.show_bottom
                                && self.bottom_tab == super::BottomTab::Profiler,
                            "Profiler",
                        ),
                        "",
                        UiAction::ShowProfiler,
                        true,
                    ),
                    MenuItem::separator(),
                    MenuItem::ui("Reset Layout", "", UiAction::ResetLayout, true),
                ]
            }
            5 => {
                use crate::build::Target;
                let idle = self.build_status.is_none();
                vec![
                    MenuItem::intent("Build", "Ctrl+Shift+B", Intent::Build(None), idle),
                    MenuItem::intent(
                        "Build for Linux",
                        "",
                        Intent::Build(Some(Target::Linux)),
                        idle,
                    ),
                    MenuItem::intent(
                        "Build for Windows",
                        "",
                        Intent::Build(Some(Target::Windows)),
                        idle,
                    ),
                    MenuItem::intent(
                        "Build for macOS",
                        "",
                        Intent::Build(Some(Target::MacOS)),
                        idle,
                    ),
                    MenuItem::intent(
                        "Build for Android",
                        "",
                        Intent::Build(Some(Target::Android)),
                        idle,
                    ),
                    MenuItem::intent("Build for Web", "", Intent::Build(Some(Target::Web)), idle),
                    MenuItem::intent("Play in Browser", "", Intent::PlayInBrowser, idle),
                    MenuItem::separator(),
                    MenuItem::ui("Build Settings…", "", UiAction::BuildSettings, true),
                    MenuItem::intent("Open Build Folder", "", Intent::OpenBuildFolder, true),
                    MenuItem::separator(),
                    MenuItem::intent(
                        "Download Export Templates",
                        "",
                        Intent::DownloadTemplates(None),
                        idle,
                    ),
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
            MenuItem::ui(
                "Duplicate Along…",
                "Ctrl+Shift+D",
                UiAction::DuplicateAlong(id),
                editing,
            ),
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
            if d.prefab.is_some() {
                let changed = doc.prefab_overrides(id).is_some_and(|o| !o.is_empty());
                items.extend([
                    MenuItem::intent(
                        "Apply to Prefab",
                        "",
                        Intent::ApplyPrefab(id),
                        editing && changed,
                    ),
                    MenuItem::intent(
                        "Revert to Prefab",
                        "",
                        Intent::RevertPrefab { id, paths: None },
                        editing && changed,
                    ),
                    MenuItem::intent("Unlink Prefab", "", Intent::UnlinkPrefab(id), editing),
                    MenuItem::intent(
                        "Select Prefab Instances",
                        "",
                        Intent::SelectInstances(id),
                        true,
                    ),
                ]);
            }
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
        let mut items: Vec<MenuItem> = CreateKind::ALL
            .iter()
            .map(|k| {
                MenuItem::intent(
                    format!("Create {}", k.label()),
                    "",
                    Intent::Create(*k),
                    editing,
                )
            })
            .collect();
        items.push(MenuItem::ui(
            "Create Model…",
            "",
            UiAction::PickModel(None),
            editing,
        ));
        items.push(MenuItem::ui(
            "Create Sound…",
            "",
            UiAction::PickSound(None),
            editing,
        ));
        items
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
            UiAction::UiDoc(a) => self.ui_doc_action(a),
            UiAction::Anim(a) => self.anim_action(a),
            UiAction::TogglePanel(panel) => {
                let p = &mut self.panels;
                match panel {
                    Panel::Hierarchy => p.show_hierarchy = !p.show_hierarchy,
                    Panel::Files => p.show_files = !p.show_files,
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
            UiAction::ShowFiles => self.panels.show_files = true,
            UiAction::ShowAssets => self.toggle_assets(),
            UiAction::ShowProfiler => {
                if self.panels.show_bottom && self.bottom_tab == super::BottomTab::Profiler {
                    self.panels.show_bottom = false;
                } else {
                    self.panels.show_bottom = true;
                    self.bottom_tab = super::BottomTab::Profiler;
                }
            }
            UiAction::DuplicateAlong(id) => self.open_duplicate_along(doc, id),
            UiAction::ToggleSnap => {
                self.prefs.snap.enabled = !self.prefs.snap.enabled;
                self.prefs_dirty = true;
            }
            UiAction::Toggle2D => {
                let on = !self.camera.two_d;
                self.camera.set_two_d(on);
            }
            UiAction::BuildSettings => {
                self.load_build_settings(doc.path.as_deref());
                self.open_settings_page(super::settings::SettingsPage::Build);
            }
            UiAction::Settings => self.open_settings(),
            UiAction::CommandPalette => self.open_palette(doc),
            UiAction::QuickOpen => self.open_quick_open(),
            UiAction::PickModel(target) => self.open_model_picker(target),
            UiAction::PickSound(target) => self.open_sound_picker(target),
            UiAction::Tab(cmd) => self.tab_command(cmd),
            UiAction::File(cmd) => self.file_command(cmd),
        }
    }

    /// Copies an entity to the entity clipboard.
    pub(super) fn copy_entity(&mut self, doc: &Document, id: u64) {
        // Inside a multi-selection, the whole selection (in scene order).
        if doc.selection_len() > 1 && doc.is_selected(id) {
            let all: Vec<EntityData> = doc
                .entities
                .iter()
                .filter(|e| doc.is_selected(e.id))
                .map(|e| e.data.clone())
                .collect();
            let n = all.len();
            self.entity_clipboard = all;
            self.log(super::LogLevel::Info, format!("Copied {n} entities"));
            return;
        }
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
        let mut x = 8.0;
        let open_bar = self.menu.as_ref().and_then(|m| m.bar);
        for (i, title) in BAR.iter().enumerate() {
            let tw = canvas.measure_text(layout::FONT_SIZE, title) + 18.0;
            let r = Rect::new(x, 7.0, tw, MENUBAR_H - 14.0);
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
        // The scene's file name and unsaved marker, after the menus.
        let name = doc.path.as_ref().map_or_else(
            || "Untitled scene".to_string(),
            |p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            },
        );
        canvas.fill_rect(x + 10.0, cy - 7.0, 1.0, 14.0, color::border());
        let nx = x + 22.0;
        let max_w = (w * 0.5 - 90.0 - nx).max(0.0);
        let nw = canvas.text_clipped(
            nx,
            cy - 8.0,
            layout::FONT_SIZE,
            &name,
            color::text_dim(),
            max_w,
        );
        if doc.dirty() && max_w > 0.0 {
            canvas.fill_circle(nx + nw + 8.0, cy, 3.0, color::text_dim());
        }
    }

    /// Draws the open popup menu (if any) and handles its clicks.
    pub(super) fn draw_menu(&mut self, canvas: &mut Canvas, doc: &Document, w: f32, h: f32) {
        let fresh = std::mem::take(&mut self.menu_fresh);
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
        #[allow(clippy::cast_precision_loss)] // a few columns
        let cw = r.w / menu.columns(h) as f32;
        let room = (h - 16.0).max(layout::ROW_H * 4.0);
        let mut cx = r.x;
        for item in &menu.items {
            let ih = if item.is_separator() {
                SEP_H
            } else {
                layout::ROW_H
            };
            if y + ih > r.y + 4.0 + room {
                cx += cw;
                y = r.y + 4.0;
            }
            if item.is_separator() {
                canvas.fill_rect(cx + 8.0, y + SEP_H / 2.0, cw - 16.0, 1.0, color::border());
                y += SEP_H;
                continue;
            }
            let row = Rect::new(cx + 4.0, y, cw - 8.0, layout::ROW_H);
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
            if hot && raw.pressed && !fresh {
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
