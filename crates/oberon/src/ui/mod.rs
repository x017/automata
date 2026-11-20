//! The Oberon editor UI: an immediate-mode, Unity-style interface rendered
//! into a [`Canvas`] each frame.
//!
//! Layout: toolbar (file / undo / transform tools / play controls) on top,
//! **Hierarchy** left, **Scene view** center with the **Console** under it,
//! **Inspector** right, status bar at the foot.
//!
//! The UI never mutates the document. Widgets are drawn and hit-tested
//! against the pointer, and interactions return [`Intent`]s the app applies
//! (with undo). Camera navigation, tool selection, menus, panel scrolling and
//! text editing are UI-local state held in [`Ui`].

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use automata_core::{AttributeValue, EntityData, TransformData};
use glam::Vec3;
use tiny_skia::Color;

use crate::canvas::Canvas;
use crate::doc::{ComponentKind, CreateKind, Document, EntityEntry};
use crate::fields::{self, Choice, Field, Flag};
use crate::theme::{color, layout};
use crate::view::{self, Gizmo, GizmoMode, Projector};

pub mod attributes;
pub mod dialogs;
pub mod menus;
pub mod picker;
pub mod project_panel;

use dialogs::Dialog;
use menus::MENUBAR_H;
pub use menus::{DialogKind, Panel};
use picker::ColorPicker;
pub use picker::ColorTarget;

pub use crate::view::ViewCamera;

/// The pointer's state for a frame, in logical pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pointer {
    /// X position.
    pub x: f32,
    /// Y position.
    pub y: f32,
    /// Whether the primary (left) button is held.
    pub down: bool,
    /// Whether the primary button was pressed since the last frame.
    pub pressed: bool,
    /// Whether that press was a double click.
    pub double_click: bool,
    /// Whether the right button is held (orbits the scene view).
    pub right_down: bool,
    /// Whether the middle button is held (pans the scene view).
    pub middle_down: bool,
    /// Scroll wheel notches since the last frame (positive = up).
    pub scroll: f32,
    /// Whether the right button was pressed since the last frame.
    pub right_pressed: bool,
    /// Shift held (fine scrubbing).
    pub shift: bool,
    /// Ctrl held (coarse scrubbing).
    pub ctrl: bool,
    /// Alt held (Alt+drag orbits, Unity-style).
    pub alt: bool,
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle from origin and size.
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether `(px, py)` is inside.
    #[must_use]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Panel sizes and visibility (resizable via splitters, toggled from the
/// Window menu).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelSizes {
    /// Hierarchy width.
    pub hierarchy_w: f32,
    /// Inspector width.
    pub inspector_w: f32,
    /// Bottom dock height.
    pub bottom_h: f32,
    /// Hierarchy visible.
    pub show_hierarchy: bool,
    /// Inspector visible.
    pub show_inspector: bool,
    /// Bottom dock visible.
    pub show_bottom: bool,
}

impl Default for PanelSizes {
    fn default() -> Self {
        Self {
            hierarchy_w: layout::HIERARCHY_W,
            inspector_w: layout::INSPECTOR_W,
            bottom_h: layout::CONSOLE_H + 40.0,
            show_hierarchy: true,
            show_inspector: true,
            show_bottom: true,
        }
    }
}

/// Tabs of the bottom dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomTab {
    /// Log output.
    #[default]
    Console,
    /// File browser.
    Project,
}

/// The editor's panel rectangles for a window of a given logical size.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Menu bar.
    pub menubar: Rect,
    /// Tool bar (under the menu bar).
    pub toolbar: Rect,
    /// Left hierarchy panel.
    pub hierarchy: Rect,
    /// Right inspector panel.
    pub inspector: Rect,
    /// Center scene view (or code editor).
    pub viewport: Rect,
    /// Bottom dock (Console / Project) under the center area.
    pub console: Rect,
    /// Bottom status bar.
    pub status: Rect,
}

impl Layout {
    /// Computes the default layout for a `w`×`h` logical window.
    #[must_use]
    pub fn compute(w: f32, h: f32) -> Self {
        Self::with_panels(w, h, &PanelSizes::default())
    }

    /// Computes the layout for a `w`×`h` logical window and panel config.
    #[must_use]
    pub fn with_panels(w: f32, h: f32, panels: &PanelSizes) -> Self {
        let top = MENUBAR_H + layout::TOOLBAR_H;
        let body_y = top;
        let body_h = (h - top - layout::STATUS_H).max(1.0);
        let hier_w = if panels.show_hierarchy {
            panels.hierarchy_w.clamp(140.0, (w * 0.4).max(140.0))
        } else {
            0.0
        };
        let insp_w = if panels.show_inspector {
            panels.inspector_w.clamp(220.0, (w * 0.45).max(220.0))
        } else {
            0.0
        };
        let center_w = (w - hier_w - insp_w).max(1.0);
        let console_h = if panels.show_bottom {
            panels.bottom_h.clamp(80.0, (body_h * 0.7).max(80.0))
        } else {
            0.0
        };
        let view_h = (body_h - console_h).max(1.0);
        Self {
            menubar: Rect::new(0.0, 0.0, w, MENUBAR_H),
            toolbar: Rect::new(0.0, MENUBAR_H, w, layout::TOOLBAR_H),
            hierarchy: Rect::new(0.0, body_y, hier_w, body_h),
            inspector: Rect::new(w - insp_w, body_y, insp_w, body_h),
            viewport: Rect::new(hier_w, body_y, center_w, view_h),
            console: Rect::new(hier_w, body_y + view_h, center_w, console_h),
            status: Rect::new(0.0, h - layout::STATUS_H, w, layout::STATUS_H),
        }
    }
}

/// An action the UI requests the app to perform on the document.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Select an entity (`None` clears the selection).
    Select(Option<u64>),
    /// Create a new entity.
    Create(CreateKind),
    /// Duplicate an entity.
    Duplicate(u64),
    /// Delete an entity.
    Delete(u64),
    /// Add a component to an entity.
    AddComponent(u64, ComponentKind),
    /// Remove a component from an entity.
    RemoveComponent(u64, ComponentKind),
    /// Set a scalar property. `gesture` edits coalesce into one undo step.
    SetField {
        /// Entity id.
        id: u64,
        /// The property.
        field: Field,
        /// The new value.
        value: f32,
        /// Part of a continuous drag (coalesced undo).
        gesture: bool,
    },
    /// Replace an entity's transform (a gizmo drag; coalesced undo).
    SetTransform {
        /// Entity id.
        id: u64,
        /// The new transform.
        transform: TransformData,
    },
    /// Rename an entity.
    Rename {
        /// Entity id.
        id: u64,
        /// The new name.
        name: String,
    },
    /// Flip a boolean property.
    Toggle {
        /// Entity id.
        id: u64,
        /// The property.
        flag: Flag,
    },
    /// Advance an enumerated property to its next option.
    Cycle {
        /// Entity id.
        id: u64,
        /// The property.
        choice: Choice,
    },
    /// Reset an entity's transform.
    ResetTransform(u64),
    /// A continuous drag ended (closes the coalesced undo step).
    EndGesture,
    /// Move the selected entity by a world-space delta (arrow keys).
    NudgeSelected([f32; 3]),
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Save the document.
    Save,
    /// New scene.
    New,
    /// Enter (or resume) play mode.
    Play,
    /// Pause / unpause play mode.
    Pause,
    /// Leave play mode, restoring the edit-time scene.
    Stop,
    /// Set a color property (picker / hex entry). Gestures coalesce.
    SetColor {
        /// Entity id.
        id: u64,
        /// Which color.
        target: ColorTarget,
        /// The new RGBA (alpha ignored for RGB colors).
        rgba: [f32; 4],
        /// Part of a continuous drag (coalesced undo).
        gesture: bool,
    },
    /// Set an enumerated property to the option labelled `label`.
    SetChoice {
        /// Entity id.
        id: u64,
        /// The property.
        choice: Choice,
        /// The option's label.
        label: &'static str,
    },
    /// Reset a component to its defaults.
    ResetComponent(u64, ComponentKind),
    /// Copy one component's values from `source` onto entity `id`.
    PasteComponent {
        /// Target entity.
        id: u64,
        /// Which component.
        kind: ComponentKind,
        /// The entity data the component was copied from.
        source: Box<EntityData>,
    },
    /// Add copies of entities (paste).
    PasteEntities(Vec<EntityData>),
    /// Set (add or replace) a custom attribute.
    SetAttribute {
        /// Entity id.
        id: u64,
        /// Attribute name.
        key: String,
        /// New value.
        value: AttributeValue,
        /// Part of a continuous drag (coalesced undo).
        gesture: bool,
    },
    /// Remove a custom attribute.
    RemoveAttribute {
        /// Entity id.
        id: u64,
        /// Attribute name.
        key: String,
    },
    /// Rename a custom attribute.
    RenameAttribute {
        /// Entity id.
        id: u64,
        /// Old name.
        from: String,
        /// New name.
        to: String,
    },
    /// Open a scene file (asks to save unsaved changes first).
    Open(PathBuf),
    /// Save the scene to a new path.
    SaveAs(PathBuf),
    /// Save an entity as a prefab file.
    SavePrefab {
        /// Entity id.
        id: u64,
        /// Destination `.prefab` path.
        path: PathBuf,
    },
    /// Add a prefab's entities to the scene.
    InstantiatePrefab(PathBuf),
    /// Open a script in the code editor.
    OpenScript(PathBuf),
    /// Create a script from the template (if missing) and open it.
    NewScript(PathBuf),
    /// Attach a script to an entity (created from the template if missing).
    AttachScript {
        /// Entity id.
        id: u64,
        /// Script path.
        path: PathBuf,
    },
    /// Detach an entity's script.
    DetachScript(u64),
    /// Open a script at a line (console links, go-to-definition).
    OpenScriptAt {
        /// Script path.
        path: PathBuf,
        /// 1-based line.
        line: u32,
    },
    /// Create a folder.
    NewFolder(PathBuf),
    /// Quit the editor (asks to save unsaved changes first).
    Quit,
    /// Run an intent without the unsaved-changes prompt.
    Force(Box<Intent>),
}

impl Intent {
    /// Whether this intent edits the document (blocked during play mode).
    #[must_use]
    pub fn edits_document(&self) -> bool {
        !matches!(
            self,
            Self::Select(_)
                | Self::EndGesture
                | Self::Save
                | Self::SaveAs(_)
                | Self::SavePrefab { .. }
                | Self::OpenScript(_)
                | Self::OpenScriptAt { .. }
                | Self::NewScript(_)
                | Self::NewFolder(_)
                | Self::Play
                | Self::Pause
                | Self::Stop
                | Self::Quit
        )
    }
}

/// Whether the editor is editing or simulating the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayState {
    /// Editing (gizmos and inspector edits enabled).
    #[default]
    Editing,
    /// Simulating.
    Playing,
    /// Simulation paused.
    Paused,
}

/// Console message severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    /// Informational.
    Info,
    /// Something went wrong but the editor continues.
    Warn,
    /// An operation failed.
    Error,
}

/// One console line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Severity.
    pub level: LogLevel,
    /// Message text.
    pub text: String,
}

/// Keys that control an active text field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKey {
    /// Delete the character before the caret.
    Backspace,
    /// Clear the field.
    Delete,
    /// Commit.
    Enter,
    /// Commit.
    Tab,
    /// Cancel.
    Escape,
}

/// The mouse cursor the UI wants this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorHint {
    /// The default arrow.
    #[default]
    Default,
    /// Over a clickable control.
    Pointer,
    /// Over a text field.
    Text,
    /// Over a scrubbable number field / vertical splitter.
    ResizeH,
    /// Over a horizontal splitter.
    ResizeV,
    /// Over a gizmo handle.
    Grab,
    /// Dragging a gizmo handle.
    Grabbing,
    /// Orbiting / panning the camera.
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Name,
    Field(Field),
    /// The color picker's hex field.
    Hex(ColorTarget),
    /// The file dialog's name field.
    DialogName,
    /// A custom attribute's name (sorted index).
    AttrKey(usize),
    /// A custom attribute's value (sorted index; component for vectors).
    AttrValue(usize, u8),
}

