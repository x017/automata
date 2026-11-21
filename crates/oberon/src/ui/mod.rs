//! The Oberon editor UI: an immediate-mode, Unity-style interface rendered
//! into a [`Canvas`] each frame.
//!
//! Layout: toolbar (file / undo / transform tools / play controls) on top,
//! **Hierarchy** left with the **Files** tree under it, **Scene view** center
//! with the **Console** under it,
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
mod code_view;
pub mod dialogs;
pub mod graphics;
pub mod menus;
pub mod palette;
pub mod picker;
pub mod project_panel;
pub mod settings;

use dialogs::Dialog;
pub use graphics::{FrameStats, RightTab};
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
    /// Whether the middle button was pressed since the last frame.
    pub middle_pressed: bool,
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
    /// Left sidebar width (Hierarchy and Files).
    pub hierarchy_w: f32,
    /// Inspector width.
    pub inspector_w: f32,
    /// Bottom dock height.
    pub bottom_h: f32,
    /// Height of the Files panel at the bottom of the left sidebar.
    pub files_h: f32,
    /// Hierarchy visible.
    pub show_hierarchy: bool,
    /// Files panel visible.
    pub show_files: bool,
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
            files_h: layout::FILES_H,
            show_hierarchy: true,
            show_files: true,
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
}

/// The editor's panel rectangles for a window of a given logical size.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Menu bar.
    pub menubar: Rect,
    /// Tool bar (under the menu bar).
    pub toolbar: Rect,
    /// Left sidebar, top: the entity hierarchy.
    pub hierarchy: Rect,
    /// Left sidebar, bottom: the project file tree.
    pub files: Rect,
    /// Right inspector panel.
    pub inspector: Rect,
    /// Center scene view (or code editor).
    pub viewport: Rect,
    /// Bottom dock (Console) under the center area.
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
        let hier_w = if panels.show_hierarchy || panels.show_files {
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
        // The left sidebar stacks Hierarchy over Files; either fills it alone.
        let files_h = match (panels.show_hierarchy, panels.show_files) {
            (true, true) => clamp_files_h(panels.files_h, body_h),
            (false, true) => body_h,
            _ => 0.0,
        };
        let hier_h = if panels.show_hierarchy {
            body_h - files_h
        } else {
            0.0
        };
        Self {
            menubar: Rect::new(0.0, 0.0, w, MENUBAR_H),
            toolbar: Rect::new(0.0, MENUBAR_H, w, layout::TOOLBAR_H),
            hierarchy: Rect::new(0.0, body_y, hier_w, hier_h),
            files: Rect::new(0.0, body_y + hier_h, hier_w, files_h),
            inspector: Rect::new(w - insp_w, body_y, insp_w, body_h),
            viewport: Rect::new(hier_w, body_y, center_w, view_h),
            console: Rect::new(hier_w, body_y + view_h, center_w, console_h),
            status: Rect::new(0.0, h - layout::STATUS_H, w, layout::STATUS_H),
        }
    }
}