#[derive(Debug, Clone)]
struct TextEdit {
    id: u64,
    target: EditTarget,
    buffer: String,
    rect: Rect,
    /// True until the first keystroke: typing replaces the whole value.
    fresh: bool,
    /// For attribute fields: the attribute's name and value when editing
    /// began (the commit needs both, and the UI has no document then).
    attr: Option<(String, AttributeValue)>,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    None,
    /// Left press in the scene view: a click selects; a drag orbits.
    ViewPress {
        start: (f32, f32),
        last: (f32, f32),
        orbiting: bool,
        double: bool,
    },
    /// Dragging a gizmo handle.
    Gizmo {
        id: u64,
        axis: usize,
        last: (f32, f32),
    },
    /// Press on a number field: drag scrubs; a click starts typing.
    Scrub {
        id: u64,
        field: Field,
        start_x: f32,
        start_value: f32,
        moved: bool,
        rect: Rect,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Splitter {
    Hierarchy,
    Inspector,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonStyle {
    Normal,
    Active,
    Accent,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    Play,
    Pause,
    Stop,
    Close,
}

/// The editor UI state.
pub struct Ui {
    /// The scene-view camera.
    pub camera: ViewCamera,
    /// The active transform tool.
    pub gizmo: GizmoMode,
    /// Edit / play state (set by the app).
    pub play: PlayState,
    console: Vec<LogLine>,
    drag: Drag,
    right_last: Option<(f32, f32)>,
    middle_last: Option<(f32, f32)>,
    edit: Option<TextEdit>,
    menu: Option<menus::Menu>,
    picker: Option<ColorPicker>,
    dialog: Option<Dialog>,
    /// Recently opened scenes (most recent first), set by the app.
    pub recent: Vec<PathBuf>,
    /// The project directory (file dialogs and the Project panel), set by
    /// the app.
    pub project_root: PathBuf,
    /// Panel sizes and visibility.
    pub panels: PanelSizes,
    /// Active bottom-dock tab.
    pub bottom_tab: BottomTab,
    recent_colors: Vec<[f32; 4]>,
    entity_clipboard: Vec<EntityData>,
    component_clipboard: Option<(ComponentKind, EntityData)>,
    /// The menu bar consumed this frame's click.
    bar_click_consumed: bool,
    /// Right press in the scene view: (start, moved) → a click opens the
    /// context menu, a drag orbits.
    right_click: Option<((f32, f32), bool)>,
    /// Splitter being dragged.
    splitter: Option<Splitter>,
    /// An attribute number being scrubbed.
    attr_scrub: Option<attributes::AttrScrub>,
    /// Console scroll offset from the newest line (0 = follow the tail).
    console_scroll: f32,
    /// Console levels hidden by the filter buttons.
    console_hidden: HashSet<LogLevel>,
    /// Project panel state.
    project: project_panel::ProjectState,
    collapsed: HashSet<&'static str>,
    inspector_scroll: f32,
    hierarchy_scroll: f32,
    hover_axis: Option<usize>,
    cursor: CursorHint,
    /// The raw pointer this frame.
    raw: Pointer,
    /// The pointer as widgets see it (masked under open menus).
    p: Pointer,
    /// Vertical clip band for hit-testing scrolled panel content.
    clip: (f32, f32),
    /// Intents produced this frame.
    out: Vec<Intent>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    /// Creates the UI.
    #[must_use]
    pub fn new() -> Self {
        Self {
            camera: ViewCamera::default(),
            gizmo: GizmoMode::Move,
            play: PlayState::Editing,
            console: Vec::new(),
            drag: Drag::None,
            right_last: None,
            middle_last: None,
            edit: None,
            menu: None,
            picker: None,
            dialog: None,
            recent: Vec::new(),
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            panels: PanelSizes::default(),
            bottom_tab: BottomTab::Console,
            recent_colors: Vec::new(),
            entity_clipboard: Vec::new(),
            component_clipboard: None,
            bar_click_consumed: false,
            right_click: None,
            splitter: None,
            attr_scrub: None,
            console_scroll: 0.0,
            console_hidden: HashSet::new(),
            project: project_panel::ProjectState::default(),
            collapsed: HashSet::new(),
            inspector_scroll: 0.0,
            hierarchy_scroll: 0.0,
            hover_axis: None,
            cursor: CursorHint::Default,
            raw: Pointer::default(),
            p: Pointer::default(),
            clip: (0.0, f32::INFINITY),
            out: Vec::new(),
        }
    }

    // ----- public API used by the app -------------------------------------

    /// Appends a console message.
    pub fn log(&mut self, level: LogLevel, text: impl Into<String>) {
        self.console.push(LogLine {
            level,
            text: text.into(),
        });
        if self.console.len() > 500 {
            self.console.drain(..100);
        }
    }

    /// The console contents (oldest first).
    #[must_use]
    pub fn console_lines(&self) -> &[LogLine] {
        &self.console
    }

    /// The mouse cursor the UI asked for in the last frame.
    #[must_use]
    pub fn cursor(&self) -> CursorHint {
        self.cursor
    }

    /// Whether a text field has keyboard focus (keys go to it, not to
    /// shortcuts).
    #[must_use]
    pub fn is_editing_text(&self) -> bool {
        self.edit.is_some()
    }

    /// Types text into the focused field.
    pub fn type_text(&mut self, text: &str) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        if edit.fresh {
            edit.buffer.clear();
            edit.fresh = false;
        }
        edit.buffer.extend(text.chars().filter(|c| !c.is_control()));
    }

    /// Handles a control key in the focused field. Returns the edit to apply
    /// when the field is committed.
    pub fn edit_key(&mut self, key: EditKey) -> Option<Intent> {
        let edit = self.edit.as_mut()?;
        match key {
            EditKey::Backspace => {
                if edit.fresh {
                    edit.buffer.clear();
                    edit.fresh = false;
                } else {
                    edit.buffer.pop();
                }
                None
            }
            EditKey::Delete => {
                edit.buffer.clear();
                edit.fresh = false;
                None
            }
            EditKey::Enter | EditKey::Tab => self.commit_edit(),
            EditKey::Escape => {
                if self
                    .edit
                    .as_ref()
                    .is_some_and(|e| e.target == EditTarget::DialogName)
                {
                    self.close_dialog();
                }
                self.edit = None;
                None
            }
        }
    }

    /// Starts renaming the selected entity (F2).
    pub fn begin_rename(&mut self, doc: &Document) {
        if let Some(i) = doc.selected_index() {
            self.begin_rename_id(doc, doc.entities[i].id);
        }
    }

    /// Starts renaming entity `id` in the inspector.
    fn begin_rename_id(&mut self, doc: &Document, id: u64) {
        if let Some(i) = doc.index_of(id) {
            let e = &doc.entities[i];
            self.edit = Some(TextEdit {
                id: e.id,
                target: EditTarget::Name,
                buffer: e.name().to_string(),
                rect: Rect::default(),
                fresh: true,
                attr: None,
            });
            self.inspector_scroll = 0.0;
        }
    }

    /// Frames the selected entity in the scene view (F).
    pub fn focus_selected(&mut self, doc: &Document) {
        if let Some(i) = doc.selected_index() {
            self.focus_entity(&doc.entities[i]);
        }
    }

    fn focus_entity(&mut self, e: &EntityEntry) {
        let Some(t) = e.data.transform else {
            return;
        };
        let radius = if e.data.mesh_visual.is_some() {
            (view::local_half_extents(&e.data) * Vec3::from(t.scale).abs()).length()
        } else {
            1.0
        };
        self.camera.focus(Vec3::from(t.position), radius.max(0.5));
    }

    fn commit_edit(&mut self) -> Option<Intent> {
        let edit = self.edit.take()?;
        match edit.target {
            EditTarget::Name => {
                let name = edit.buffer.trim();
                (!name.is_empty()).then(|| Intent::Rename {
                    id: edit.id,
                    name: name.to_string(),
                })
            }
            EditTarget::Field(field) => eval_number(&edit.buffer).map(|value| Intent::SetField {
                id: edit.id,
                field,
                value,
                gesture: false,
            }),
            EditTarget::Hex(target) => {
                let rgba = picker::parse_hex(&edit.buffer)?;
                self.remember_color(rgba);
                Some(Intent::SetColor {
                    id: edit.id,
                    target,
                    rgba,
                    gesture: false,
                })
            }
            EditTarget::DialogName => self.submit_dialog(&edit.buffer),
            EditTarget::AttrKey(_) => {
                let (from, _) = edit.attr?;
                let to = edit.buffer.trim().to_string();
                (!to.is_empty() && to != from).then_some(Intent::RenameAttribute {
                    id: edit.id,
                    from,
                    to,
                })
            }
            EditTarget::AttrValue(_, component) => {
                let (key, old) = edit.attr?;
                let value = attributes::parse_value(&old, component, &edit.buffer)?;
                Some(Intent::SetAttribute {
                    id: edit.id,
                    key,
                    value,
                    gesture: false,
                })
            }
        }
    }

    // ----- frame ---------------------------------------------------------

    /// The current layout for a `w`×`h` logical window.
    #[must_use]
    pub fn layout(&self, w: f32, h: f32) -> Layout {
        Layout::with_panels(w, h, &self.panels)
    }

    /// Whether a popup, picker, or dialog is open (Escape closes it).
    #[must_use]
    pub fn overlay_open(&self) -> bool {
        self.menu.is_some() || self.picker.is_some() || self.dialog.is_some()
    }

    /// Closes the topmost popup / picker / dialog. Returns false if none.
    pub fn close_overlay(&mut self) -> bool {
        if self.dialog.is_some() {
            self.close_dialog();
        } else if self.menu.is_some() {
            self.menu = None;
        } else if self.picker.is_some() {
            self.picker = None;
        } else {
            return false;
        }
        true
    }

    /// Opens a dialog (for keyboard shortcuts such as Ctrl+O, F1).
    pub fn open_dialog_kind(&mut self, kind: DialogKind, doc: &Document) {
        self.open_dialog(kind, doc);
    }

    /// Opens a script in the code editor (optionally at a 1-based line).
    pub fn open_code(&mut self, path: &std::path::Path, line: Option<u32>) {
        let at = line.map_or_else(String::new, |l| format!(":{l}"));
        self.log(LogLevel::Info, format!("Script: {}{at}", path.display()));
    }

    /// Opens the color picker for an entity's color (tools / screenshots).
    pub fn open_color_picker(&mut self, doc: &Document, id: u64, target: ColorTarget) {
        self.open_picker(doc, id, target);
    }

    /// Opens menu-bar menu `index` (0 = File …).
    pub fn open_bar_menu(&mut self, index: usize, doc: &Document) {
        let items = self.bar_items(index, doc);
        self.menu = Some(menus::Menu {
            x: 98.0,
            y: MENUBAR_H,
            items,
            bar: Some(index),
        });
    }

    /// Copies the selected entity (Ctrl+C).
    pub fn copy_selected(&mut self, doc: &Document) {
        if let Some(i) = doc.selected_index() {
            self.copy_entity(doc, doc.entities[i].id);
        }
    }

    /// The paste intent for the entity clipboard (Ctrl+V), if not empty.
    #[must_use]
    pub fn paste_intent(&self) -> Option<Intent> {
        (!self.entity_clipboard.is_empty())
            .then(|| Intent::PasteEntities(self.entity_clipboard.clone()))
    }

    /// Draws the whole editor for this frame and returns the requested edits.
    ///
    /// `viewport` is the live scene frame (RGBA8, **physical** pixels) to show
    /// in the scene view, if one was rendered.
    pub fn frame(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        pointer: Pointer,
        viewport: Option<(&[u8], u32, u32)>,
    ) -> Vec<Intent> {
        let (w, h) = canvas.logical_size();
        let lay = self.layout(w, h);
        self.cursor = CursorHint::Default;
        self.raw = pointer;
        self.p = pointer;
        self.clip = (0.0, h);
        self.bar_click_consumed = false;

        // Clicking outside the focused text field commits it.
        if pointer.pressed {
            let outside = self.edit.as_ref().is_some_and(|e| {
                e.target != EditTarget::DialogName && !e.rect.contains(pointer.x, pointer.y)
            });
            if outside {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
            }
        }

        // Overlays capture the pointer: a modal dialog masks everything; a
        // menu or picker masks what is under it, and a click outside closes
        // it.
        if self.dialog.is_some() {
            self.mask_pointer();
        } else {
            let over_menu = self
                .menu
                .as_ref()
                .map(|m| m.rect(canvas, w, h))
                .is_some_and(|r| r.contains(pointer.x, pointer.y));
            let over_bar = lay.menubar.contains(pointer.x, pointer.y);
            let over_picker = self
                .picker_rect(w, h)
                .is_some_and(|r| r.contains(pointer.x, pointer.y))
                || self.picker_dragging();
            if self.menu.is_some() {
                if pointer.pressed && !over_menu && !over_bar {
                    self.menu = None;
                    self.p.pressed = false;
                    self.p.right_pressed = false;
                } else if over_menu {
                    self.mask_pointer();
                }
            }
            if self.picker.is_some() && self.menu.is_none() {
                if pointer.pressed && !over_picker {
                    self.picker = None;
                    self.p.pressed = false;
                } else if over_picker {
                    self.mask_pointer();
                }
            }
        }

        self.update_scrub();
        self.update_attr_scrub();
        self.update_splitters(&lay, w, h);

        canvas.clear(color::background());
        // Scene view first: its overlays may spill outside its rect and are
        // covered by the panels drawn after it.
        self.scene_view(canvas, doc, lay.viewport, viewport);
        if self.panels.show_bottom {
            self.bottom_dock(canvas, doc, lay.console);
        }
        if self.panels.show_hierarchy {
            self.hierarchy(canvas, doc, lay.hierarchy);
        }
        if self.panels.show_inspector {
            self.inspector(canvas, doc, lay.inspector);
        }
        self.toolbar(canvas, doc, lay.toolbar);
        self.status_bar(canvas, doc, lay.status);
        self.clip = (0.0, h);
        self.draw_splitters(canvas, &lay);
        self.menu_bar(canvas, doc, w);
        self.draw_picker(canvas, doc, w, h);
        self.draw_menu(canvas, doc, w, h);
        self.draw_dialog(canvas, w, h);

        std::mem::take(&mut self.out)
    }

    /// Hides the pointer from ordinary widgets (an overlay owns it).
    fn mask_pointer(&mut self) {
        self.p.pressed = false;
        self.p.right_pressed = false;
        self.p.double_click = false;
        self.p.scroll = 0.0;
        self.p.x = -1.0e6;
        self.p.y = -1.0e6;
    }

    /// Splitter hit areas for the current layout.
    fn splitter_rects(&self, lay: &Layout) -> Vec<(Splitter, Rect)> {
        let mut v = Vec::new();
        if self.panels.show_hierarchy {
            let r = lay.hierarchy;
            v.push((
                Splitter::Hierarchy,
                Rect::new(r.x + r.w - 3.0, r.y, 6.0, r.h),
            ));
        }
        if self.panels.show_inspector {
            let r = lay.inspector;
            v.push((Splitter::Inspector, Rect::new(r.x - 3.0, r.y, 6.0, r.h)));
        }
        if self.panels.show_bottom {
            let r = lay.console;
            v.push((Splitter::Bottom, Rect::new(r.x, r.y - 3.0, r.w, 6.0)));
        }
        v
    }

    /// Starts, continues, and ends panel-splitter drags.
    fn update_splitters(&mut self, lay: &Layout, w: f32, h: f32) {
        let p = self.raw;
        if let Some(which) = self.splitter {
            if p.down {
                match which {
                    Splitter::Hierarchy => self.panels.hierarchy_w = p.x.clamp(140.0, w * 0.4),
                    Splitter::Inspector => {
                        self.panels.inspector_w = (w - p.x).clamp(220.0, w * 0.45);
                    }
                    Splitter::Bottom => {
                        self.panels.bottom_h = (lay.status.y - p.y).clamp(80.0, h * 0.7);
                    }
                }
                self.cursor = match which {
                    Splitter::Bottom => CursorHint::ResizeV,
                    _ => CursorHint::ResizeH,
                };
                self.mask_pointer();
            } else {
                self.splitter = None;
            }
            return;
        }
        if !matches!(self.drag, Drag::None) || self.overlay_open() {
            return;
        }
        for (which, r) in self.splitter_rects(lay) {
            if r.contains(self.p.x, self.p.y) {
                self.cursor = match which {
                    Splitter::Bottom => CursorHint::ResizeV,
                    _ => CursorHint::ResizeH,
                };
                if self.p.pressed {
                    self.splitter = Some(which);
                    self.mask_pointer();
                }
                break;
            }
        }
    }

    fn draw_splitters(&mut self, canvas: &mut Canvas, lay: &Layout) {
        let hot = self.splitter;
        for (which, r) in self.splitter_rects(lay) {
            if hot == Some(which) {
                canvas.fill_rect(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0, color::accent());
            }
        }
    }

    /// Continues (or finishes) a number-field scrub started on a press.
    fn update_scrub(&mut self) {
        let Drag::Scrub {
            id,
            field,
            start_x,
            start_value,
            moved,
            rect,
        } = self.drag
        else {
            return;
        };
        let p = self.raw;
        if p.down {
            let dx = p.x - start_x;
            let moved = moved || dx.abs() > 3.0;
            if moved {
                let mut step = fields::scrub_step(field);
                if p.shift {
                    step *= 0.1;
                }
                if p.ctrl {
                    step *= 10.0;
                }
                self.out.push(Intent::SetField {
                    id,
                    field,
                    value: start_value + dx * step,
                    gesture: true,
                });
                self.cursor = CursorHint::ResizeH;
            }
            self.drag = Drag::Scrub {
                id,
                field,
                start_x,
                start_value,
                moved,
                rect,
            };
        } else {
            self.drag = Drag::None;
            if moved {
                self.out.push(Intent::EndGesture);
            } else {
                // A click (no drag): type a value instead.
                self.edit = Some(TextEdit {
                    id,
                    target: EditTarget::Field(field),
                    buffer: fields::format_value(start_value),
                    rect,
                    fresh: true,
                    attr: None,
                });
            }
        }
    }

    // ----- widgets -------------------------------------------------------

    /// Whether the widget pointer is over `r` (and inside the clip band).
    fn hot(&self, r: Rect) -> bool {
        r.contains(self.p.x, self.p.y) && self.p.y >= self.clip.0 && self.p.y < self.clip.1
    }

    /// Whether `r` intersects the current clip band (worth drawing).
    fn row_visible(&self, r: Rect) -> bool {
        r.y + r.h > self.clip.0 && r.y < self.clip.1
    }

    fn panel_title(&self, canvas: &mut Canvas, rect: Rect, title: &str) {
        canvas.fill_rect(rect.x, rect.y, rect.w, layout::TITLEBAR_H, color::surface());
        canvas.fill_rect(
            rect.x,
            rect.y + layout::TITLEBAR_H - 1.0,
            rect.w,
            1.0,
            color::border(),
        );
        canvas.text(
            rect.x + layout::PAD,
            rect.y + (layout::TITLEBAR_H - layout::TITLE_SIZE) / 2.0 - 1.0,
            layout::TITLE_SIZE,
            title,
            color::text_dim(),
        );
    }

    fn button(&mut self, canvas: &mut Canvas, r: Rect, label: &str, style: ButtonStyle) -> bool {
        let enabled = style != ButtonStyle::Disabled;
        let hot = enabled && self.hot(r);
        let (bg, fg, border) = match style {
            ButtonStyle::Active => (color::select(), color::text(), color::accent()),
            ButtonStyle::Accent => (
                if hot { color::hover() } else { color::raised() },
                color::accent_soft(),
                color::accent(),
            ),
            ButtonStyle::Disabled => (color::surface(), color::border(), color::border()),
            ButtonStyle::Normal => (
                if hot { color::hover() } else { color::raised() },
                color::text(),
                color::border(),
            ),
        };
        canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, bg);
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, border, 1.0);
        let tw = canvas.measure_text(layout::FONT_SIZE, label).min(r.w - 6.0);
        canvas.text_clipped(
            r.x + (r.w - tw) / 2.0,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            label,
            fg,
            r.w - 6.0,
        );
        if hot {
            self.cursor = CursorHint::Pointer;
        }
        hot && self.p.pressed
    }

    fn icon_button(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        icon: Icon,
        style: ButtonStyle,
    ) -> bool {
        let clicked = self.button(canvas, r, "", style);
        let c = match style {
            ButtonStyle::Disabled => color::border(),
            ButtonStyle::Active => color::accent_soft(),
            _ => color::text(),
        };
        draw_icon(canvas, icon, r.x + r.w / 2.0, r.y + r.h / 2.0, c);
        clicked
    }

    // ----- toolbar -------------------------------------------------------

    fn toolbar(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, color::border());

        let by = rect.y + 7.0;
        let bh = rect.h - 14.0;
        let mut x = 10.0;
        for (label, intent, enabled) in [
            ("New", Intent::New, true),
            ("Save", Intent::Save, true),
            ("Undo", Intent::Undo, doc.can_undo()),
            ("Redo", Intent::Redo, doc.can_redo()),
        ] {
            let bw = canvas.measure_text(layout::FONT_SIZE, label) + 22.0;
            // Saving stays available in play mode (it saves the edit scene).
            let style = if enabled && (self.play == PlayState::Editing || intent == Intent::Save) {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, Rect::new(x, by, bw, bh), label, style) {
                self.out.push(intent);
            }
            x += bw + 4.0;
        }

        // Transform tools.
        x += 10.0;
        canvas.fill_rect(x, by + 4.0, 1.0, bh - 8.0, color::border());
        x += 11.0;
        for (mode, key) in [
            (GizmoMode::Move, "W"),
            (GizmoMode::Rotate, "E"),
            (GizmoMode::Scale, "R"),
        ] {
            let label = format!("{}  {key}", mode.label());
            let bw = canvas.measure_text(layout::FONT_SIZE, &label) + 22.0;
            let style = if self.gizmo == mode {
                ButtonStyle::Active
            } else {
                ButtonStyle::Normal
            };
            if self.button(canvas, Rect::new(x, by, bw, bh), &label, style) {
                self.gizmo = mode;
            }
            x += bw + 4.0;
        }

        // Play controls, centered (pushed right if the tools need the room).
        let bw = 38.0;
        let group = bw * 3.0 + 8.0;
        let px = (rect.w / 2.0 - group / 2.0).max(x + 16.0);
        let editing = self.play == PlayState::Editing;
        let play_style = if self.play == PlayState::Playing {
            ButtonStyle::Active
        } else {
            ButtonStyle::Accent
        };
        if self.icon_button(canvas, Rect::new(px, by, bw, bh), Icon::Play, play_style) {
            self.out.push(Intent::Play);
        }
        let pause_style = match self.play {
            PlayState::Editing => ButtonStyle::Disabled,
            PlayState::Paused => ButtonStyle::Active,
            PlayState::Playing => ButtonStyle::Normal,
        };
        if self.icon_button(
            canvas,
            Rect::new(px + bw + 4.0, by, bw, bh),
            Icon::Pause,
            pause_style,
        ) {
            self.out.push(Intent::Pause);
        }
        let stop_style = if editing {
            ButtonStyle::Disabled
        } else {
            ButtonStyle::Normal
        };
        if self.icon_button(
            canvas,
            Rect::new(px + 2.0 * (bw + 4.0), by, bw, bh),
            Icon::Stop,
            stop_style,
        ) {
            self.out.push(Intent::Stop);
        }