/// The Files panel height for a left sidebar `body_h` tall: at least 90 px,
/// leaving the Hierarchy at least 120 px.
fn clamp_files_h(files_h: f32, body_h: f32) -> f32 {
    files_h.clamp(
        90.0_f32.min(body_h * 0.5),
        (body_h - 120.0).max(90.0_f32.min(body_h * 0.5)),
    )
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
    /// Restart the language server (its settings changed).
    RestartLanguageServer,
    /// Replace an entity's whole Environment (a preset).
    SetEnvironment(u64, automata_renderer::Environment),
    /// Add a graphics test arrangement.
    SpawnTest(crate::doc::TestScene),
    /// Remove every `Test …` entity.
    ClearTests,
    /// Move a camera entity to where the editor view is.
    AlignCameraToView(u64),
    /// Export the game: one platform, or (`None`) the platforms ticked in
    /// Build Settings.
    Build(Option<crate::build::Target>),
    /// Download and install the export template for a platform (`None` =
    /// every platform).
    DownloadTemplates(Option<crate::build::Target>),
    /// Open the project's build folder in the system file manager.
    OpenBuildFolder,
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
                | Self::RestartLanguageServer
                | Self::Build(_)
                | Self::DownloadTemplates(_)
                | Self::OpenBuildFolder
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
    /// A Settings text field.
    Setting(settings::SettingText),
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
    Files,
    Inspector,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonStyle {
    /// Secondary: a quiet fill, no outline.
    Normal,
    /// A toggled-on option.
    Active,
    /// The primary action: paper-white fill, black label.
    Accent,
    /// Unavailable.
    Disabled,
    /// Tertiary: no fill until hovered.
    Ghost,
    /// A destructive tertiary action: red label, no fill until hovered.
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    Play,
    Pause,
    Stop,
    Close,
    Gear,
    Search,
    Plus,
}

/// The editor UI state.
pub struct Ui {
    /// The scene-view camera.
    pub camera: ViewCamera,
    /// The active transform tool.
    pub gizmo: GizmoMode,
    /// Edit / play state (set by the app).
    pub play: PlayState,
    /// The running export / template download, for the status bar (set by
    /// the app).
    pub build_status: Option<String>,
    /// The last finished build: success and summary (set by the app).
    pub build_last: Option<(bool, String)>,
    /// Build Settings being edited: the project folder and its `build.ron`.
    build_cfg: Option<(PathBuf, crate::build::BuildConfig)>,
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
    /// The project directory (file dialogs and the Files panel), set by
    /// the app.
    pub project_root: PathBuf,
    /// Panel sizes and visibility.
    pub panels: PanelSizes,
    /// Active bottom-dock tab.
    pub bottom_tab: BottomTab,
    /// Active right-dock tab.
    pub right_tab: RightTab,
    /// Frame statistics for the Graphics panel (set by the app).
    pub stats: FrameStats,
    /// The selected camera's view, rendered by the app: RGBA, size.
    camera_preview: Option<(Vec<u8>, u32, u32)>,
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
    /// Files panel state.
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
    /// The command palette, while open.
    palette: Option<palette::Palette>,
    /// The Settings page shown last.
    settings_page: settings::SettingsPage,
    /// A tooltip requested this frame: anchor and text.
    tip_request: Option<(Rect, String)>,
    /// The tooltip being hovered and since when.
    tip_since: Option<(String, std::time::Instant)>,
    /// Editor preferences (owned here so Settings can edit them live).
    pub prefs: crate::project::Prefs,
    /// Set when Settings changed `prefs` (the app saves and applies them).
    prefs_dirty: bool,
    /// The built-in script editor.
    pub code: crate::code::CodeEditor,
    /// Editor requests waiting for the app (language server, …).
    code_requests: Vec<crate::code::CodeEvent>,
    /// Drag-selecting text (`true` = whole lines from the gutter).
    code_drag: Option<bool>,
    /// The completion popup's rectangle this frame.
    code_popup: Option<Rect>,
    /// Last click time, click count and line (double / triple click).
    code_clicks: Option<(std::time::Instant, u8, usize)>,
    /// Mouse rest for hover info: since, word start, request sent.
    code_dwell: Option<(std::time::Instant, crate::code::Pos, bool)>,
    /// How far the script tabs are scrolled (pixels).
    tab_scroll: f32,
    /// The active tab the strip last scrolled into view (`None` = Scene;
    /// starts at an impossible index so the first frame reveals).
    tab_revealed: Option<usize>,
    /// Dragging a script tab: its index and the press x.
    tab_drag: Option<(usize, f32)>,
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
            build_status: None,
            build_last: None,
            build_cfg: None,
            panels: PanelSizes::default(),
            bottom_tab: BottomTab::Console,
            right_tab: RightTab::Inspector,
            stats: FrameStats::default(),
            camera_preview: None,
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
            palette: None,
            settings_page: settings::SettingsPage::default(),
            tip_request: None,
            tip_since: None,
            prefs: crate::project::Prefs::default(),
            prefs_dirty: false,
            code: crate::code::CodeEditor::new(),
            code_requests: Vec::new(),
            code_drag: None,
            code_popup: None,
            code_clicks: None,
            tab_scroll: 0.0,
            tab_revealed: Some(usize::MAX),
            tab_drag: None,
            code_dwell: None,
        }
    }

    // ----- public API used by the app -------------------------------------

    /// Shows the Console tab in the bottom dock.
    pub fn show_console(&mut self) {
        self.panels.show_bottom = true;
        self.bottom_tab = BottomTab::Console;
    }

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

    /// The camera the scene view wants a preview of, with the preview's
    /// physical pixel size, or `None` (no camera selected, preview off, or
    /// the scene view hidden). `scale` is the canvas scale; `(w, h)` the
    /// window's logical size.
    #[must_use]
    pub fn camera_preview_request(
        &self,
        doc: &Document,
        w: f32,
        h: f32,
        scale: f32,
    ) -> Option<(automata_renderer::Camera3D, u32, u32)> {
        if !self.prefs.camera_preview || !self.scene_visible() || self.play != PlayState::Editing {
            return None;
        }
        let cam = doc
            .selected_index()
            .and_then(|i| doc.entities[i].data.camera_3d)?;
        let view = self.layout(w, h).viewport;
        let (pw, ph) = graphics::preview_size(view);
        if view.w < 260.0 || view.h < 200.0 {
            return None;
        }
        let px = |v: f32| (v * scale).round().max(1.0) as u32;
        Some((cam, px(pw), px(ph)))
    }

    /// Hands the UI the rendered camera preview (`None` clears it).
    pub fn set_camera_preview(&mut self, preview: Option<(&[u8], u32, u32)>) {
        match preview {
            Some((rgba, w, h)) => {
                let slot = self
                    .camera_preview
                    .get_or_insert_with(|| (Vec::new(), 0, 0));
                slot.0.clear();
                slot.0.extend_from_slice(rgba);
                slot.1 = w;
                slot.2 = h;
            }
            None => self.camera_preview = None,
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
            EditTarget::Setting(which) => self.commit_setting(which, &edit.buffer),
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
        let mut lay = Layout::with_panels(w, h, &self.panels);
        if !self.code.tabs.is_empty() {
            // Document tabs (Scene / scripts) sit on top of the center area.
            let tabs = code_view::DOC_TABS_H.min(lay.viewport.h);
            lay.viewport.y += tabs;
            lay.viewport.h -= tabs;
        }
        lay
    }

    /// Whether a popup, picker, or dialog is open (Escape closes it).
    #[must_use]
    pub fn overlay_open(&self) -> bool {
        self.menu.is_some()
            || self.picker.is_some()
            || self.dialog.is_some()
            || self.palette.is_some()
    }

    /// Closes the topmost popup / picker / dialog. Returns false if none.
    pub fn close_overlay(&mut self) -> bool {
        if self.dialog.is_some() {
            self.close_dialog();
        } else if self.palette.is_some() {
            self.palette = None;
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

    /// Whether Settings changed the preferences since the last call.
    pub fn take_prefs_dirty(&mut self) -> bool {
        std::mem::take(&mut self.prefs_dirty)
    }

    /// Opens a script in the code editor (optionally at a 1-based line).
    pub fn open_code(&mut self, path: &std::path::Path, line: Option<u32>) {
        if let Err(e) = self.code.open(path, line) {
            self.log(LogLevel::Error, e);
        }
        self.drain_code_events();
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
            if self.palette.is_some() {
                let over = self
                    .palette_rect(w)
                    .is_some_and(|r| r.contains(pointer.x, pointer.y));
                if pointer.pressed && !over {
                    self.palette = None;
                }
                self.mask_pointer();
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

        self.code.settings.clone_from(&self.prefs.editor);
        if self.code.active.is_some() {
            self.code.scene_names = scene_names(doc);
        }
        self.update_scrub();
        self.update_attr_scrub();
        self.update_splitters(&lay, w, h);

        canvas.clear(color::background());
        // Scene view first: its overlays may spill outside its rect and are
        // covered by the panels drawn after it.
        if self.code.active.is_some() {
            self.code_view(canvas, lay.viewport);
        } else {
            self.scene_view(canvas, doc, lay.viewport, viewport);
        }
        if let Some(r) = self.doc_tabs_rect(lay.viewport) {
            self.doc_tabs(canvas, r);
        }
        if self.panels.show_bottom {
            self.bottom_dock(canvas, lay.console);
        }
        if self.panels.show_hierarchy {
            self.hierarchy(canvas, doc, lay.hierarchy);
        }
        if self.panels.show_files {
            self.files_panel(canvas, doc, lay.files);
        }
        if self.panels.show_inspector {
            self.inspector(canvas, doc, lay.inspector);
        }
        self.status_bar(canvas, doc, lay.status);
        self.clip = (0.0, h);
        self.draw_splitters(canvas, &lay);
        self.menu_bar(canvas, doc, w);
        self.toolbar(canvas, doc, lay.menubar);
        self.draw_picker(canvas, doc, w, h);
        self.draw_menu(canvas, doc, w, h);
        self.draw_palette(canvas, doc, w);
        self.draw_switcher(canvas, w);
        self.draw_dialog(canvas, w, h);
        self.draw_tooltip(canvas, w, h);

        std::mem::take(&mut self.out)
    }

    /// Hides the pointer from ordinary widgets (an overlay owns it).
    fn mask_pointer(&mut self) {
        self.p.pressed = false;
        self.p.right_pressed = false;
        self.p.middle_pressed = false;
        self.p.double_click = false;
        self.p.scroll = 0.0;
        self.p.x = -1.0e6;
        self.p.y = -1.0e6;
    }

    /// Splitter hit areas for the current layout.
    fn splitter_rects(&self, lay: &Layout) -> Vec<(Splitter, Rect)> {
        let mut v = Vec::new();
        if self.panels.show_hierarchy || self.panels.show_files {
            let r = lay.hierarchy;
            let h = lay.hierarchy.h + lay.files.h;
            v.push((Splitter::Hierarchy, Rect::new(r.x + r.w - 3.0, r.y, 6.0, h)));
        }
        if self.panels.show_hierarchy && self.panels.show_files {
            let r = lay.files;
            v.push((Splitter::Files, Rect::new(r.x, r.y - 3.0, r.w - 3.0, 6.0)));
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
                    Splitter::Files => {
                        let body_h = lay.hierarchy.h + lay.files.h;
                        self.panels.files_h = clamp_files_h(lay.status.y - p.y, body_h);
                    }
                }
                self.cursor = match which {
                    Splitter::Bottom | Splitter::Files => CursorHint::ResizeV,
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
                    Splitter::Bottom | Splitter::Files => CursorHint::ResizeV,
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
        let clear = Color::TRANSPARENT;
        let (bg, fg) = match style {
            ButtonStyle::Active => (color::select(), color::text()),
            ButtonStyle::Accent => (
                if hot {
                    color::accent_soft()
                } else {
                    color::accent()
                },
                color::on_accent(),
            ),
            ButtonStyle::Disabled => (clear, color::text_faint()),
            ButtonStyle::Normal => (
                if hot { color::hover() } else { color::raised() },
                color::text(),
            ),
            ButtonStyle::Ghost => (if hot { color::hover() } else { clear }, color::text()),
            ButtonStyle::Danger => (if hot { color::hover() } else { clear }, color::danger()),
        };
        if bg.alpha() > 0.0 {
            canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, bg);
        }
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
            ButtonStyle::Disabled => color::text_faint(),
            ButtonStyle::Accent => color::on_accent(),
            ButtonStyle::Danger => color::danger(),
            ButtonStyle::Ghost => {
                if self.hot(r) {
                    color::text()
                } else {
                    color::text_dim()
                }
            }
            _ => color::text(),
        };
        draw_icon(canvas, icon, r.x + r.w / 2.0, r.y + r.h / 2.0, c);
        clicked
    }

    // ----- toolbar -------------------------------------------------------

    /// The right half of the top bar: play controls (centred), the play
    /// state, Search and Settings. The menus own the left half.
    #[allow(clippy::too_many_lines)]
    fn toolbar(&mut self, canvas: &mut Canvas, doc: &Document, rect: Rect) {
        let bh = 28.0;
        let by = rect.y + (rect.h - bh) / 2.0;

        // Play controls, centred on the window.
        let bw = 34.0;
        let group = bw * 3.0 + 4.0;
        let px = rect.w / 2.0 - group / 2.0;
        let editing = self.play == PlayState::Editing;
        canvas.fill_rounded(px, by, group, bh, layout::RADIUS + 1.0, color::raised());
        let play_r = Rect::new(px + 2.0, by + 2.0, bw - 2.0, bh - 4.0);
        let play_style = if editing {
            ButtonStyle::Ghost
        } else {
            ButtonStyle::Accent
        };
        if self.icon_button(canvas, play_r, Icon::Play, play_style) {
            self.out.push(Intent::Play);
        }
        self.tip(
            play_r,
            if editing {
                "Play  (Ctrl+P)"
            } else {
                "Stop playing  (Ctrl+P)"
            },
        );
        let pause_style = match self.play {
            PlayState::Editing => ButtonStyle::Disabled,
            PlayState::Paused => ButtonStyle::Active,
            PlayState::Playing => ButtonStyle::Ghost,
        };
        let pause_r = Rect::new(px + bw + 2.0, by + 2.0, bw - 2.0, bh - 4.0);
        if self.icon_button(canvas, pause_r, Icon::Pause, pause_style) {
            self.out.push(Intent::Pause);
        }
        self.tip(pause_r, "Pause");
        let stop_style = if editing {
            ButtonStyle::Disabled
        } else {
            ButtonStyle::Ghost
        };
        let stop_r = Rect::new(px + 2.0 * bw + 2.0, by + 2.0, bw - 2.0, bh - 4.0);
        if self.icon_button(canvas, stop_r, Icon::Stop, stop_style) {
            self.out.push(Intent::Stop);
        }
        self.tip(stop_r, "Stop and restore the scene");
        // While playing, a plain-text state next to the controls.
        let state = match self.play {
            PlayState::Editing => None,
            PlayState::Playing => Some(("Playing", color::success())),
            PlayState::Paused => Some(("Paused", color::warn())),
        };
        if let Some((text, c)) = state {
            let sx = px + group + 14.0;
            canvas.fill_circle(sx + 3.0, by + bh / 2.0, 3.0, c);
            canvas.text(
                sx + 12.0,
                by + (bh - layout::FONT_SIZE) / 2.0 - 1.0,
                layout::FONT_SIZE,
                text,
                c,
            );
        }

        // Right: Search, then Settings.
        let gear = Rect::new(rect.w - 8.0 - bh, by, bh, bh);
        if self.icon_button(canvas, gear, Icon::Gear, ButtonStyle::Ghost) {
            self.open_settings();
        }
        self.tip(gear, "Settings  (Ctrl+,)");
        let search_w = 220.0_f32;
        let search = Rect::new(gear.x - 6.0 - search_w, by, search_w, bh);
        if search.x > px + group + 110.0 {
            let hot = self.hot(search);
            let bg = if hot { color::hover() } else { color::raised() };
            canvas.fill_rounded(search.x, search.y, search.w, search.h, layout::RADIUS, bg);
            draw_icon(
                canvas,
                Icon::Search,
                search.x + 15.0,
                search.y + bh / 2.0,
                color::text_dim(),
            );
            canvas.text(
                search.x + 30.0,
                search.y + (bh - layout::FONT_SIZE) / 2.0 - 1.0,
                layout::FONT_SIZE,
                "Search",
                color::text_dim(),
            );
            let k = "Ctrl Shift P";
            let kw = canvas.measure_text(layout::SMALL_SIZE, k);
            canvas.text(
                search.x + search.w - kw - 10.0,
                search.y + (bh - layout::SMALL_SIZE) / 2.0 - 1.0,
                layout::SMALL_SIZE,
                k,
                color::text_faint(),
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    self.open_palette(doc);
                }
            }
            self.tip(search, "Commands, entities, scripts and settings");
        }
    }

    /// The Move / Rotate / Scale switch floating in the scene view's
    /// top-left corner.
    fn tool_group_rect(canvas: &mut Canvas, view: Rect) -> Rect {
        let w: f32 = GizmoMode::ALL
            .iter()
            .map(|m| canvas.measure_text(layout::FONT_SIZE, m.label()) + 24.0)
            .sum();
        Rect::new(view.x + 10.0, view.y + 10.0, w + 4.0, 30.0)
    }

    fn tool_group(&mut self, canvas: &mut Canvas, view: Rect) {
        let g = Self::tool_group_rect(canvas, view);
        canvas.fill_rounded(g.x, g.y, g.w, g.h, layout::RADIUS + 1.0, color::surface());
        let mut x = g.x + 2.0;
        for (mode, tip) in [
            (GizmoMode::Move, "Move  (W)"),
            (GizmoMode::Rotate, "Rotate  (E)"),
            (GizmoMode::Scale, "Scale  (R)"),
        ] {
            let bw = canvas.measure_text(layout::FONT_SIZE, mode.label()) + 24.0;
            let r = Rect::new(x, g.y + 2.0, bw, g.h - 4.0);
            let style = if self.gizmo == mode {
                ButtonStyle::Active
            } else {
                ButtonStyle::Ghost
            };
            if self.gizmo != mode && self.hot(r) {
                self.cursor = CursorHint::Pointer;
            }
            let fg = if self.gizmo == mode {
                color::text()
            } else {
                color::text_dim()
            };
            if style == ButtonStyle::Active || self.hot(r) {
                let bg = if style == ButtonStyle::Active {
                    color::select()
                } else {
                    color::hover()
                };
                canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, bg);
            }
            canvas.text(
                r.x + 12.0,
                r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
                layout::FONT_SIZE,
                mode.label(),
                fg,
            );
            if self.hot(r) && self.p.pressed {
                self.gizmo = mode;
            }
            self.tip(r, tip);
            x += bw;
        }
    }

    /// Requests a tooltip for `r` while the pointer rests on it.
    fn tip(&mut self, r: Rect, text: &str) {
        if self.hot(r) && !self.p.down {
            self.tip_request = Some((r, text.to_string()));
        }
    }

    /// Draws the tooltip requested this frame once the pointer has rested
    /// on its widget for a moment.
    fn draw_tooltip(&mut self, canvas: &mut Canvas, w: f32, h: f32) {
        let Some((r, text)) = self.tip_request.take() else {
            self.tip_since = None;
            return;
        };
        let now = std::time::Instant::now();
        let since = match &self.tip_since {
            Some((t, at)) if *t == text => *at,
            _ => {
                self.tip_since = Some((text.clone(), now));
                now
            }
        };
        if (now - since).as_secs_f32() < 0.45 {
            return;
        }
        let tw = canvas.measure_text(layout::SMALL_SIZE, &text) + 20.0;
        let th = 24.0;
        let x = (r.x + r.w / 2.0 - tw / 2.0).clamp(4.0, (w - tw - 4.0).max(4.0));
        let mut y = r.y + r.h + 6.0;
        if y + th > h - 4.0 {
            y = r.y - th - 6.0;
        }
        canvas.fill_rounded(x + 1.0, y + 3.0, tw, th, 5.0, color::shadow());
        canvas.fill_rounded(x, y, tw, th, 5.0, color::raised());
        canvas.stroke_rounded(x, y, tw, th, 5.0, color::border(), 1.0);
        canvas.text(x + 10.0, y + 5.0, layout::SMALL_SIZE, &text, color::text());
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
            rect.x + rect.w - layout::TITLEBAR_H,
            rect.y + 4.0,
            layout::TITLEBAR_H - 8.0,
            layout::TITLEBAR_H - 8.0,
        );
        let style = if self.play == PlayState::Editing {
            ButtonStyle::Ghost
        } else {
            ButtonStyle::Disabled
        };
        self.tip(add, "Create an entity");
        if self.icon_button(canvas, add, Icon::Plus, style) {
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
            _ if self.right_tab == RightTab::Graphics => {
                self.graphics_body(canvas, doc, x, start, w)
            }
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
        self.right_tabs(canvas, rect);
        if self.play != PlayState::Editing {
            canvas.text(
                rect.x + rect.w - 118.0,
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
                y = self.float_row(canvas, x, y, w, e, "Azimuth", Field::LightAzimuth);
                y = self.float_row(canvas, x, y, w, e, "Elevation", Field::LightElevation);
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
                y = self.camera_tools(canvas, x, y + 4.0, w, e);
            }
        }
        if d.environment.is_some() {
            let kind = ComponentKind::Environment;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.color_row(canvas, x, y, w, e, "Sky", ColorTarget::EnvSky);
                y = self.color_row(canvas, x, y, w, e, "Ground", ColorTarget::EnvGround);
                y = self.color_row(canvas, x, y, w, e, "Background", ColorTarget::EnvBackground);
                y = self.choice_row(canvas, x, y, w, e, "Tone Mapping", Choice::Tonemapping);
                y = self.float_row(canvas, x, y, w, e, "Exposure", Field::EnvExposure);
                y = self.flag_row(canvas, x, y, w, e, "Fog", Flag::EnvFog);
                y = self.color_row(canvas, x, y, w, e, "Fog Color", ColorTarget::EnvFog);
                y = self.float_row(canvas, x, y, w, e, "Fog Density", Field::EnvFogDensity);
                y = self.float_row(canvas, x, y, w, e, "Fog Start", Field::EnvFogStart);
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
        let add = Rect::new(x, y, w, 28.0);
        if self.row_visible(add) {
            let style = if self.play == PlayState::Editing {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, add, "Add Component", style) {
                let items = self.bar_items(3, doc);
                self.open_menu(add.x, add.y + add.h + 2.0, items);
            }
        }
        y += 34.0;
        let del = Rect::new(x, y, w, 28.0);
        if self.row_visible(del) {
            let style = if self.play == PlayState::Editing {
                ButtonStyle::Danger
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
            canvas.fill_rect(header.x, header.y - 4.0, header.w, 1.0, color::border());
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
            canvas.text_in(
                crate::canvas::Face::Medium,
                x + 16.0,
                y + 5.0,
                layout::FONT_SIZE,
                title,
                color::text(),
            );

            let editing = self.play == PlayState::Editing;
            let mut right = x + w;
            if let Some(kind) = removable {
                let r = Rect::new(right - 22.0, y + 3.0, 22.0, 18.0);
                let style = if editing {
                    ButtonStyle::Ghost
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

    /// A color field: a swatch and its hex code; opens the color picker.
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
        let bg = if hot { color::hover() } else { color::field() };
        canvas.fill_rounded(sw.x, sw.y, sw.w, sw.h, 3.0, bg);
        let chip = Rect::new(sw.x + 4.0, sw.y + 4.0, sw.h - 8.0, sw.h - 8.0);
        canvas.fill_rounded(chip.x, chip.y, chip.w, chip.h, 2.0, opaque);
        canvas.stroke_rounded(chip.x, chip.y, chip.w, chip.h, 2.0, color::border(), 1.0);
        let hex = picker::to_hex(rgba, false);
        let tx = chip.x + chip.w + 8.0;
        canvas.text_clipped(
            tx,
            sw.y + (sw.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            &hex,
            color::text(),
            sw.x + sw.w - tx - 4.0,
        );
        if target.has_alpha() && rgba[3] < 0.995 {
            let a = format!("{:.0}%", c(rgba[3]) * 100.0);
            let aw = canvas.measure_text(layout::SMALL_SIZE, &a);
            canvas.text(
                sw.x + sw.w - aw - 8.0,
                sw.y + (sw.h - layout::SMALL_SIZE) / 2.0 - 1.0,
                layout::SMALL_SIZE,
                &a,
                color::text_dim(),
            );
        }
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
            if !on {
                canvas.stroke_rounded(b.x, b.y, b.w, b.h, 3.0, color::border(), 1.0);
            }
            if on {
                canvas.polyline(
                    &[
                        (b.x + 4.0, b.y + 9.5),
                        (b.x + 7.5, b.y + 13.0),
                        (b.x + 14.0, b.y + 5.0),
                    ],
                    color::on_accent(),
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
        if editing {
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.0);
        }
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
            tx = draw_axis_tag(canvas, r, c);
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
        if editing {
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.0);
        }
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
        let editing = self.play == PlayState::Editing;
        // The floating tool switch owns its corner of the view.
        let over_tools = editing && Self::tool_group_rect(canvas, rect).contains(p.x, p.y);
        let inside = rect.contains(p.x, p.y) && !over_tools;
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
        draw_camera_frustums(canvas, doc, &proj, selected.map(|e| e.id));
        if let Some(e) = selected {
            draw_selection(canvas, &e.data, &proj);
        }
        if let Some(g) = &gizmo {
            draw_gizmo(canvas, g, self.hover_axis);
        }
        self.draw_axes_widget(canvas, rect);

        if editing {
            self.tool_group(canvas, rect);
            self.draw_camera_preview(canvas, selected, rect);
        }
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

    /// The selected camera's view as an inset in the scene view's corner.
    fn draw_camera_preview(
        &mut self,
        canvas: &mut Canvas,
        selected: Option<&EntityEntry>,
        rect: Rect,
    ) {
        let Some(e) = selected.filter(|e| e.data.camera_3d.is_some()) else {
            return;
        };
        let Some((rgba, fw, fh)) = &self.camera_preview else {
            return;
        };
        let s = canvas.scale();
        let (lw, lh) = (*fw as f32 / s, *fh as f32 / s);
        let r = graphics::preview_rect(rect, lw, lh);
        canvas.fill_rounded(
            r.x - 1.0,
            r.y - 1.0,
            r.w + 2.0,
            r.h + 2.0,
            layout::RADIUS,
            color::border(),
        );
        canvas.blit_rgba(r.x, r.y, *fw, *fh, rgba);
        let label = e.name().to_string();
        let tw = canvas.measure_text(layout::SMALL_SIZE, &label) + 14.0;
        canvas.fill_rounded(r.x + 6.0, r.y + 6.0, tw, 18.0, 4.0, color::surface());
        canvas.text(
            r.x + 13.0,
            r.y + 8.0,
            layout::SMALL_SIZE,
            &label,
            color::text(),
        );
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

    /// The bottom dock: the Console.
    fn bottom_dock(&mut self, canvas: &mut Canvas, rect: Rect) {
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
        let x = rect.x + 6.0;
        let errors = self
            .console
            .iter()
            .filter(|l| l.level == LogLevel::Error)
            .count();
        let console_label = if errors > 0 {
            format!(
                "Console · {errors} error{}",
                if errors == 1 { "" } else { "s" }
            )
        } else {
            "Console".to_string()
        };
        // A single tab today; the underline keeps it reading as a dock title.
        let label = console_label;
        let tw = canvas.measure_text(layout::TITLE_SIZE, &label) + 22.0;
        let r = Rect::new(x, rect.y, tw, layout::TITLEBAR_H);
        canvas.fill_rect(r.x + 8.0, r.y + r.h - 2.0, r.w - 16.0, 2.0, color::accent());
        canvas.text(
            r.x + 11.0,
            r.y + (r.h - layout::TITLE_SIZE) / 2.0 - 1.0,
            layout::TITLE_SIZE,
            &label,
            color::text(),
        );
        if self.bottom_tab == BottomTab::Console {
            let clear = Rect::new(
                rect.x + rect.w - 64.0,
                rect.y + 4.0,
                56.0,
                layout::TITLEBAR_H - 8.0,
            );
            if self.button(canvas, clear, "Clear", ButtonStyle::Ghost) {
                self.console.clear();
                self.console_scroll = 0.0;
            }
            let mut fx = clear.x - 6.0;
            for (level, label) in [
                (LogLevel::Error, "Errors"),
                (LogLevel::Warn, "Warnings"),
                (LogLevel::Info, "Info"),
            ] {
                let n = self.console.iter().filter(|l| l.level == level).count();
                let text = format!("{label} {n}");
                let bw = canvas.measure_text(layout::SMALL_SIZE, &text) + 30.0;
                fx -= bw + 2.0;
                let r = Rect::new(fx, rect.y + 4.0, bw, layout::TITLEBAR_H - 8.0);
                let on = !self.console_hidden.contains(&level);
                let hot = self.hot(r);
                if hot {
                    canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, color::hover());
                    self.cursor = CursorHint::Pointer;
                }
                let dot = match level {
                    LogLevel::Error => color::danger(),
                    LogLevel::Warn => color::warn(),
                    LogLevel::Info => color::text_dim(),
                };
                let (dot, fg) = if on {
                    (dot, color::text_dim())
                } else {
                    (color::text_faint(), color::text_faint())
                };
                if on {
                    canvas.fill_circle(r.x + 12.0, r.y + r.h / 2.0, 3.0, dot);
                } else {
                    canvas.stroke_circle(r.x + 12.0, r.y + r.h / 2.0, 3.0, dot, 1.0);
                }
                canvas.text(
                    r.x + 21.0,
                    r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
                    layout::SMALL_SIZE,
                    &text,
                    fg,
                );
                self.tip(r, if on { "Click to hide" } else { "Click to show" });
                if hot && self.p.pressed {
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
        if lines.is_empty() {
            let msg = if self.console.is_empty() {
                "Messages from Oberon, print() in your scripts and Python errors appear here."
            } else {
                "All messages are hidden by the filters above."
            };
            canvas.text_clipped(
                rect.x + layout::PAD,
                rect.y + 12.0,
                layout::FONT_SIZE,
                msg,
                color::text_faint(),
                rect.w - 2.0 * layout::PAD,
            );
            return;
        }
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
            let face = canvas.set_face(crate::canvas::Face::Mono);
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
            canvas.set_face(face);
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
        if let Some(status) = self.build_status.clone() {
            let tw = canvas.measure_text(layout::SMALL_SIZE, &status);
            let x = rect.x + rect.w - tw - layout::PAD;
            if x > rect.x + lw + 2.0 * layout::PAD {
                canvas.fill_circle(x - 10.0, rect.y + 11.5, 3.0, color::accent());
                canvas.text(x, rect.y + 5.0, layout::SMALL_SIZE, &status, color::text());
            }
            return;
        }
        if !self.prefs.status_hints {
            return;
        }
        let hint = match self.play {
            PlayState::Editing if self.code.active.is_some() => {
                "Ctrl+Space complete    F12 definition    Ctrl+F find    Ctrl+S save    Esc scene keys"
            }
            PlayState::Editing => {
                "Drag orbit    Middle-drag pan    Wheel zoom    F focus    W E R tools    Ctrl+P play"
            }
            _ => "Playing a copy of the scene. Stop (Ctrl+P) returns to editing.",
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
        Icon::Gear => {
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (s, co) = a.sin_cos();
                canvas.line(
                    cx + co * 5.0,
                    cy + s * 5.0,
                    cx + co * 8.0,
                    cy + s * 8.0,
                    c,
                    2.6,
                );
            }
            canvas.stroke_circle(cx, cy, 5.2, c, 2.0);
        }
        Icon::Search => {
            canvas.stroke_circle(cx - 1.0, cy - 1.0, 4.5, c, 1.4);
            canvas.line(cx + 2.4, cy + 2.4, cx + 6.0, cy + 6.0, c, 1.4);
        }
        Icon::Plus => {
            canvas.line(cx - 5.0, cy, cx + 5.0, cy, c, 1.5);
            canvas.line(cx, cy - 5.0, cx, cy + 5.0, c, 1.5);
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
    } else if d.environment.is_some() && d.mesh_visual.is_none() {
        // A horizon: sky arc over a ground line.
        let c = color::text_dim();
        canvas.stroke_circle(cx, cy + 1.0, 6.0, c, 1.3);
        canvas.fill_rect(cx - 8.0, cy + 1.0, 16.0, 7.0, color::surface());
        canvas.fill_rect(cx - 8.0, cy + 1.0, 16.0, 1.3, c);
    } else if d.camera_3d.is_some() {
        canvas.fill_rounded(cx - 6.0, cy - 4.0, 9.0, 8.0, 1.5, color::camera_icon());
        canvas.fill_polygon(
            &[(cx + 3.0, cy), (cx + 7.0, cy - 4.0), (cx + 7.0, cy + 4.0)],
            color::camera_icon(),
        );
    } else if d.mesh_visual.is_some() {
        let c = color::text_dim();
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
        // Meshes are their own marker; an environment has no position.
        if e.data.mesh_visual.is_some()
            || (e.data.environment.is_some()
                && e.data.camera_3d.is_none()
                && e.data.directional_light.is_none())
        {
            continue;
        }
        // A camera's marker sits at its eye, which is where its frustum starts.
        let at = match (e.data.camera_3d, e.data.transform) {
            (Some(cam), _) => cam.position,
            (None, Some(t)) => Vec3::from(t.position),
            (None, None) => continue,
        };
        let Some((x, y)) = proj.project(at) else {
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

/// Every camera's view pyramid (the selected one brighter).
fn draw_camera_frustums(
    canvas: &mut Canvas,
    doc: &Document,
    proj: &Projector,
    selected: Option<u64>,
) {
    for e in &doc.entities {
        let Some(cam) = e.data.camera_3d else {
            continue;
        };
        let hot = selected == Some(e.id);
        let depth = if hot { 1.6 } else { 0.9 };
        let corners = view::camera_frustum(&cam, 16.0 / 9.0, depth);
        let pts: Vec<Option<(f32, f32)>> = corners.iter().map(|c| proj.project(*c)).collect();
        let c = if hot {
            color::camera_icon()
        } else {
            color::text_faint()
        };
        let width = if hot { 1.5 } else { 1.0 };
        for i in 1..5 {
            let j = if i == 4 { 1 } else { i + 1 };
            if let (Some(a), Some(b)) = (pts[0], pts[i]) {
                canvas.line(a.0, a.1, b.0, b.1, c, width);
            }
            if let (Some(a), Some(b)) = (pts[i], pts[j]) {
                canvas.line(a.0, a.1, b.0, b.1, c, width);
            }
        }
        // A small notch marks the top edge (which way is up).
        if let (Some(a), Some(b)) = (pts[1], pts[2]) {
            let (mx, my) = (f32::midpoint(a.0, b.0), f32::midpoint(a.1, b.1));
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let (nx, ny) = (dy / len * 6.0, -dx / len * 6.0);
            canvas.fill_polygon(
                &[
                    (mx - dx / len * 5.0, my - dy / len * 5.0),
                    (mx + dx / len * 5.0, my + dy / len * 5.0),
                    (mx + nx, my + ny),
                ],
                c,
            );
        }
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

/// Entity and custom-attribute names in `doc` (for script completion).
fn scene_names(doc: &Document) -> crate::code::api::SceneNames {
    let mut names = crate::code::api::SceneNames::default();
    for e in &doc.entities {
        if let Some(n) = &e.data.name {
            if !names.entities.contains(n) {
                names.entities.push(n.clone());
            }
        }
        for key in e.data.attributes.keys() {
            if !names.attributes.contains(key) {
                names.attributes.push(key.clone());
            }
        }
    }
    names
}

/// Draws an X / Y / Z letter in its axis color at the left of a field;
/// returns the x where the value text starts.
pub(super) fn draw_axis_tag(canvas: &mut Canvas, r: Rect, c: Color) -> f32 {
    let letter = if c == color::axis_x() {
        "X"
    } else if c == color::axis_y() {
        "Y"
    } else {
        "Z"
    };
    canvas.text_in(
        crate::canvas::Face::Medium,
        r.x + 7.0,
        r.y + (r.h - layout::SMALL_SIZE) / 2.0 - 1.0,
        layout::SMALL_SIZE,
        letter,
        c,
    );
    r.x + 20.0
}

/// A 14 px folder or file outline centred on `(cx, cy)`.
pub(super) fn draw_small_file_icon(canvas: &mut Canvas, cx: f32, cy: f32, dir: bool) {
    let c = color::text_dim();
    if dir {
        canvas.polyline(
            &[
                (cx - 7.0, cy + 5.0),
                (cx - 7.0, cy - 5.0),
                (cx - 2.5, cy - 5.0),
                (cx - 1.0, cy - 3.0),
                (cx + 7.0, cy - 3.0),
                (cx + 7.0, cy + 5.0),
                (cx - 7.0, cy + 5.0),
            ],
            c,
            1.2,
        );
    } else {
        canvas.polyline(
            &[
                (cx - 5.0, cy - 6.0),
                (cx + 2.0, cy - 6.0),
                (cx + 5.0, cy - 3.0),
                (cx + 5.0, cy + 6.0),
                (cx - 5.0, cy + 6.0),
                (cx - 5.0, cy - 6.0),
            ],
            c,
            1.2,
        );
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
        let plus = (
            hier.x + hier.w - layout::TITLEBAR_H / 2.0,
            hier.y + layout::TITLEBAR_H / 2.0,
        );
        click(&mut ui, &mut c, &doc, plus.0, plus.1);
        assert!(ui.menu.is_some(), "+ Create opens the menu");
        let r = ui.menu.as_ref().unwrap().rect(&mut c, 1600.0, 950.0);
        // Second item: Cube.
        let item = (r.x + 30.0, r.y + 4.0 + layout::ROW_H * 1.5);
        let out = click(&mut ui, &mut c, &doc, item.0, item.1);
        assert!(out.contains(&Intent::Create(CreateKind::Cube)), "{out:?}");
        assert!(ui.menu.is_none());
    }

    #[test]
    fn graphics_tab_offers_scene_setup_and_tests() {
        let mut c = canvas();
        let mut doc = Document::starter();
        let mut ui = Ui::new();
        let insp = lay().inspector;
        // Click the "Graphics" tab (second tab in the right dock's title).
        let tab_x = insp.x + 2.0 + c.measure_text(layout::TITLE_SIZE, "Inspector") + 24.0 + 20.0;
        click(
            &mut ui,
            &mut c,
            &doc,
            tab_x,
            insp.y + layout::TITLEBAR_H / 2.0,
        );
        assert_eq!(ui.right_tab, RightTab::Graphics);
        // No environment yet: the panel offers to create one; with one, the
        // presets emit SetEnvironment.
        let _ = ui.frame(&mut c, &doc, Pointer::default(), None);
        doc.create(CreateKind::Environment);
        let env_id = doc.entities.last().unwrap().id;
        let _ = ui.frame(&mut c, &doc, Pointer::default(), None);
        // Quality buttons edit the preference directly.
        ui.prefs.graphics_quality = automata_renderer::GraphicsQuality::Low;
        let mut found_preset = false;
        // Scan the panel for the "Night" preset button by clicking along
        // the preset row candidates.
        for row in 0..40 {
            let y = insp.y + layout::TITLEBAR_H + 10.0 + row as f32 * 12.0;
            let x = insp.x + insp.w - layout::PAD - 20.0;
            let out = click(&mut ui, &mut c, &doc, x, y);
            // Other rows open pickers / menus or jump to the Inspector.
            ui.right_tab = RightTab::Graphics;
            ui.picker = None;
            ui.menu = None;
            if out.iter().any(|i| {
                matches!(i, Intent::SetEnvironment(id, env)
                    if *id == env_id && *env == automata_renderer::Environment::night())
            }) {
                found_preset = true;
                break;
            }
        }
        assert!(found_preset, "the Night preset is clickable");
    }

    #[test]
    fn camera_preview_is_requested_only_for_a_selected_camera() {
        let mut doc = Document::starter();
        let ui = Ui::new();
        assert!(ui
            .camera_preview_request(&doc, 1600.0, 950.0, 1.0)
            .is_none());
        let id = doc.create(CreateKind::Camera);
        doc.select(Some(id));
        let (_, w, h) = ui.camera_preview_request(&doc, 1600.0, 950.0, 2.0).unwrap();
        assert!(w > h && w >= 400, "{w}x{h} at 2x");
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