        // Play state badge, right-aligned.
        let badge = match self.play {
            PlayState::Editing => None,
            PlayState::Playing => Some(("PLAYING", color::accent())),
            PlayState::Paused => Some(("PAUSED", color::warn())),
        };
        if let Some((text, c)) = badge {
            let tw = canvas.measure_text(layout::SMALL_SIZE, text) + 16.0;
            let bx = rect.w - tw - layout::PAD;
            if bx > px + group + 10.0 {
                canvas.stroke_rounded(bx, by + 4.0, tw, bh - 8.0, 4.0, c, 1.0);
                canvas.text(bx + 8.0, by + bh / 2.0 - 7.0, layout::SMALL_SIZE, text, c);
            }
        }
    }

    // ----- hierarchy -----------------------------------------------------

    fn hierarchy(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x + rect.w - 1.0, rect.y, 1.0, rect.h, color::border());

        let top = rect.y + layout::TITLEBAR_H;
        let list = Rect::new(rect.x, top, rect.w, rect.h - layout::TITLEBAR_H);
        self.clip = (top, rect.y + rect.h);

        if self.hot(list) && self.p.scroll != 0.0 {
            self.hierarchy_scroll -= self.p.scroll * layout::ROW_H * 2.0;
        }
        let content_h = doc.entities.len() as f32 * layout::ROW_H + 8.0;
        let max_scroll = (content_h - list.h).max(0.0);
        self.hierarchy_scroll = self.hierarchy_scroll.clamp(0.0, max_scroll);

        let mut y = top + 4.0 - self.hierarchy_scroll;
        let mut row_clicked = false;
        for entity in &doc.entities {
            let row = Rect::new(rect.x + 4.0, y, rect.w - 9.0, layout::ROW_H);
            if self.row_visible(row) {
                let selected = doc.selected == Some(entity.id);
                let hot = self.hot(row);
                if selected {
                    canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::select());
                } else if hot {
                    canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::hover());
                }
                entity_icon(canvas, row.x + 12.0, row.y + row.h / 2.0, &entity.data);
                canvas.text_clipped(
                    row.x + 26.0,
                    row.y + 5.0,
                    layout::FONT_SIZE,
                    entity.name(),
                    if selected {
                        color::text()
                    } else {
                        color::text_dim()
                    },
                    row.w - 30.0,
                );
                if hot && self.p.pressed {
                    row_clicked = true;
                    self.out.push(Intent::Select(Some(entity.id)));
                    if self.p.double_click {
                        self.focus_entity(entity);
                    }
                }
                if hot && self.p.right_pressed {
                    row_clicked = true;
                    self.out.push(Intent::Select(Some(entity.id)));
                    let items = self.entity_menu(doc, entity.id);
                    self.open_menu(self.p.x, self.p.y, items);
                }
            }
            y += layout::ROW_H;
        }
        if doc.entities.is_empty() {
            canvas.text(
                rect.x + layout::PAD,
                top + 10.0,
                layout::SMALL_SIZE,
                "Empty scene. Use + Create.",
                color::text_dim(),
            );
        }
        // Clicking empty space clears the selection; right-click offers
        // Create / Paste.
        if !row_clicked && self.hot(list) && self.p.pressed {
            self.out.push(Intent::Select(None));
        }
        if !row_clicked && self.hot(list) && self.p.right_pressed {
            let mut items = self.create_items();
            if let Some(paste) = self.paste_intent() {
                items.insert(
                    0,
                    menus::MenuItem {
                        label: "Paste".into(),
                        shortcut: "Ctrl+V",
                        action: Some(menus::Action::Intent(paste)),
                        enabled: self.play == PlayState::Editing,
                    },
                );
            }
            self.open_menu(self.p.x, self.p.y, items);
        }

        self.clip = (0.0, f32::INFINITY);
        self.panel_title(canvas, rect, "Hierarchy");
        let add = Rect::new(
            rect.x + rect.w - 84.0,
            rect.y + 4.0,
            76.0,
            layout::TITLEBAR_H - 8.0,
        );
        let style = if self.play == PlayState::Editing {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Disabled
        };
        if self.button(canvas, add, "+ Create", style) {
            let items = self.create_items();
            self.open_menu(add.x, add.y + add.h + 2.0, items);
        }
    }

    // ----- inspector -----------------------------------------------------

    fn inspector(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y, 1.0, rect.h, color::border());

        let top = rect.y + layout::TITLEBAR_H;
        let bottom = rect.y + rect.h;
        let body = Rect::new(rect.x, top, rect.w, rect.h - layout::TITLEBAR_H);
        self.clip = (top, bottom);
        if self.hot(body) && self.p.scroll != 0.0 {
            self.inspector_scroll -= self.p.scroll * 48.0;
        }
        self.inspector_scroll = self.inspector_scroll.max(0.0);

        let x = rect.x + layout::PAD;
        let w = rect.w - 2.0 * layout::PAD;
        let start = top + 10.0 - self.inspector_scroll;
        let end = match doc.selected_index() {
            Some(idx) => self.inspector_body(canvas, doc, &doc.entities[idx], x, start, w),
            None => {
                canvas.text(
                    x,
                    start + 4.0,
                    layout::FONT_SIZE,
                    "No entity selected",
                    color::text_dim(),
                );
                canvas.text(
                    x,
                    start + 26.0,
                    layout::SMALL_SIZE,
                    "Click an object in the Scene view,",
                    color::text_dim(),
                );
                canvas.text(
                    x,
                    start + 42.0,
                    layout::SMALL_SIZE,
                    "or pick one in the Hierarchy.",
                    color::text_dim(),
                );
                start + 60.0
            }
        };
        let content_h = end - start + 20.0;
        let max_scroll = (content_h - body.h).max(0.0);
        self.inspector_scroll = self.inspector_scroll.min(max_scroll);

        self.clip = (0.0, f32::INFINITY);
        self.panel_title(canvas, rect, "Inspector");
        if self.play != PlayState::Editing {
            canvas.text(
                rect.x + rect.w - 128.0,
                rect.y + 8.0,
                layout::SMALL_SIZE,
                "read-only (playing)",
                color::warn(),
            );
        }
    }

    /// Draws the selected entity's components. Returns the y after the last row.
    fn inspector_body(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        e: &EntityEntry,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let mut y = y;
        let id = e.id;
        let d = &e.data;

        // Header: icon + editable name.
        let header = Rect::new(x, y, w, 24.0);
        if self.row_visible(header) {
            entity_icon(canvas, x + 9.0, y + 12.0, d);
            self.name_field(canvas, Rect::new(x + 24.0, y, w - 24.0, 24.0), id, e.name());
        }
        y += 30.0;
        if self.row_visible(Rect::new(x, y, w, 14.0)) {
            canvas.text(
                x,
                y,
                layout::SMALL_SIZE,
                &format!("Entity #{id}"),
                color::text_dim(),
            );
        }
        y += 22.0;

        if d.transform.is_some() {
            let (ny, open) = self.section(canvas, x, y, w, "Transform", id, None, true);
            y = ny;
            if open {
                y = self.vec3_row(canvas, x, y, w, e, "Position", Field::Position);
                y = self.vec3_row(canvas, x, y, w, e, "Rotation", Field::Rotation);
                y = self.vec3_row(canvas, x, y, w, e, "Scale", Field::Scale);
                y += 6.0;
            }
        }
        if d.mesh_visual.is_some() {
            let kind = ComponentKind::Mesh;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.choice_row(canvas, x, y, w, e, "Primitive", Choice::MeshPrimitive);
                y = self.float_row(canvas, x, y, w, e, "Size", Field::MeshSize);
                y = self.color_row(canvas, x, y, w, e, "Color", ColorTarget::Mesh);
                y += 6.0;
            }
        }
        if d.directional_light.is_some() {
            let kind = ComponentKind::Light;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.vec3_row(canvas, x, y, w, e, "Direction", Field::LightDirection);
                y = self.color_row(canvas, x, y, w, e, "Color", ColorTarget::Light);
                y = self.float_row(canvas, x, y, w, e, "Ambient", Field::LightAmbient);
                y = self.flag_row(canvas, x, y, w, e, "Cast Shadows", Flag::LightShadows);
                y += 6.0;
            }
        }
        if d.camera_3d.is_some() {
            let kind = ComponentKind::Camera;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.float_row(canvas, x, y, w, e, "Field of View", Field::CameraFov);
                y = self.float_row(canvas, x, y, w, e, "Near Clip", Field::CameraNear);
                y = self.float_row(canvas, x, y, w, e, "Far Clip", Field::CameraFar);
                y = self.vec3_row(canvas, x, y, w, e, "Eye", Field::CameraPosition);
                y = self.vec3_row(canvas, x, y, w, e, "Look At", Field::CameraTarget);
                y += 6.0;
            }
        }
        if d.rigid_body.is_some() {
            let kind = ComponentKind::RigidBody;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.choice_row(canvas, x, y, w, e, "Body Type", Choice::BodyType);
                y = self.float_row(canvas, x, y, w, e, "Gravity Scale", Field::BodyGravityScale);
                y = self.float_row(canvas, x, y, w, e, "Linear Damp", Field::BodyLinearDamping);
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Angular Damp",
                    Field::BodyAngularDamping,
                );
                y = self.flag_row(canvas, x, y, w, e, "Enabled", Flag::BodyEnabled);
                y += 6.0;
            }
        }
        if d.collider_3d.is_some() {
            let kind = ComponentKind::Collider;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.choice_row(canvas, x, y, w, e, "Shape", Choice::ColliderShape);
                y = self.vec3_row(canvas, x, y, w, e, "Size", Field::ColliderExtents);
                y = self.float_row(canvas, x, y, w, e, "Radius", Field::ColliderRadius);
                y = self.float_row(canvas, x, y, w, e, "Half Height", Field::ColliderHalfHeight);
                y = self.float_row(canvas, x, y, w, e, "Density", Field::ColliderDensity);
                y = self.float_row(canvas, x, y, w, e, "Friction", Field::ColliderFriction);
                y = self.float_row(canvas, x, y, w, e, "Bounciness", Field::ColliderRestitution);
                y = self.flag_row(canvas, x, y, w, e, "Is Trigger", Flag::ColliderSensor);
                y = self.flag_row(canvas, x, y, w, e, "Enabled", Flag::ColliderEnabled);
                y += 6.0;
            }
        }

        if d.character_controller.is_some() {
            let kind = ComponentKind::Character;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.float_row(canvas, x, y, w, e, "Max Slope", Field::CharacterSlope);
                for line in [
                    "Play mode: WASD / arrows move,",
                    "Space jumps, Shift sprints.",
                ] {
                    if self.row_visible(Rect::new(x, y, w, 16.0)) {
                        canvas.text(x, y, layout::SMALL_SIZE, line, color::text_dim());
                    }
                    y += 16.0;
                }
                y += 8.0;
            }
        }

        y = self.script_section(canvas, doc, e, x, y, w);
        y = self.attributes_section(canvas, e, x, y, w);

        // Add Component / Delete.
        y += 8.0;
        let add = Rect::new(x + w * 0.12, y, w * 0.76, 26.0);
        if self.row_visible(add) {
            let style = if self.play == PlayState::Editing {
                ButtonStyle::Accent
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, add, "Add Component", style) {
                let items = self.bar_items(3, doc);
                self.open_menu(add.x, add.y + add.h + 2.0, items);
            }
        }
        y += 34.0;
        let del = Rect::new(x + w * 0.12, y, w * 0.76, 24.0);
        if self.row_visible(del) {
            let style = if self.play == PlayState::Editing {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, del, "Delete Entity", style) {
                self.out.push(Intent::Delete(id));
            }
        }
        y + 30.0
    }

    /// A collapsible component header. Returns `(next_y, expanded)`.
    fn section(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        title: &'static str,
        id: u64,
        removable: Option<ComponentKind>,
        resettable: bool,
    ) -> (f32, bool) {
        let header = Rect::new(x - 4.0, y, w + 8.0, 24.0);
        let open = !self.collapsed.contains(title);
        if self.row_visible(header) {
            canvas.fill_rounded(header.x, header.y, header.w, header.h, 3.0, color::raised());
            let (cx, cy) = (x + 5.0, y + 12.0);
            if open {
                canvas.fill_polygon(
                    &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
                    color::text_dim(),
                );
            } else {
                canvas.fill_polygon(
                    &[(cx - 2.0, cy - 4.0), (cx + 3.0, cy), (cx - 2.0, cy + 4.0)],
                    color::text_dim(),
                );
            }
            canvas.text(
                x + 16.0,
                y + 5.0,
                layout::FONT_SIZE,
                title,
                color::accent_soft(),
            );

            let editing = self.play == PlayState::Editing;
            let mut right = x + w;
            if let Some(kind) = removable {
                let r = Rect::new(right - 22.0, y + 3.0, 22.0, 18.0);
                let style = if editing {
                    ButtonStyle::Normal
                } else {
                    ButtonStyle::Disabled
                };
                if self.icon_button(canvas, r, Icon::Close, style) {
                    self.out.push(Intent::RemoveComponent(id, kind));
                }
                right -= 26.0;
            }
            if resettable || removable.is_some() {
                let r = Rect::new(right - 22.0, y + 3.0, 22.0, 18.0);
                if self.kebab_button(canvas, r) {
                    let items = self.component_menu(id, removable);
                    self.open_menu(r.x, r.y + r.h + 2.0, items);
                }
                right -= 26.0;
            }
            let toggle_area = Rect::new(header.x, header.y, (right - header.x).max(0.0), header.h);
            if self.hot(toggle_area) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    if open {
                        self.collapsed.insert(title);
                    } else {
                        self.collapsed.remove(title);
                    }
                }
            }
        }
        (y + 30.0, open)
    }

    fn row_label(&self, canvas: &mut Canvas, x: f32, y: f32, label: &str) {
        canvas.text_clipped(
            x,
            y + 3.0,
            layout::FONT_SIZE,
            label,
            color::text_dim(),
            layout::LABEL_W - 8.0,
        );
    }

    fn vec3_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        label: &str,
        field: fn(u8) -> Field,
    ) -> f32 {
        if fields::get(&e.data, field(0)).is_none() {
            return y;
        }
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let gap = 4.0;
            let fw = (w - layout::LABEL_W - 2.0 * gap) / 3.0;
            let tags = [color::axis_x(), color::axis_y(), color::axis_z()];
            for (i, tag) in tags.into_iter().enumerate() {
                let f = field(i as u8);
                if let Some(v) = fields::get(&e.data, f) {
                    let fx = x + layout::LABEL_W + i as f32 * (fw + gap);
                    self.float_field(
                        canvas,
                        Rect::new(fx, y, fw, layout::FIELD_H),
                        e.id,
                        f,
                        v,
                        Some(tag),
                    );
                }
            }
        }
        y + layout::ROW_STEP
    }

    fn float_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        label: &str,
        field: Field,
    ) -> f32 {
        let Some(v) = fields::get(&e.data, field) else {
            return y;
        };
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let fr = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
            self.float_field(canvas, fr, e.id, field, v, None);
        }
        y + layout::ROW_STEP
    }

    fn color_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        label: &str,
        target: ColorTarget,
    ) -> f32 {
        let Some(rgba) = target.read(&e.data) else {
            return y;
        };
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let sw = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
            self.color_swatch(canvas, sw, e.id, target, rgba);
        }
        y + layout::ROW_STEP
    }

    /// A clickable color swatch with its hex code; opens the color picker.
    fn color_swatch(
        &mut self,
        canvas: &mut Canvas,
        sw: Rect,
        id: u64,
        target: ColorTarget,
        rgba: [f32; 4],
    ) {
        let c = |v: f32| v.clamp(0.0, 1.0);
        let opaque =
            Color::from_rgba(c(rgba[0]), c(rgba[1]), c(rgba[2]), 1.0).unwrap_or(Color::WHITE);
        let hot = self.hot(sw);
        canvas.fill_rounded(sw.x, sw.y, sw.w, sw.h, 3.0, opaque);
        if target.has_alpha() {
            // Alpha strip along the bottom edge.
            let a = c(rgba[3]);
            canvas.fill_rect(sw.x + 2.0, sw.y + sw.h - 4.0, sw.w - 4.0, 2.0, Color::BLACK);
            canvas.fill_rect(
                sw.x + 2.0,
                sw.y + sw.h - 4.0,
                (sw.w - 4.0) * a,
                2.0,
                Color::WHITE,
            );
        }
        canvas.stroke_rounded(
            sw.x,
            sw.y,
            sw.w,
            sw.h,
            3.0,
            if hot {
                color::accent()
            } else {
                color::border()
            },
            1.0,
        );
        let luma = 0.299 * rgba[0] + 0.587 * rgba[1] + 0.114 * rgba[2];
        let fg = if luma > 0.55 {
            Color::from_rgba8(20, 20, 24, 255)
        } else {
            Color::WHITE
        };
        canvas.text(
            sw.x + 6.0,
            sw.y + 3.0,
            layout::SMALL_SIZE,
            &picker::to_hex(rgba, target.has_alpha()),
            fg,
        );
        if hot {
            self.cursor = CursorHint::Pointer;
            if self.p.pressed {
                self.open_picker_rgba(id, target, rgba);
            }
        }
    }

    fn flag_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        label: &str,
        flag: Flag,
    ) -> f32 {
        let Some(on) = fields::get_flag(&e.data, flag) else {
            return y;
        };
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let b = Rect::new(x + layout::LABEL_W, y + 1.0, 18.0, 18.0);
            let hot = self.hot(b);
            canvas.fill_rounded(
                b.x,
                b.y,
                b.w,
                b.h,
                3.0,
                if on {
                    color::accent()
                } else if hot {
                    color::hover()
                } else {
                    color::field()
                },
            );
            canvas.stroke_rounded(b.x, b.y, b.w, b.h, 3.0, color::border(), 1.0);
            if on {
                canvas.polyline(
                    &[
                        (b.x + 4.0, b.y + 9.5),
                        (b.x + 7.5, b.y + 13.0),
                        (b.x + 14.0, b.y + 5.0),
                    ],
                    color::text(),
                    2.0,
                );
            }
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed && self.play == PlayState::Editing {
                    self.out.push(Intent::Toggle { id: e.id, flag });
                }
            }
        }
        y + layout::ROW_STEP
    }

    fn choice_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
        label: &str,
        choice: Choice,
    ) -> f32 {
        let Some(value) = fields::choice_label(&e.data, choice) else {
            return y;
        };
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let b = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
            let hot = self.hot(b);
            canvas.fill_rounded(
                b.x,
                b.y,
                b.w,
                b.h,
                3.0,
                if hot { color::hover() } else { color::raised() },
            );
            canvas.stroke_rounded(b.x, b.y, b.w, b.h, 3.0, color::border(), 1.0);
            canvas.text_clipped(
                b.x + 6.0,
                b.y + 3.0,
                layout::FONT_SIZE,
                value,
                color::text(),
                b.w - 24.0,
            );
            let (cx, cy) = (b.x + b.w - 11.0, b.y + b.h / 2.0);
            canvas.fill_polygon(
                &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
                color::text_dim(),
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed && self.play == PlayState::Editing {
                    let items = fields::choice_options(&e.data, choice)
                        .into_iter()
                        .map(|label| menus::MenuItem {
                            label: if label == value {
                                format!("✓ {label}")
                            } else {
                                format!("   {label}")
                            },
                            shortcut: "",
                            action: Some(menus::Action::Intent(Intent::SetChoice {
                                id: e.id,
                                choice,
                                label,
                            })),
                            enabled: true,
                        })
                        .collect();
                    self.open_menu(b.x, b.y + b.h + 2.0, items);
                }
            }
        }
        y + layout::ROW_STEP
    }

    fn is_editing(&self, id: u64, target: EditTarget) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|ed| ed.id == id && ed.target == target)
    }

    /// Draws the focused text field's buffer and caret, updating its rect.
    fn draw_edit_text(&mut self, canvas: &mut Canvas, r: Rect, tx: f32) {
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        edit.rect = r;
        let text = edit.buffer.clone();
        let fresh = edit.fresh;
        let max_w = r.x + r.w - tx - 4.0;
        let tw = canvas.measure_text(layout::FONT_SIZE, &text).min(max_w);
        if fresh && !text.is_empty() {
            canvas.fill_rect(tx - 1.0, r.y + 3.0, tw + 2.0, r.h - 6.0, color::select());
        }
        canvas.text_clipped(
            tx,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            &text,
            color::text(),
            max_w,
        );
        canvas.fill_rect(
            (tx + tw + 1.0).min(r.x + r.w - 3.0),
            r.y + 3.0,
            1.5,
            r.h - 6.0,
            color::accent_soft(),
        );
    }

    fn float_field(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        id: u64,
        field: Field,
        value: f32,
        tag: Option<Color>,
    ) {
        let editing = self.is_editing(id, EditTarget::Field(field));
        let hot = self.hot(r);
        let scrubbing = matches!(
            self.drag,
            Drag::Scrub { id: did, field: df, moved: true, .. } if did == id && df == field
        );
        let bg = if editing {
            color::viewport_bg()
        } else if hot || scrubbing {
            color::hover()
        } else {
            color::field()
        };
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, bg);
        canvas.stroke_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            3.0,
            if editing {
                color::accent()
            } else {
                color::border()
            },
            1.0,
        );
        let (lo, hi) = fields::range(field);
        if !editing && lo.abs() < 1e-6 && [1.0, 89.0, 179.0].iter().any(|b| (hi - b).abs() < 1e-6) {
            // Bounded values read as sliders: a fill shows where in the range
            // the value sits.
            let t = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
            canvas.fill_rounded(
                r.x + 1.0,
                r.y + 1.0,
                (r.w - 2.0) * t,
                r.h - 2.0,
                2.0,
                color::select(),
            );
        }
        let mut tx = r.x + 5.0;
        if let Some(c) = tag {
            canvas.fill_rounded(r.x + 1.0, r.y + 2.0, 3.0, r.h - 4.0, 1.5, c);
            tx += 3.0;
        }
        if editing {
            self.draw_edit_text(canvas, r, tx);
            self.cursor = CursorHint::Text;
            return;
        }
        canvas.text_clipped(
            tx,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            &fields::format_value(value),
            color::text(),
            r.x + r.w - tx - 3.0,
        );
        if self.play != PlayState::Editing {
            return;
        }
        if hot {
            self.cursor = CursorHint::ResizeH;
            if self.p.pressed && matches!(self.drag, Drag::None) {
                self.drag = Drag::Scrub {
                    id,
                    field,
                    start_x: self.p.x,
                    start_value: value,
                    moved: false,
                    rect: r,
                };
            }
        }
    }

    fn name_field(&mut self, canvas: &mut Canvas, r: Rect, id: u64, name: &str) {
        let editing = self.is_editing(id, EditTarget::Name);
        let hot = self.hot(r);
        canvas.fill_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            3.0,
            if editing {
                color::viewport_bg()
            } else if hot {
                color::hover()
            } else {
                color::field()
            },
        );
        canvas.stroke_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            3.0,
            if editing {
                color::accent()
            } else {
                color::border()
            },
            1.0,
        );
        if editing {
            self.draw_edit_text(canvas, r, r.x + 7.0);
            self.cursor = CursorHint::Text;
            return;
        }
        canvas.text_clipped(
            r.x + 7.0,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            name,
            color::text(),
            r.w - 12.0,
        );
        if hot && self.play == PlayState::Editing {
            self.cursor = CursorHint::Text;
            if self.p.pressed {
                self.edit = Some(TextEdit {
                    id,
                    target: EditTarget::Name,
                    buffer: name.to_string(),
                    rect: r,
                    fresh: true,
                    attr: None,
                });
            }
        }
    }

    // ----- scene view ----------------------------------------------------

    fn scene_view(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        rect: Rect,
        frame: Option<(&[u8], u32, u32)>,
    ) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::viewport_bg());
        if let Some((rgba, fw, fh)) = frame {
            let s = canvas.scale();
            let (lw, lh) = (fw as f32 / s, fh as f32 / s);
            let dx = rect.x + ((rect.w - lw) / 2.0).max(0.0);
            let dy = rect.y + ((rect.h - lh) / 2.0).max(0.0);
            canvas.blit_rgba(dx, dy, fw, fh, rgba);
        }

        let proj = Projector::new(&self.camera, rect);
        let p = self.p;
        let raw = self.raw;
        let inside = rect.contains(p.x, p.y);
        let editing = self.play == PlayState::Editing;
        let selected = doc.selected_index().map(|i| &doc.entities[i]);
        let gizmo = if editing {
            selected
                .and_then(|e| e.data.transform)
                .and_then(|t| Gizmo::build(self.gizmo, &t, &proj))
        } else {
            None
        };

        // --- Camera: wheel zoom, RMB orbit, MMB pan (continue outside). ---
        if inside && p.scroll != 0.0 {
            self.camera.zoom(p.scroll);
        }
        // Right click (no drag) opens the context menu for what is under
        // the cursor; right drag orbits.
        if self.p.right_pressed && inside {
            self.right_click = Some(((p.x, p.y), false));
        }
        if let Some((start, moved)) = self.right_click {
            if raw.right_down {
                if (raw.x - start.0).abs() + (raw.y - start.1).abs() > 4.0 {
                    self.right_click = Some((start, true));
                }
            } else {
                self.right_click = None;
                if !moved {
                    let items = match view::pick(doc, &proj, start.0, start.1) {
                        Some(id) => {
                            self.out.push(Intent::Select(Some(id)));
                            self.entity_menu(doc, id)
                        }
                        None => {
                            let mut items = self.create_items();
                            if let Some(paste) = self.paste_intent() {
                                items.insert(
                                    0,
                                    menus::MenuItem {
                                        label: "Paste".into(),
                                        shortcut: "Ctrl+V",
                                        action: Some(menus::Action::Intent(paste)),
                                        enabled: editing,
                                    },
                                );
                            }
                            items
                        }
                    };
                    self.open_menu(start.0, start.1, items);
                }
            }
        }
        if raw.right_down {
            match self.right_last {
                Some(last) => {
                    self.camera.orbit(raw.x - last.0, raw.y - last.1);
                    self.right_last = Some((raw.x, raw.y));
                    self.cursor = CursorHint::Move;
                }
                None if inside => self.right_last = Some((raw.x, raw.y)),
                None => {}
            }
        } else {
            self.right_last = None;
        }
        if raw.middle_down {
            match self.middle_last {
                Some(last) => {
                    self.camera.pan(raw.x - last.0, raw.y - last.1);
                    self.middle_last = Some((raw.x, raw.y));
                    self.cursor = CursorHint::Move;
                }
                None if inside => self.middle_last = Some((raw.x, raw.y)),
                None => {}
            }
        } else {
            self.middle_last = None;
        }

        // --- Left button: select / gizmo drag / orbit drag. ---
        let drag = self.drag;
        match drag {
            Drag::Gizmo { id, axis, last } => {
                if raw.down {
                    let delta = (raw.x - last.0, raw.y - last.1);
                    if delta != (0.0, 0.0) {
                        let t = doc
                            .index_of(id)
                            .and_then(|i| doc.entities[i].data.transform);
                        if let Some(t) = t {
                            if let Some(g) = Gizmo::build(self.gizmo, &t, &proj) {
                                let transform = g.drag(axis, &t, (raw.x, raw.y), delta);
                                self.out.push(Intent::SetTransform { id, transform });
                            }
                        }
                    }
                    self.drag = Drag::Gizmo {
                        id,
                        axis,
                        last: (raw.x, raw.y),
                    };
                    self.cursor = CursorHint::Grabbing;
                } else {
                    self.drag = Drag::None;
                    self.out.push(Intent::EndGesture);
                }
            }
            Drag::ViewPress {
                start,
                last,
                orbiting,
                double,
            } => {
                if raw.down {
                    let dist = ((raw.x - start.0).powi(2) + (raw.y - start.1).powi(2)).sqrt();
                    let orbiting = orbiting || dist > 4.0;
                    if orbiting {
                        self.camera.orbit(raw.x - last.0, raw.y - last.1);
                        self.cursor = CursorHint::Move;
                    }
                    self.drag = Drag::ViewPress {
                        start,
                        last: (raw.x, raw.y),
                        orbiting,
                        double,
                    };
                } else {
                    self.drag = Drag::None;
                    if !orbiting {
                        let hit = view::pick(doc, &proj, start.0, start.1);
                        self.out.push(Intent::Select(hit));
                        if double {
                            if let Some(e) = hit
                                .and_then(|id| doc.index_of(id))
                                .map(|i| &doc.entities[i])
                            {
                                self.focus_entity(e);
                            }
                        }
                    }
                }
            }
            Drag::None if inside && p.pressed => {
                let handle = if p.alt {
                    None
                } else {
                    gizmo.as_ref().and_then(|g| g.hit(p.x, p.y))
                };
                self.drag = match (handle, selected) {
                    (Some(axis), Some(e)) => Drag::Gizmo {
                        id: e.id,
                        axis,
                        last: (p.x, p.y),
                    },
                    _ => Drag::ViewPress {
                        start: (p.x, p.y),
                        last: (p.x, p.y),
                        orbiting: p.alt,
                        double: p.double_click,
                    },
                };
            }
            _ => {}
        }
        self.hover_axis = match self.drag {
            Drag::Gizmo { axis, .. } => Some(axis),
            Drag::None if inside => gizmo.as_ref().and_then(|g| g.hit(p.x, p.y)),
            _ => None,
        };
        if self.hover_axis.is_some() && matches!(self.drag, Drag::None) {
            self.cursor = CursorHint::Grab;
        }

        // --- Overlays. ---
        draw_markers(canvas, doc, &proj, rect);
        if let Some(e) = selected {
            draw_selection(canvas, &e.data, &proj);
        }
        if let Some(g) = &gizmo {
            draw_gizmo(canvas, g, self.hover_axis);
        }
        self.draw_axes_widget(canvas, rect);

        // Header pill: view + active tool.
        let label = match self.play {
            PlayState::Editing => format!("Scene  ·  {} tool", self.gizmo.label()),
            PlayState::Playing => "Playing  ·  edits are disabled".to_string(),
            PlayState::Paused => "Paused".to_string(),
        };
        let tw = canvas.measure_text(layout::SMALL_SIZE, &label);
        canvas.fill_rounded(
            rect.x + 8.0,
            rect.y + 8.0,
            tw + 16.0,
            20.0,
            10.0,
            color::surface(),
        );
        canvas.text(
            rect.x + 16.0,
            rect.y + 11.0,
            layout::SMALL_SIZE,
            &label,
            if editing {
                color::text_dim()
            } else {
                color::warn()
            },
        );
        if editing {
            canvas.stroke_rect(rect.x, rect.y, rect.w, rect.h, color::border(), 1.0);
        } else {
            canvas.stroke_rect(
                rect.x + 1.0,
                rect.y + 1.0,
                rect.w - 2.0,
                rect.h - 2.0,
                color::accent(),
                2.0,
            );
        }
    }

    /// The orientation widget in the scene view's top-right corner.
    fn draw_axes_widget(&self, canvas: &mut Canvas, rect: Rect) {
        let (cx, cy) = (rect.x + rect.w - 44.0, rect.y + 46.0);
        if rect.w < 140.0 || rect.h < 100.0 {
            return;
        }
        canvas.fill_circle(cx, cy, 32.0, color::surface());
        let eye = self.camera.eye();
        let f = (Vec3::from(self.camera.target) - eye).normalize_or_zero();
        let r = f.cross(Vec3::Y).normalize_or_zero();
        let u = r.cross(f);
        let axes = [
            (Vec3::X, color::axis_x(), "X"),
            (Vec3::Y, color::axis_y(), "Y"),
            (Vec3::Z, color::axis_z(), "Z"),
        ];
        // Draw back-to-front so the axis facing the viewer is on top.
        let mut order: Vec<usize> = vec![0, 1, 2];
        order.sort_by(|&a, &b| {
            axes[b]
                .0
                .dot(f)
                .partial_cmp(&axes[a].0.dot(f))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in order {
            let (axis, c, label) = axes[i];
            let (sx, sy) = (axis.dot(r), -axis.dot(u));
            let (tx, ty) = (cx + sx * 22.0, cy + sy * 22.0);
            canvas.line(cx, cy, tx, ty, c, 2.0);
            canvas.fill_circle(tx, ty, 7.0, c);
            canvas.text(
                tx - 3.5,
                ty - 7.0,
                layout::SMALL_SIZE,
                label,
                color::background(),
            );
        }
    }

    // ----- console & status -----------------------------------------------

    /// The bottom dock: Console and Project tabs.
    fn bottom_dock(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y, rect.w, 1.0, color::border());
        let body = Rect::new(
            rect.x,
            rect.y + layout::TITLEBAR_H,
            rect.w,
            (rect.h - layout::TITLEBAR_H).max(0.0),
        );
        match self.bottom_tab {
            BottomTab::Console => self.console_body(canvas, body),
            BottomTab::Project => self.project_body(canvas, doc, body),
        }
        // Tab strip.
        canvas.fill_rect(
            rect.x,
            rect.y + 1.0,
            rect.w,
            layout::TITLEBAR_H - 1.0,
            color::surface(),
        );
        canvas.fill_rect(
            rect.x,
            rect.y + layout::TITLEBAR_H - 1.0,
            rect.w,
            1.0,
            color::border(),
        );
        let mut x = rect.x + 6.0;
        let errors = self
            .console
            .iter()
            .filter(|l| l.level == LogLevel::Error)
            .count();
        let console_label = if errors > 0 {
            format!("Console  ({})  ×{errors}", self.console.len())
        } else {
            format!("Console  ({})", self.console.len())
        };
        for (tab, label) in [
            (BottomTab::Console, console_label),
            (BottomTab::Project, "Project".to_string()),
        ] {
            let tw = canvas.measure_text(layout::TITLE_SIZE, &label) + 22.0;
            let r = Rect::new(x, rect.y + 3.0, tw, layout::TITLEBAR_H - 4.0);
            let active = self.bottom_tab == tab;
            if active {
                canvas.fill_rounded(r.x, r.y, r.w, r.h + 4.0, 4.0, color::raised());
                canvas.fill_rect(r.x + 4.0, r.y, r.w - 8.0, 2.0, color::accent());
            } else if self.hot(r) {
                canvas.fill_rounded(r.x, r.y, r.w, r.h, 4.0, color::hover());
            }
            canvas.text(
                r.x + 11.0,
                r.y + (r.h - layout::TITLE_SIZE) / 2.0 - 1.0,
                layout::TITLE_SIZE,
                &label,
                if active {
                    color::text()
                } else {
                    color::text_dim()
                },
            );
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    self.bottom_tab = tab;
                }
            }
            x += tw + 2.0;
        }
        if self.bottom_tab == BottomTab::Console {
            let clear = Rect::new(
                rect.x + rect.w - 64.0,
                rect.y + 4.0,
                56.0,
                layout::TITLEBAR_H - 8.0,
            );
            if self.button(canvas, clear, "Clear", ButtonStyle::Normal) {
                self.console.clear();
                self.console_scroll = 0.0;
            }
            let mut fx = clear.x - 6.0;
            for (level, label) in [
                (LogLevel::Error, "Errors"),
                (LogLevel::Warn, "Warnings"),
                (LogLevel::Info, "Info"),
            ] {
                let bw = canvas.measure_text(layout::SMALL_SIZE, label) + 18.0;
                fx -= bw + 4.0;
                let r = Rect::new(fx, rect.y + 4.0, bw, layout::TITLEBAR_H - 8.0);
                let on = !self.console_hidden.contains(&level);
                let style = if on {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Normal
                };
                if self.button(canvas, r, label, style) {
                    if on {
                        self.console_hidden.insert(level);
                    } else {
                        self.console_hidden.remove(&level);
                    }
                }
            }
        }
    }

    fn console_body(&mut self, canvas: &mut Canvas, rect: Rect) {
        let line_h = 17.0;
        let lines: Vec<usize> = self
            .console
            .iter()
            .enumerate()
            .filter(|(_, l)| !self.console_hidden.contains(&l.level))
            .map(|(i, _)| i)
            .collect();
        let visible = ((rect.h - 8.0) / line_h).floor().max(0.0) as usize;
        let max_scroll = lines.len().saturating_sub(visible) as f32 * line_h;
        if self.hot(rect) && self.p.scroll != 0.0 {
            // Scrolling up moves into history; scroll = 0 follows the tail.
            self.console_scroll =
                (self.console_scroll + self.p.scroll * line_h * 3.0).clamp(0.0, max_scroll);
        }
        self.console_scroll = self.console_scroll.min(max_scroll);
        let skip_from_end = (self.console_scroll / line_h).round() as usize;
        let end = lines.len().saturating_sub(skip_from_end);
        let start = end.saturating_sub(visible);
        let prev = canvas.set_clip(Some((rect.x, rect.y, rect.w, rect.h)));
        let mut y = rect.y + 4.0;
        let mut open = None;
        for &i in &lines[start..end] {
            let line = &self.console[i];
            let (c, tag) = match line.level {
                LogLevel::Info => (color::text_dim(), "•"),
                LogLevel::Warn => (color::warn(), "!"),
                LogLevel::Error => (color::danger(), "×"),
            };
            let row = Rect::new(rect.x, y - 1.0, rect.w, line_h);
            let link = console_location(&line.text);
            if link.is_some() && self.hot(row) {
                canvas.fill_rect(row.x, row.y, row.w, row.h, color::hover());
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    open.clone_from(&link);
                }
            }
            canvas.text(rect.x + layout::PAD, y, layout::SMALL_SIZE, tag, c);
            canvas.text_clipped(
                rect.x + layout::PAD + 14.0,
                y,
                layout::SMALL_SIZE,
                &line.text,
                if link.is_some() {
                    color::accent_soft()
                } else {
                    c
                },
                rect.w - 2.0 * layout::PAD - 14.0,
            );
            y += line_h;
        }
        canvas.restore_clip(prev);
        if let Some((path, line)) = open {
            self.out.push(Intent::OpenScriptAt { path, line });
        }
    }

    fn status_bar(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        canvas.fill_rect(rect.x, rect.y, rect.w, 1.0, color::border());
        let mut left = format!("{} entities", doc.entity_count());
        if let Some(i) = doc.selected_index() {
            left.push_str("  ·  ");
            left.push_str(doc.entities[i].name());
        }
        let lw = canvas.text(
            rect.x + layout::PAD,
            rect.y + 5.0,
            layout::SMALL_SIZE,
            &left,
            color::text_dim(),
        );
        let hint = match self.play {
            PlayState::Editing => {
                "Click select · Drag/RMB orbit · MMB pan · Wheel zoom · W/E/R tools · F focus · F2 rename · Ctrl+D duplicate · Del delete · Ctrl+Z/Y undo/redo · Ctrl+P play"
            }
            _ => "Play mode: the simulation runs on a copy of the scene · Stop (or Ctrl+P) returns to editing",
        };
        let room = rect.w - lw - 3.0 * layout::PAD;
        if room > 60.0 {
            let tw = canvas.measure_text(layout::SMALL_SIZE, hint).min(room);
            canvas.text_clipped(
                rect.x + rect.w - tw - layout::PAD,
                rect.y + 5.0,
                layout::SMALL_SIZE,
                hint,
                color::text_dim(),
                room,
            );
        }
    }
}

/// Finds a `path:line` reference in a console line (Python tracebacks print
/// `File "x.py", line 12`; linters print `x.py:12:`).
#[must_use]
pub fn console_location(text: &str) -> Option<(PathBuf, u32)> {
    if let Some(start) = text.find("File \"") {
        let rest = &text[start + 6..];
        let end = rest.find('"')?;
        let path = &rest[..end];
        let after = &rest[end..];
        let line_at = after.find("line ")?;
        let digits: String = after[line_at + 5..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("py"))
        {
            return Some((PathBuf::from(path), digits.parse().ok()?));
        }
    }
    let idx = text.find(".py:")?;
    let begin = text[..idx]
        .rfind(|c: char| c.is_whitespace() || c == '(' || c == '"')
        .map_or(0, |i| i + 1);
    let path = &text[begin..idx + 3];
    let digits: String = text[idx + 4..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((PathBuf::from(path), digits.parse().ok()?))
}

// ----- free drawing helpers -------------------------------------------------

fn draw_icon(canvas: &mut Canvas, icon: Icon, cx: f32, cy: f32, c: Color) {
    match icon {
        Icon::Play => canvas.fill_polygon(
            &[(cx - 4.0, cy - 6.0), (cx + 6.0, cy), (cx - 4.0, cy + 6.0)],
            c,
        ),
        Icon::Pause => {
            canvas.fill_rect(cx - 5.0, cy - 6.0, 3.5, 12.0, c);
            canvas.fill_rect(cx + 1.5, cy - 6.0, 3.5, 12.0, c);
        }
        Icon::Stop => canvas.fill_rect(cx - 5.0, cy - 5.0, 10.0, 10.0, c),
        Icon::Close => {
            canvas.line(cx - 4.0, cy - 4.0, cx + 4.0, cy + 4.0, c, 1.6);
            canvas.line(cx - 4.0, cy + 4.0, cx + 4.0, cy - 4.0, c, 1.6);
        }
    }
}

/// A small type icon: light, camera, mesh, or empty.
fn entity_icon(canvas: &mut Canvas, cx: f32, cy: f32, d: &EntityData) {
    if d.directional_light.is_some() {
        canvas.fill_circle(cx, cy, 4.5, color::light_icon());
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            let (s, c) = a.sin_cos();
            canvas.line(
                cx + c * 6.0,
                cy + s * 6.0,
                cx + c * 8.0,
                cy + s * 8.0,
                color::light_icon(),
                1.2,
            );
        }
    } else if d.camera_3d.is_some() {
        canvas.fill_rounded(cx - 6.0, cy - 4.0, 9.0, 8.0, 1.5, color::camera_icon());
        canvas.fill_polygon(
            &[(cx + 3.0, cy), (cx + 7.0, cy - 4.0), (cx + 7.0, cy + 4.0)],
            color::camera_icon(),
        );
    } else if d.mesh_visual.is_some() {
        let c = if d.rigid_body.is_some() {
            color::warn()
        } else {
            color::accent_soft()
        };
        canvas.fill_polygon(
            &[
                (cx, cy - 6.0),
                (cx + 6.0, cy - 3.0),
                (cx, cy),
                (cx - 6.0, cy - 3.0),
            ],
            c,
        );
        canvas.fill_polygon(
            &[
                (cx - 6.0, cy - 3.0),
                (cx, cy),
                (cx, cy + 6.5),
                (cx - 6.0, cy + 3.5),
            ],
            color::accent(),
        );
        canvas.fill_polygon(
            &[
                (cx + 6.0, cy - 3.0),
                (cx, cy),
                (cx, cy + 6.5),
                (cx + 6.0, cy + 3.5),
            ],
            color::select(),
        );
    } else {
        canvas.stroke_circle(cx, cy, 5.0, color::text_dim(), 1.4);
    }
}

/// Icons for entities that have no visible mesh (lights, cameras, empties).
fn draw_markers(canvas: &mut Canvas, doc: &Document, proj: &Projector, rect: Rect) {
    for e in &doc.entities {
        if e.data.mesh_visual.is_some() {
            continue;
        }
        let Some(t) = e.data.transform else {
            continue;
        };
        let Some((x, y)) = proj.project(Vec3::from(t.position)) else {
            continue;
        };
        if !rect.contains(x, y) {
            continue;
        }
        canvas.fill_circle(x, y, 11.0, color::surface());
        entity_icon(canvas, x, y, &e.data);
        canvas.text(
            x + 14.0,
            y - 7.0,
            layout::SMALL_SIZE,
            e.name(),
            color::text_dim(),
        );
    }
}

/// The selection outline (oriented box), plus a light's direction arrow.
fn draw_selection(canvas: &mut Canvas, d: &EntityData, proj: &Projector) {
    if d.mesh_visual.is_some() {
        if let Some(corners) = view::box_corners(d) {
            let screen: Vec<Option<(f32, f32)>> =
                corners.iter().map(|c| proj.project(*c)).collect();
            for i in 0..8 {
                for bit in [1usize, 2, 4] {
                    if i & bit == 0 {
                        if let (Some(a), Some(b)) = (screen[i], screen[i | bit]) {
                            canvas.line(a.0, a.1, b.0, b.1, color::gizmo_hot(), 1.5);
                        }
                    }
                }
            }
        }
    }
    if let (Some(light), Some(t)) = (d.directional_light, d.transform) {
        let from = Vec3::from(t.position);
        let to = from + light.direction.normalize_or_zero() * 2.0;
        if let (Some(a), Some(b)) = (proj.project(from), proj.project(to)) {
            canvas.line(a.0, a.1, b.0, b.1, color::light_icon(), 2.0);
            canvas.fill_circle(b.0, b.1, 3.5, color::light_icon());
        }
    }
}

fn draw_gizmo(canvas: &mut Canvas, g: &Gizmo, hover: Option<usize>) {
    let colors = [color::axis_x(), color::axis_y(), color::axis_z()];
    for (a, base) in colors.into_iter().enumerate() {
        let c = if hover == Some(a) {
            color::gizmo_hot()
        } else {
            base
        };
        match g.mode {
            GizmoMode::Move | GizmoMode::Scale => {
                let Some(tip) = g.tips[a] else {
                    continue;
                };
                canvas.line(g.origin.0, g.origin.1, tip.0, tip.1, c, 2.5);
                let (dx, dy) = (tip.0 - g.origin.0, tip.1 - g.origin.1);
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (ux, uy) = (dx / len, dy / len);
                if g.mode == GizmoMode::Move {
                    let (px, py) = (-uy, ux);
                    canvas.fill_polygon(
                        &[
                            (tip.0 + ux * 12.0, tip.1 + uy * 12.0),
                            (tip.0 + px * 5.5, tip.1 + py * 5.5),
                            (tip.0 - px * 5.5, tip.1 - py * 5.5),
                        ],
                        c,
                    );
                } else {
                    canvas.fill_rect(tip.0 - 5.0, tip.1 - 5.0, 10.0, 10.0, c);
                }
            }
            GizmoMode::Rotate => canvas.polyline(&g.rings[a], c, 2.2),
        }
    }
    canvas.fill_circle(g.origin.0, g.origin.1, 4.0, color::text());
}

/// Evaluates a typed number: plain values or simple arithmetic
/// (`+ - * /` and parentheses), e.g. `2*3+0.5`. Commas act as decimal points.
#[must_use]
pub fn eval_number(text: &str) -> Option<f32> {
    let s: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    if s.is_empty() {
        return None;
    }
    let mut pos = 0;
    let v = expr(&s, &mut pos)?;
    (pos == s.len() && v.is_finite()).then_some(v as f32)
}

fn expr(s: &[char], pos: &mut usize) -> Option<f64> {
    let mut v = term(s, pos)?;
    while *pos < s.len() && (s[*pos] == '+' || s[*pos] == '-') {
        let op = s[*pos];
        *pos += 1;
        let rhs = term(s, pos)?;
        v = if op == '+' { v + rhs } else { v - rhs };
    }
    Some(v)
}

fn term(s: &[char], pos: &mut usize) -> Option<f64> {
    let mut v = factor(s, pos)?;
    while *pos < s.len() && (s[*pos] == '*' || s[*pos] == '/') {
        let op = s[*pos];
        *pos += 1;
        let rhs = factor(s, pos)?;
        v = if op == '*' { v * rhs } else { v / rhs };
    }
    Some(v)
}

fn factor(s: &[char], pos: &mut usize) -> Option<f64> {
    match s.get(*pos)? {
        '-' => {
            *pos += 1;
            factor(s, pos).map(|v| -v)
        }
        '+' => {
            *pos += 1;
            factor(s, pos)
        }
        '(' => {
            *pos += 1;
            let v = expr(s, pos)?;
            if s.get(*pos) == Some(&')') {
                *pos += 1;
                Some(v)
            } else {
                None
            }
        }
        _ => {
            let start = *pos;
            while *pos < s.len() && (s[*pos].is_ascii_digit() || s[*pos] == '.' || s[*pos] == 'e') {
                // Allow a sign right after an exponent marker (1e-3).
                if s[*pos] == 'e' && matches!(s.get(*pos + 1), Some('-' | '+')) {
                    *pos += 1;
                }
                *pos += 1;
            }
            let num: String = s[start..*pos].iter().collect();
            num.parse::<f64>().ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SV_TEST_H: f32 = 150.0;

    fn canvas() -> Canvas {
        Canvas::new(1600, 950)
    }

    fn lay() -> Layout {
        Layout::compute(1600.0, 950.0)
    }

    fn center_of(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    fn click(ui: &mut Ui, c: &mut Canvas, doc: &Document, x: f32, y: f32) -> Vec<Intent> {
        let press = Pointer {
            x,
            y,
            down: true,
            pressed: true,
            ..Pointer::default()
        };
        let mut out = ui.frame(c, doc, press, None);
        let release = Pointer {
            x,
            y,
            ..Pointer::default()
        };
        out.extend(ui.frame(c, doc, release, None));
        out
    }

    #[test]
    fn clicking_the_cube_in_the_scene_selects_it() {
        let mut c = canvas();
        let mut doc = Document::starter();
        doc.select(None); // no gizmo in the way
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        let mut ui = Ui::new();
        let proj = Projector::new(&ui.camera, lay().viewport);
        let (x, y) = proj.project(Vec3::new(0.0, 0.5, 0.0)).unwrap();
        let out = click(&mut ui, &mut c, &doc, x, y);
        assert!(out.contains(&Intent::Select(Some(cube))), "{out:?}");
    }

    #[test]
    fn dragging_in_the_scene_orbits_instead_of_selecting() {
        let mut c = canvas();
        let mut doc = Document::starter();
        doc.select(None); // no gizmo in the way
        let mut ui = Ui::new();
        let (x, y) = center_of(lay().viewport);
        let before = ui.camera;
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                down: true,
                pressed: true,
                ..Pointer::default()
            },
            None,
        );
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 40.0,
                y,
                down: true,
                ..Pointer::default()
            },
            None,
        );
        let out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 40.0,
                y,
                ..Pointer::default()
            },
            None,
        );
        assert_ne!(ui.camera, before);
        assert!(!out.iter().any(|i| matches!(i, Intent::Select(_))));
    }

    #[test]
    fn right_drag_orbits_and_wheel_zooms() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        let (x, y) = center_of(lay().viewport);
        let yaw = ui.camera.yaw;
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                right_down: true,
                ..Pointer::default()
            },
            None,
        );
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 30.0,
                y,
                right_down: true,
                ..Pointer::default()
            },
            None,
        );
        assert!((ui.camera.yaw - yaw).abs() > 1e-4);
        let dist = ui.camera.distance;
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                scroll: 2.0,
                ..Pointer::default()
            },
            None,
        );
        assert!(ui.camera.distance < dist);
    }

    #[test]
    fn dragging_the_move_gizmo_emits_transform_edits() {
        let mut c = canvas();
        let mut doc = Document::starter();
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        doc.select(Some(cube));
        let mut ui = Ui::new();
        let proj = Projector::new(&ui.camera, lay().viewport);
        let t = doc.entities[doc.index_of(cube).unwrap()]
            .data
            .transform
            .unwrap();
        let g = Gizmo::build(GizmoMode::Move, &t, &proj).unwrap();
        let tip = g.tips[1].unwrap();
        let grab = (
            f32::midpoint(g.origin.0, tip.0),
            f32::midpoint(g.origin.1, tip.1),
        );
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: grab.0,
                y: grab.1,
                down: true,
                pressed: true,
                ..Pointer::default()
            },
            None,
        );
        let out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: grab.0,
                y: grab.1 - 30.0,
                down: true,
                ..Pointer::default()
            },
            None,
        );
        let moved = out.iter().find_map(|i| match i {
            Intent::SetTransform { id, transform } if *id == cube => Some(*transform),
            _ => None,
        });
        let moved = moved.expect("gizmo drag edits the transform");
        assert!(
            moved.position[1] > t.position[1],
            "dragging up the Y handle raises it"
        );
        let out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: grab.0,
                y: grab.1 - 30.0,
                ..Pointer::default()
            },
            None,
        );
        assert!(out.contains(&Intent::EndGesture));
    }

    #[test]
    fn clicking_a_number_field_then_typing_sets_it() {
        let mut ui = Ui::new();
        ui.edit = Some(TextEdit {
            id: 7,
            target: EditTarget::Field(Field::Position(0)),
            buffer: "0".into(),
            rect: Rect::default(),
            fresh: true,
            attr: None,
        });
        ui.type_text("2*3");
        let intent = ui.edit_key(EditKey::Enter);
        assert_eq!(
            intent,
            Some(Intent::SetField {
                id: 7,
                field: Field::Position(0),
                value: 6.0,
                gesture: false
            })
        );
        assert!(!ui.is_editing_text());
    }

    #[test]
    fn scrubbing_a_field_emits_gesture_edits() {
        let mut c = canvas();
        let mut doc = Document::starter();
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        doc.select(Some(cube));
        let mut ui = Ui::new();
        // Find the Position X field by scanning the inspector's first rows.
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let insp = lay().inspector;
        let x = insp.x + layout::PAD + layout::LABEL_W + 10.0;
        // Header (30) + id (22) + section (30) from the inspector's top.
        let y = insp.y + layout::TITLEBAR_H + 10.0 + 30.0 + 22.0 + 30.0 + 8.0;
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                down: true,
                pressed: true,
                ..Pointer::default()
            },
            None,
        );
        let out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 50.0,
                y,
                down: true,
                ..Pointer::default()
            },
            None,
        );
        assert!(
            out.iter().any(|i| matches!(
                i,
                Intent::SetField {
                    field: Field::Position(0),
                    gesture: true,
                    ..
                }
            )),
            "{out:?}"
        );
        let out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 50.0,
                y,
                ..Pointer::default()
            },
            None,
        );
        assert!(out.contains(&Intent::EndGesture));
    }

    #[test]
    fn create_menu_creates_entities() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        let hier = lay().hierarchy;
        let plus = (hier.x + hier.w - 46.0, hier.y + 14.0);
        click(&mut ui, &mut c, &doc, plus.0, plus.1);
        assert!(ui.menu.is_some(), "+ Create opens the menu");
        let r = ui.menu.as_ref().unwrap().rect(&mut c, 1600.0, 950.0);
        // Second item: Cube.
        let item = (r.x + 30.0, r.y + 4.0 + layout::ROW_H * 1.5);
        let out = click(&mut ui, &mut c, &doc, item.0, item.1);
        assert!(out.contains(&Intent::Create(CreateKind::Cube)), "{out:?}");
        assert!(ui.menu.is_none());
    }

    fn select_named(doc: &mut Document, name: &str) -> u64 {
        let id = doc.entities.iter().find(|e| e.name() == name).unwrap().id;
        doc.select(Some(id));
        id
    }

    fn right_click(ui: &mut Ui, c: &mut Canvas, doc: &Document, x: f32, y: f32) -> Vec<Intent> {
        let mut out = ui.frame(
            c,
            doc,
            Pointer {
                x,
                y,
                right_down: true,
                right_pressed: true,
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

    #[test]
    fn menu_bar_opens_menus_and_runs_items() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        // "GameObject" is the third bar entry; find it by hovering along.
        let mut opened = false;
        for x in (100..400).step_by(6) {
            click(&mut ui, &mut c, &doc, x as f32, 13.0);
            if ui.menu.as_ref().is_some_and(|m| m.bar == Some(2)) {
                opened = true;
                break;
            }
            ui.menu = None;
        }
        assert!(opened, "GameObject menu opens from the bar");
        let r = ui.menu.as_ref().unwrap().rect(&mut c, 1600.0, 950.0);
        let out = click(
            &mut ui,
            &mut c,
            &doc,
            r.x + 30.0,
            r.y + 4.0 + layout::ROW_H * 1.5,
        );
        assert!(out.contains(&Intent::Create(CreateKind::Cube)), "{out:?}");
    }

    #[test]
    fn right_clicking_a_hierarchy_row_opens_the_entity_menu() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        let hier = lay().hierarchy;
        let row_y = hier.y + layout::TITLEBAR_H + 4.0 + layout::ROW_H * 0.5;
        let out = right_click(&mut ui, &mut c, &doc, hier.x + 60.0, row_y);
        assert!(
            matches!(out.first(), Some(Intent::Select(Some(_)))),
            "{out:?}"
        );
        let menu = ui.menu.as_ref().expect("context menu");
        assert!(menu.items.iter().any(|i| i.label == "Save as Prefab…"));
        assert!(menu.items.iter().any(|i| i.label == "Duplicate"));
    }

    #[test]
    fn the_color_swatch_opens_a_picker_that_edits_the_color() {
        let mut c = canvas();
        let mut doc = Document::starter();
        let id = select_named(&mut doc, "Cube");
        let mut ui = Ui::new();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        ui.raw = Pointer {
            x: 900.0,
            y: 300.0,
            ..Pointer::default()
        };
        ui.run_action(menus::UiAction::PickColor(id, ColorTarget::Mesh), &doc);
        assert!(ui.picker.is_some(), "the picker opens");
        let r = ui.picker_rect(1600.0, 950.0).unwrap();
        // Drag in the SV square: bottom-left is black.
        let (x, y) = (r.x + 14.0, r.y + 12.0 + SV_TEST_H - 1.0);
        let mut out = ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                down: true,
                pressed: true,
                ..Pointer::default()
            },
            None,
        );
        out.extend(ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                down: true,
                ..Pointer::default()
            },
            None,
        ));
        out.extend(ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                ..Pointer::default()
            },
            None,
        ));
        let set = out.iter().find_map(|i| match i {
            Intent::SetColor {
                id: got,
                rgba,
                gesture: true,
                ..
            } if *got == id => Some(*rgba),
            _ => None,
        });
        let rgba = set.unwrap_or_else(|| panic!("no SetColor in {out:?}"));
        assert!(
            rgba[0] < 0.05 && rgba[1] < 0.05 && rgba[2] < 0.05,
            "{rgba:?}"
        );
        assert!(out.contains(&Intent::EndGesture));
    }

    #[test]
    fn dragging_a_splitter_resizes_the_panel() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        let hier = lay().hierarchy;
        let x = hier.x + hier.w;
        let y = hier.y + 200.0;
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x,
                y,
                down: true,
                pressed: true,
                ..Pointer::default()
            },
            None,
        );
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 80.0,
                y,
                down: true,
                ..Pointer::default()
            },
            None,
        );
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: x + 80.0,
                y,
                ..Pointer::default()
            },
            None,
        );
        assert!(
            (ui.panels.hierarchy_w - (hier.w + 80.0)).abs() < 1.0,
            "{}",
            ui.panels.hierarchy_w
        );
    }

    #[test]
    fn the_save_as_dialog_submits_a_path_with_the_extension() {
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root = std::env::temp_dir();
        ui.open_dialog_kind(DialogKind::SaveSceneAs, &doc);
        assert!(ui.dialog_open() && ui.is_editing_text());
        ui.edit_key(EditKey::Delete);
        ui.type_text("oberon_dialog_test_level");
        let intent = ui.edit_key(EditKey::Enter);
        assert_eq!(
            intent,
            Some(Intent::SaveAs(
                std::env::temp_dir().join("oberon_dialog_test_level.ron")
            ))
        );
        assert!(!ui.dialog_open());
    }

    #[test]
    fn console_links_find_python_locations() {
        assert_eq!(
            console_location("  File \"scripts/player.py\", line 12, in update"),
            Some((PathBuf::from("scripts/player.py"), 12))
        );
        assert_eq!(
            console_location("error: player.py:7: bad"),
            Some((PathBuf::from("player.py"), 7))
        );
        assert_eq!(console_location("plain message"), None);
    }

    #[test]
    fn play_mode_blocks_document_edits() {
        assert!(Intent::Delete(1).edits_document());
        assert!(!Intent::Select(Some(1)).edits_document());
        assert!(!Intent::Stop.edits_document());
    }

    #[test]
    fn number_expressions_evaluate() {
        assert_eq!(eval_number("1.5"), Some(1.5));
        assert_eq!(eval_number("2*(3+1)"), Some(8.0));
        assert_eq!(eval_number("-0,5"), Some(-0.5));
        assert_eq!(eval_number("1e-3"), Some(0.001));
        assert_eq!(eval_number("abc"), None);
        assert_eq!(eval_number(""), None);
    }

    #[test]
    fn hidpi_layout_uses_logical_pixels() {
        let mut c = Canvas::new(3200, 1900);
        c.set_scale(2.0);
        assert_eq!(c.logical_size(), (1600.0, 950.0));
        let doc = Document::starter();
        let mut ui = Ui::new();
        // Renders without panicking at 2x.
        ui.frame(&mut c, &doc, Pointer::default(), None);
    }
}
