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
use std::time::Instant;

use automata_core::{AttributeValue, EntityData, TransformData};
use glam::Vec3;
use tiny_skia::Color;

use crate::canvas::Canvas;
use crate::doc::{ComponentKind, CreateKind, Document, EntityEntry};
use crate::fields::{self, Choice, Field, Flag, NavBakeField, NavField};
use crate::theme::{color, layout};
use crate::view::{self, Gizmo, GizmoMode, Projector};

mod anim_canvas;
pub mod anim_editor;
mod anim_panels;
pub mod assets_panel;
pub mod attributes;
pub mod brush;
pub mod clip_files;
mod code_view;
pub mod dialogs;
pub mod file_browser;
pub mod graphics;
pub mod material_editor;
pub mod menus;
pub mod palette;
pub mod picker;
pub mod prefab_panel;
pub mod profiler;
pub mod project_panel;
pub mod settings;
pub mod springs;
pub mod ui_editor;

use dialogs::Dialog;
pub use graphics::{FrameStats, RightTab};
pub use material_editor::MaterialShape;
use menus::MENUBAR_H;
pub use menus::{DialogKind, Panel};
use picker::ColorPicker;
pub use picker::ColorTarget;

pub use crate::view::ViewCamera;

/// The Inspector's animation preview: an animated model playing (or
/// scrubbed to a moment) in the scene view while editing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimPreview {
    /// The document entity being previewed.
    pub id: u64,
    /// Whether time advances.
    pub playing: bool,
    /// Seconds into the entity's clip.
    pub time: f32,
}

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
    /// The project's models and sounds as thumbnails.
    Assets,
    /// Per-frame timings: scripts, physics, rendering, editor UI.
    Profiler,
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
    /// Ctrl+click (`range: false`: add or remove one entity) or Shift+click
    /// (`range: true`: everything from the primary to it) in the selection.
    SelectMore {
        /// The entity clicked.
        id: u64,
        /// Select the range up to it.
        range: bool,
    },
    /// Select exactly these entities (the last one primary).
    SelectMany(Vec<u64>),
    /// Copy an entity `count` times along a line (one undo step).
    DuplicateAlong {
        /// The entity to copy.
        id: u64,
        /// How many copies.
        count: usize,
        /// The offset from one copy to the next (metres, world space).
        step: [f32; 3],
        /// The turn from one copy to the next about Y (radians).
        turn_y: f32,
    },
    /// Write an instance's overrides into its prefab file; every instance
    /// follows.
    ApplyPrefab(u64),
    /// Put the prefab's values back on an instance: these override paths,
    /// or all of them (`None`).
    RevertPrefab {
        /// Entity id.
        id: u64,
        /// The override paths (`None`: every override).
        paths: Option<Vec<String>>,
    },
    /// Take an instance off its prefab (it keeps its values).
    UnlinkPrefab(u64),
    /// Select every instance of the prefab an entity follows.
    SelectInstances(u64),
    /// Write a variant of a prefab file (a prefab that follows it).
    CreateVariant(PathBuf),
    /// Lay a foliage brush dab on an entity's Grass or Scatter (part of a
    /// stroke: coalesced undo).
    Brush {
        /// Entity id.
        id: u64,
        /// The dab's centre on the surface (world space).
        point: [f32; 3],
        /// The surface's normal there.
        normal: [f32; 3],
        /// The brush's radius (metres).
        radius: f32,
        /// -1..1: positive paints, negative erases.
        strength: f32,
    },
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
    /// Point an entity's Animation at another clip ("" for none).
    SetAnimationClip {
        /// Entity id.
        id: u64,
        /// The clip's name.
        clip: String,
    },
    /// Drive an entity's Animation with an animation graph (`None`: play
    /// its clip again). A graph file that doesn't exist yet is made,
    /// starting from the model's clips.
    SetAnimGraph {
        /// Entity id.
        id: u64,
        /// The `.anim.ron` file.
        path: Option<PathBuf>,
    },
    /// Point an entity's Attach To Bone at another entity (by name) and/or
    /// one of its bones; `None` leaves that part as it is.
    SetAttach {
        /// Entity id.
        id: u64,
        /// The entity to follow.
        to: Option<String>,
        /// The bone to follow ("" for the entity itself).
        bone: Option<String>,
    },
    /// Bake the scene's navigation mesh from its static colliders and
    /// save it next to the scene (the Navigation component's Bake).
    BakeNav(u64),
    /// Point a Nav Link at another entity by name.
    SetNavLink {
        /// Entity id.
        id: u64,
        /// The entity at the other end.
        to: String,
    },
    /// Point an entity's Joint at another entity by name (`None`: the world).
    SetJointTarget {
        /// Entity id.
        id: u64,
        /// The entity to join.
        target: Option<String>,
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
    /// Create an entity showing a model file. `at` is the screen point it
    /// was dropped on (it lands on the ground there); `None` places it
    /// where the scene camera is looking.
    AddModel {
        /// The model file.
        path: PathBuf,
        /// Drop point in logical pixels.
        at: Option<(f32, f32)>,
    },
    /// Show a model file on an existing entity.
    SetModel {
        /// Entity id.
        id: u64,
        /// The model file.
        path: PathBuf,
    },
    /// Put a material file on an entity's Mesh Renderer (`None` clears
    /// it).
    SetMaterial {
        /// Entity id.
        id: u64,
        /// The `.mat.ron` file.
        path: Option<PathBuf>,
    },
    /// Create a material file (if missing) and open it in the material
    /// editor.
    NewMaterial(PathBuf),
    /// Refit an entity's box collider to its mesh.
    FitCollider(u64),
    /// Save a model file's import settings (scale, rotation, pivot) to its
    /// `.import.ron`; every entity using the file updates.
    SetImport {
        /// The model file (absolute).
        model: PathBuf,
        /// The new settings.
        settings: automata_assets::import::ImportSettings,
    },
    /// Bring a font into the project (copying it into `fonts/` when it's
    /// outside) and put its path, as scripts write it, on the clipboard.
    ImportFont(PathBuf),
    /// Create an entity with an Audio Source playing a sound file. `at` is
    /// the drop point in the scene view (`None`: where the camera looks).
    AddSound {
        /// The sound file.
        path: PathBuf,
        /// Drop point in logical pixels.
        at: Option<(f32, f32)>,
    },
    /// Give an entity an Audio Source with this clip (or swap its clip).
    SetAudioClip {
        /// Entity id.
        id: u64,
        /// The sound file.
        path: PathBuf,
    },
    /// Play a sound file once in the editor (again stops it).
    PreviewSound(PathBuf),
    /// Open a script in the code editor.
    OpenScript(PathBuf),
    /// Create a script from the template (if missing) and open it.
    NewScript(PathBuf),
    /// Create a UI document from a template and open it.
    NewUiScreen(PathBuf),
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
    /// Rename or move a file or folder on disk; scene references to it
    /// follow.
    MoveFile {
        /// The current path.
        from: PathBuf,
        /// The new path.
        to: PathBuf,
    },
    /// Copy a file or folder next to itself (`name copy.ext`).
    DuplicateFile(PathBuf),
    /// Delete a file or folder (to the Trash where the desktop has one).
    DeleteFile(PathBuf),
    /// Show a file in the system file manager.
    RevealFile(PathBuf),
    /// Quit the editor (asks to save unsaved changes first).
    Quit,
    /// Restart the language server (its settings changed).
    RestartLanguageServer,
    /// Replace an entity's whole Environment (a preset).
    SetEnvironment(u64, Box<automata_renderer::Environment>),
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
    /// The entity an edit applies to, for intents that can apply to every
    /// selected entity at once.
    #[must_use]
    pub fn fan_target(&self) -> Option<u64> {
        match self {
            Self::AddComponent(id, _)
            | Self::RemoveComponent(id, _)
            | Self::ResetComponent(id, _)
            | Self::ResetTransform(id)
            | Self::FitCollider(id)
            | Self::DetachScript(id)
            | Self::Delete(id)
            | Self::Duplicate(id)
            | Self::UnlinkPrefab(id)
            | Self::SetField { id, .. }
            | Self::SetColor { id, .. }
            | Self::SetChoice { id, .. }
            | Self::Toggle { id, .. }
            | Self::Cycle { id, .. }
            | Self::SetTransform { id, .. }
            | Self::SetAnimationClip { id, .. }
            | Self::SetAnimGraph { id, .. }
            | Self::SetJointTarget { id, .. }
            | Self::SetAttach { id, .. }
            | Self::SetNavLink { id, .. }
            | Self::PasteComponent { id, .. }
            | Self::SetAttribute { id, .. }
            | Self::RemoveAttribute { id, .. }
            | Self::RenameAttribute { id, .. }
            | Self::SetModel { id, .. }
            | Self::SetMaterial { id, .. }
            | Self::SetAudioClip { id, .. }
            | Self::AttachScript { id, .. }
            | Self::RevertPrefab { id, paths: None } => Some(*id),
            _ => None,
        }
    }

    /// This intent aimed at entity `to` instead (for the intents
    /// [`Intent::fan_target`] names, except the ones that need the
    /// document: toggles, cycles, transforms, duplicates).
    #[must_use]
    pub fn retarget(&self, to: u64) -> Option<Self> {
        let mut out = self.clone();
        match &mut out {
            Self::AddComponent(id, _)
            | Self::RemoveComponent(id, _)
            | Self::ResetComponent(id, _)
            | Self::ResetTransform(id)
            | Self::FitCollider(id)
            | Self::DetachScript(id)
            | Self::Delete(id)
            | Self::UnlinkPrefab(id)
            | Self::SetField { id, .. }
            | Self::SetColor { id, .. }
            | Self::SetChoice { id, .. }
            | Self::SetAnimationClip { id, .. }
            | Self::SetAnimGraph { id, .. }
            | Self::SetJointTarget { id, .. }
            | Self::SetAttach { id, .. }
            | Self::SetNavLink { id, .. }
            | Self::PasteComponent { id, .. }
            | Self::SetAttribute { id, .. }
            | Self::RemoveAttribute { id, .. }
            | Self::RenameAttribute { id, .. }
            | Self::SetModel { id, .. }
            | Self::SetMaterial { id, .. }
            | Self::SetAudioClip { id, .. }
            | Self::AttachScript { id, .. }
            | Self::RevertPrefab { id, paths: None } => *id = to,
            _ => return None,
        }
        Some(out)
    }

    /// Whether this intent edits the document (blocked during play mode).
    #[must_use]
    pub fn edits_document(&self) -> bool {
        !matches!(
            self,
            Self::Select(_)
                | Self::SelectMore { .. }
                | Self::SelectMany(_)
                | Self::SelectInstances(_)
                | Self::CreateVariant(_)
                | Self::EndGesture
                | Self::Save
                | Self::SaveAs(_)
                | Self::SavePrefab { .. }
                | Self::OpenScript(_)
                | Self::OpenScriptAt { .. }
                | Self::NewScript(_)
                | Self::NewUiScreen(_)
                | Self::NewMaterial(_)
                | Self::NewFolder(_)
                | Self::DuplicateFile(_)
                | Self::DeleteFile(_)
                | Self::RevealFile(_)
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
    /// A number of a model's import settings. The file is kept in
    /// `Ui::import_edit`.
    Import(springs::ImportField),
    /// The Assets browser's filter.
    AssetQuery,
    /// A Files panel row being renamed in place.
    FileRename,
    /// A UI editor property.
    UiProp(ui_editor::UiProp),
    /// A material editor property.
    MatProp(material_editor::MatProp),
    /// An animation graph editor field.
    AnimProp(anim_editor::AnimProp),
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
    /// With Ctrl or Shift held (`adding`), a click adds to the selection
    /// and a drag draws a selection box.
    ViewPress {
        start: (f32, f32),
        last: (f32, f32),
        orbiting: bool,
        double: bool,
        adding: bool,
    },
    /// Dragging a gizmo handle. `start` is the transform at the press,
    /// `free` where the drag has got to before snapping.
    Gizmo {
        id: u64,
        axis: usize,
        last: (f32, f32),
        start: TransformData,
        free: TransformData,
    },
    /// A foliage brush stroke on an entity.
    Brush {
        id: u64,
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
    /// Decoded sound summaries for the Inspector.
    pub clips: crate::sounds::ClipCache,
    /// The project directory (file dialogs and the Files panel), set by
    /// the app.
    pub project_root: PathBuf,
    /// A file being dragged out of the Files panel (drop a model on the
    /// scene view to add it, or on the Inspector to use it).
    file_drag: Option<project_panel::FileDrag>,
    /// Whether the OS is dragging files over the window (shows the drop
    /// hint), set by the app.
    pub os_file_hover: bool,
    /// The scene view's rectangle in the last frame (maps drop points to
    /// the ground).
    last_viewport: Rect,
    /// Set by [`Ui::frame_composited`]: the size (physical pixels) of a
    /// scene view the GPU draws, so the UI leaves a hole for it.
    composited_view: Option<(u32, u32)>,
    /// Where that hole's top-left corner landed this frame (physical).
    composited_origin: Option<(i32, i32)>,
    /// Panel sizes and visibility.
    pub panels: PanelSizes,
    /// Active bottom-dock tab.
    pub bottom_tab: BottomTab,
    /// Active right-dock tab.
    pub right_tab: RightTab,
    /// Frame statistics for the Graphics panel (set by the app).
    pub stats: FrameStats,
    /// Per-frame timings for the Profiler tab (recorded by the app).
    pub profiler: profiler::Profiler,
    /// The selected camera's view, rendered by the app: RGBA, size.
    camera_preview: Option<(Vec<u8>, u32, u32)>,
    /// The animation playing or scrubbed in the scene view while editing
    /// (the app advances it and poses the model).
    pub anim_preview: Option<AnimPreview>,
    /// The baked navigation mesh drawn over the scene view, and the file
    /// it came from.
    pub nav_overlay: Option<(PathBuf, std::sync::Arc<automata_nav::NavMesh>)>,
    /// A navigation mesh file that failed to load (not retried each frame).
    pub nav_missing: Option<PathBuf>,
    /// Whether the navigation mesh is drawn while editing.
    pub show_nav: bool,
    /// The foliage brush (Grass and Scatter painting).
    pub brush: brush::Brush,
    /// The animation preview's time bar is being dragged.
    anim_scrub: bool,
    /// The context menu opened this frame (its opening press picks nothing).
    menu_fresh: bool,
    recent_colors: Vec<[f32; 4]>,
    /// The Files panel's height last frame (keyboard scrolling).
    last_files_h: f32,
    /// Where each kind of file dialog was last used (this session).
    dialog_dirs: std::collections::HashMap<&'static str, PathBuf>,
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
    /// The project's string tables (`lang/*.csv`).
    pub strings: ProjectStrings,
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
    /// The UI editor (for `.ui.ron` tabs).
    uied: ui_editor::UiEditor,
    mated: material_editor::MaterialEditor,
    /// The animation graph editor (for `.anim.ron` tabs).
    animed: anim_editor::AnimEditor,
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
    /// The model file and settings an `EditTarget::Import` field edits.
    import_edit: Option<(PathBuf, automata_assets::import::ImportSettings)>,
    /// The Assets tab: model thumbnails and sounds.
    pub assets: assets_panel::AssetBrowser,
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
            import_edit: None,
            assets: assets_panel::AssetBrowser::default(),
            dialog: None,
            recent: Vec::new(),
            clips: crate::sounds::ClipCache::default(),
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            file_drag: None,
            os_file_hover: false,
            last_viewport: Rect::default(),
            composited_view: None,
            composited_origin: None,
            build_status: None,
            build_last: None,
            build_cfg: None,
            panels: PanelSizes::default(),
            bottom_tab: BottomTab::Console,
            right_tab: RightTab::Inspector,
            stats: FrameStats::default(),
            profiler: profiler::Profiler::default(),
            camera_preview: None,
            anim_preview: None,
            nav_overlay: None,
            nav_missing: None,
            show_nav: true,
            brush: brush::Brush::default(),
            anim_scrub: false,
            menu_fresh: false,
            recent_colors: Vec::new(),
            dialog_dirs: std::collections::HashMap::new(),
            last_files_h: 0.0,
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
            strings: ProjectStrings::default(),
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
            uied: ui_editor::UiEditor::default(),
            mated: material_editor::MaterialEditor::default(),
            animed: anim_editor::AnimEditor::default(),
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

    /// The scene view's rectangle in the last frame (logical pixels).
    #[must_use]
    pub fn viewport_rect(&self) -> Rect {
        self.last_viewport
    }

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
        let dialog_field = edit.target == EditTarget::DialogName;
        match key {
            EditKey::Backspace if dialog_field && edit.buffer.is_empty() => {
                self.file_dialog_key(file_browser::NavKey::Parent);
                None
            }
            EditKey::Tab if dialog_field => {
                self.file_dialog_complete();
                None
            }
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
                if dialog_field {
                    if !self.file_dialog_escape() {
                        self.close_dialog();
                    }
                    return None;
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
            EditTarget::FileRename => self.commit_file_rename(&edit.buffer),
            EditTarget::UiProp(prop) => {
                self.commit_ui_prop(prop, &edit.buffer);
                None
            }
            EditTarget::MatProp(prop) => {
                self.commit_material_prop(prop, &edit.buffer);
                None
            }
            EditTarget::AnimProp(prop) => {
                self.commit_anim_prop(prop, &edit.buffer);
                None
            }
            EditTarget::Setting(which) => self.commit_setting(which, &edit.buffer),
            EditTarget::AssetQuery => {
                self.assets.query = edit.buffer.trim().to_string();
                None
            }
            EditTarget::Import(which) => {
                let (model, mut settings) = self.import_edit.take()?;
                let value = eval_number(&edit.buffer)?;
                which
                    .apply(&mut settings, value)
                    .then_some(Intent::SetImport { model, settings })
            }
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

    /// [`Self::frame`] for a scene view the GPU composites
    /// ([`automata_renderer::Renderer::present_composited`]): the view's
    /// `size` (physical pixels) is left transparent. Returns the edits and
    /// where the view's top-left corner landed (physical pixels), or `None`
    /// when the scene view isn't showing.
    pub fn frame_composited(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        pointer: Pointer,
        size: (u32, u32),
    ) -> (Vec<Intent>, Option<(i32, i32)>) {
        self.composited_view = Some(size);
        self.composited_origin = None;
        let intents = self.frame(canvas, doc, pointer, None);
        self.composited_view = None;
        (intents, self.composited_origin.take())
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

        let t = Instant::now();
        self.code.settings.clone_from(&self.prefs.editor);
        self.refresh_strings(canvas, doc);
        if self.code.active.is_some() {
            self.code.scene_names = scene_names(doc);
            self.code.scene_names.strings = self.string_keys();
        }
        self.profiler.since("UI: string tables, completions", t);
        self.update_scrub();
        self.update_attr_scrub();
        self.update_splitters(&lay, w, h);

        let t = Instant::now();
        canvas.clear(color::background());
        self.profiler.since("UI: clear", t);
        // Scene view first: its overlays may spill outside its rect and are
        // covered by the panels drawn after it.
        let t = Instant::now();
        if self.code.active.is_some() {
            if self.anim_editor_shown() {
                self.anim_find_model(doc);
            }
            self.code_view(canvas, lay.viewport);
            self.profiler.since("UI: script / document editor", t);
        } else {
            self.scene_view(canvas, doc, lay.viewport, viewport);
            self.profiler.since("UI: scene view overlays", t);
        }
        if let Some(r) = self.doc_tabs_rect(lay.viewport) {
            self.doc_tabs(canvas, r);
        }
        let t = Instant::now();
        if self.panels.show_bottom {
            self.bottom_dock(canvas, doc, lay.console);
        }
        self.profiler.since("UI: bottom dock", t);
        let t = Instant::now();
        if self.panels.show_hierarchy {
            self.hierarchy(canvas, doc, lay.hierarchy);
        }
        self.profiler.since("UI: hierarchy", t);
        let t = Instant::now();
        if self.panels.show_files {
            self.files_panel(canvas, doc, lay.files);
        }
        self.profiler.since("UI: files", t);
        let t = Instant::now();
        if self.panels.show_inspector {
            self.inspector(canvas, doc, lay.inspector);
        }
        self.profiler.since("UI: inspector", t);
        let t = Instant::now();
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
        self.last_viewport = lay.viewport;
        self.update_file_drag(canvas, doc, &lay);
        self.profiler.since("UI: menus, bars, popups", t);

        std::mem::take(&mut self.out)
    }

    /// Where a dragged model would go if dropped at `(x, y)`.
    fn file_drop_target(&self, doc: &Document, lay: &Layout, x: f32, y: f32) -> FileDrop {
        if self.panels.show_inspector && lay.inspector.contains(x, y) {
            if let Some(i) = doc.selected_index() {
                return FileDrop::Entity(doc.entities[i].id, lay.inspector);
            }
        }
        if self.panels.show_hierarchy && lay.hierarchy.contains(x, y) {
            if let Some((id, row)) = self.hierarchy_row_at(doc, lay.hierarchy, x, y) {
                return FileDrop::Entity(id, row);
            }
        }
        if self.code.active.is_none() && lay.viewport.contains(x, y) {
            return FileDrop::Scene(lay.viewport);
        }
        FileDrop::Nothing
    }

    /// The Hierarchy row under `(x, y)`: its entity and rectangle (the
    /// same layout [`Self::hierarchy`] draws).
    fn hierarchy_row_at(&self, doc: &Document, rect: Rect, x: f32, y: f32) -> Option<(u64, Rect)> {
        let top = rect.y + layout::TITLEBAR_H;
        if y < top || !rect.contains(x, y) {
            return None;
        }
        let first = top + 4.0 - self.hierarchy_scroll;
        let i = ((y - first) / layout::ROW_H).floor();
        if i < 0.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let e = doc.entities.get(i as usize)?;
        let row = Rect::new(
            rect.x + 4.0,
            first + i * layout::ROW_H,
            rect.w - 9.0,
            layout::ROW_H,
        );
        Some((e.id, row))
    }

    /// The object with a Mesh Renderer under `(x, y)` in the scene view.
    fn mesh_under(&self, doc: &Document, view: Rect, x: f32, y: f32) -> Option<u64> {
        let proj = view::Projector::new(&self.camera, view);
        view::pick(doc, &proj, x, y).filter(|&id| {
            doc.index_of(id)
                .is_some_and(|i| doc.entities[i].data.mesh_visual.is_some())
        })
    }

    /// Drives a file dragged out of the Files panel (or over the window from
    /// the OS): highlights where it would land, and on release moves it
    /// into a folder, adds a model / sound / prefab to the scene, puts a
    /// model or sound on the selected entity, or attaches a script.
    fn update_file_drag(&mut self, canvas: &mut Canvas, doc: &Document, lay: &Layout) {
        use project_panel::AssetKind;
        let (x, y) = (self.raw.x, self.raw.y);
        if self.os_file_hover {
            if self.code.active.is_none() {
                draw_drop_zone(canvas, lay.viewport, "Drop a model to add it to the scene");
            }
            return;
        }
        let Some(drag) = self.file_drag.as_mut() else {
            return;
        };
        if !drag.active && ((x - drag.origin.0).abs() > 4.0 || (y - drag.origin.1).abs() > 4.0) {
            drag.active = true;
        }
        let drag = drag.clone();
        let kind = project_panel::asset_kind(&drag.path, false);
        let folder = self.project_drop_dir();
        let target = match (self.file_drop_target(doc, lay, x, y), kind) {
            _ if folder.is_some() => FileDrop::Nothing,
            // A material goes on the object under the pointer.
            (FileDrop::Scene(r), AssetKind::Material) => self
                .mesh_under(doc, r, x, y)
                .map_or(FileDrop::Nothing, |id| FileDrop::Entity(id, r)),
            (
                t @ (FileDrop::Scene(_) | FileDrop::Entity(..)),
                AssetKind::Model | AssetKind::Audio,
            )
            | (
                t @ FileDrop::Entity(..),
                AssetKind::Material | AssetKind::Script | AssetKind::AnimGraph,
            )
            | (t @ FileDrop::Scene(_), AssetKind::Prefab) => t,
            _ => FileDrop::Nothing,
        };
        let editing = self.play == PlayState::Editing;
        if !self.raw.down {
            self.file_drag = None;
            if !drag.active || !editing {
                return;
            }
            if let Some(dir) = folder {
                let to = dir.join(drag.path.file_name().unwrap_or_default());
                self.out.push(Intent::MoveFile {
                    from: drag.path,
                    to,
                });
                return;
            }
            let path = drag.path;
            let intent = match (target, kind) {
                (FileDrop::Scene(_), AssetKind::Audio) => Some(Intent::AddSound {
                    path,
                    at: Some((x, y)),
                }),
                (FileDrop::Scene(_), AssetKind::Prefab) => Some(Intent::InstantiatePrefab(path)),
                (FileDrop::Scene(_), _) => Some(Intent::AddModel {
                    path,
                    at: Some((x, y)),
                }),
                (FileDrop::Entity(id, _), AssetKind::Audio) => {
                    Some(Intent::SetAudioClip { id, path })
                }
                (FileDrop::Entity(id, _), AssetKind::Script) => {
                    Some(Intent::AttachScript { id, path })
                }
                (FileDrop::Entity(id, _), AssetKind::Material) => Some(Intent::SetMaterial {
                    id,
                    path: Some(path),
                }),
                (FileDrop::Entity(id, _), AssetKind::AnimGraph) => Some(Intent::SetAnimGraph {
                    id,
                    path: Some(path),
                }),
                (FileDrop::Entity(id, _), _) => Some(Intent::SetModel { id, path }),
                (FileDrop::Nothing, _) => None,
            };
            if let Some(intent) = intent {
                self.out.push(intent);
            }
            return;
        }
        if !drag.active {
            return;
        }
        let name = drag
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut label = name.clone();
        match &target {
            FileDrop::Scene(r) => {
                let what = match kind {
                    AssetKind::Audio => "Add a sound here",
                    AssetKind::Prefab => "Instantiate the prefab",
                    _ => "Add to scene",
                };
                draw_drop_zone(canvas, *r, what);
            }
            FileDrop::Entity(id, r) => {
                let who = doc.index_of(*id).map_or("", |i| doc.entities[i].name());
                let what = match kind {
                    AssetKind::Audio => format!("Play from {who}"),
                    AssetKind::Script => format!("Attach to {who}"),
                    AssetKind::AnimGraph => format!("Animate {who}"),
                    _ => format!("Use on {who}"),
                };
                if r.h < 60.0 {
                    // A Hierarchy row: outline it; the label rides along.
                    canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.5);
                    label = what;
                } else {
                    draw_drop_zone(canvas, *r, &what);
                }
            }
            FileDrop::Nothing => {
                if let Some(dir) = &folder {
                    label = format!("Move {name} to {}", crate::files::display_name(dir));
                } else {
                    self.cursor = CursorHint::Grabbing;
                }
            }
        }
        // The file follows the pointer.
        let tw = canvas.measure_text(layout::SMALL_SIZE, &label) + 34.0;
        let (gx, gy) = (x + 14.0, y + 10.0);
        canvas.fill_rounded(gx + 1.0, gy + 3.0, tw, 24.0, 5.0, color::shadow());
        canvas.fill_rounded(gx, gy, tw, 24.0, 5.0, color::raised());
        canvas.stroke_rounded(gx, gy, tw, 24.0, 5.0, color::border(), 1.0);
        project_panel::draw_tree_icon(canvas, kind, gx + 14.0, gy + 12.0);
        canvas.text(
            gx + 26.0,
            gy + 5.0,
            layout::SMALL_SIZE,
            &label,
            color::text(),
        );
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
        self.snap_group(canvas, view);
    }

    /// The snapping controls, right of the tool group: an on/off toggle
    /// and the current tool's step (click for the next, Shift for the
    /// previous).
    fn snap_group_rect(canvas: &mut Canvas, view: Rect) -> Rect {
        let tools = Self::tool_group_rect(canvas, view);
        let w = canvas.measure_text(layout::FONT_SIZE, "Snap")
            + canvas.measure_text(layout::FONT_SIZE, "0.25 m")
            + 52.0;
        Rect::new(tools.x + tools.w + 6.0, tools.y, w, tools.h)
    }

    fn snap_group(&mut self, canvas: &mut Canvas, view: Rect) {
        let g = Self::snap_group_rect(canvas, view);
        canvas.fill_rounded(g.x, g.y, g.w, g.h, layout::RADIUS + 1.0, color::surface());
        let on = self.prefs.snap.enabled;
        let sw = canvas.measure_text(layout::FONT_SIZE, "Snap") + 24.0;
        let toggle = Rect::new(g.x + 2.0, g.y + 2.0, sw, g.h - 4.0);
        let step = Rect::new(toggle.x + toggle.w, g.y + 2.0, g.w - sw - 4.0, g.h - 4.0);
        for (r, active) in [(toggle, on), (step, false)] {
            if active || self.hot(r) {
                let bg = if active {
                    color::select()
                } else {
                    color::hover()
                };
                canvas.fill_rounded(r.x, r.y, r.w, r.h, layout::RADIUS, bg);
            }
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
            }
        }
        let ty = toggle.y + (toggle.h - layout::FONT_SIZE) / 2.0 - 1.0;
        canvas.text(
            toggle.x + 12.0,
            ty,
            layout::FONT_SIZE,
            "Snap",
            if on { color::text() } else { color::text_dim() },
        );
        let label = self.prefs.snap.label(self.gizmo);
        let lw = canvas.measure_text(layout::FONT_SIZE, &label);
        canvas.text(
            step.x + (step.w - lw) / 2.0,
            ty,
            layout::FONT_SIZE,
            &label,
            if on {
                color::text_dim()
            } else {
                color::text_faint()
            },
        );
        self.tip(
            toggle,
            "Snap to the grid while dragging  (hold Ctrl to flip)",
        );
        self.tip(step, "Snap step for this tool (click for the next)");
        if self.hot(toggle) && self.p.pressed {
            self.prefs.snap.enabled = !on;
            self.prefs_dirty = true;
        }
        if self.hot(step) && self.p.pressed {
            let options: &[f32] = match self.gizmo {
                GizmoMode::Move => &view::Snap::STEPS,
                GizmoMode::Rotate => &view::Snap::ANGLES,
                GizmoMode::Scale => &view::Snap::SCALES,
            };
            let current = self.prefs.snap.step_for(self.gizmo);
            let at = options
                .iter()
                .position(|v| (v - current).abs() < 1e-4)
                .unwrap_or(0);
            let n = options.len();
            let next = if self.p.shift {
                (at + n - 1) % n
            } else {
                (at + 1) % n
            };
            self.prefs.snap.set_step(self.gizmo, options[next]);
            self.prefs.snap.enabled = true;
            self.prefs_dirty = true;
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
                let selected = doc.is_selected(entity.id);
                let primary = doc.selected == Some(entity.id);
                let hot = self.hot(row);
                if selected {
                    canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::select());
                    if primary && doc.selection_len() > 1 {
                        // The lead (the Inspector shows its values).
                        canvas.fill_rect(row.x, row.y + 4.0, 2.0, row.h - 8.0, color::accent());
                    }
                } else if hot {
                    canvas.fill_rounded(row.x, row.y, row.w, row.h, 3.0, color::hover());
                }
                entity_icon(canvas, row.x + 12.0, row.y + row.h / 2.0, &entity.data);
                // Prefab instances read in blue (as in Unity).
                let fg = match (entity.data.prefab.is_some(), selected) {
                    (true, _) => color::axis_z(),
                    (false, true) => color::text(),
                    (false, false) => color::text_dim(),
                };
                canvas.text_clipped(
                    row.x + 26.0,
                    row.y + 5.0,
                    layout::FONT_SIZE,
                    entity.name(),
                    fg,
                    row.w - 30.0,
                );
                if hot && self.p.pressed {
                    row_clicked = true;
                    // Ctrl adds or removes one; Shift extends a range.
                    if self.p.ctrl || self.p.shift {
                        self.out.push(Intent::SelectMore {
                            id: entity.id,
                            range: self.p.shift,
                        });
                    } else {
                        self.out.push(Intent::Select(Some(entity.id)));
                        if self.p.double_click {
                            self.focus_entity(entity);
                        }
                    }
                }
                if hot && self.p.right_pressed {
                    row_clicked = true;
                    // Right-clicking inside a multi-selection keeps it, so
                    // the menu's commands apply to all of it.
                    if !selected {
                        self.out.push(Intent::Select(Some(entity.id)));
                    }
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
        let line = Rect::new(x, y, w, 14.0);
        if self.row_visible(line) {
            canvas.text(
                x,
                y,
                layout::SMALL_SIZE,
                &format!("Entity #{id}"),
                color::text_dim(),
            );
            // Script keys ("Name:animation:clip") need one entity per name.
            if let Some(problem) = name_problem(doc, e) {
                let warning = Rect::new(x + 70.0, y - 2.0, w - 70.0, 16.0);
                canvas.text_clipped(
                    warning.x,
                    y,
                    layout::SMALL_SIZE,
                    &problem,
                    color::warn(),
                    warning.w,
                );
                self.tip(
                    warning,
                    "Scripts find entities by name (ctx.get(\"Name:attr:health\")), so each needs its own, without ':'",
                );
            }
        }
        y += 22.0;
        y = self.multi_select_note(canvas, doc, x, y, w);
        y = self.prefab_block(canvas, doc, e, x, y, w);

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
                y = self.model_rows(canvas, doc, x, y, w, e);
                y = self.float_row(canvas, x, y, w, e, "Size", Field::MeshSize);
                let material = d
                    .mesh_visual
                    .as_ref()
                    .map(|m| m.material.trim().to_string())
                    .unwrap_or_default();
                y = self.material_row(canvas, doc, x, y, w, id, &material);
                if !material.is_empty() {
                    // The material owns colour and surface.
                    if self.row_visible(Rect::new(x, y, w, 16.0)) {
                        canvas.text(
                            x + layout::LABEL_W,
                            y,
                            layout::SMALL_SIZE,
                            "Colour and surface come from the material",
                            color::text_faint(),
                        );
                    }
                    y += 22.0;
                }
            }
            if open && material_free(d) {
                let is_model = matches!(
                    d.mesh_visual.as_ref().map(|m| &m.primitive),
                    Some(automata_core::PrimitiveData::Model { .. })
                );
                let color_label = if is_model { "Tint" } else { "Color" };
                y = self.color_row(canvas, x, y, w, e, color_label, ColorTarget::Mesh);
                y = self.choice_row(canvas, x, y, w, e, "Surface", Choice::MeshSurface);
                y = self.float_row(canvas, x, y, w, e, "Roughness", Field::MeshRoughness);
                y = self.float_row(canvas, x, y, w, e, "Metallic", Field::MeshMetallic);
                y = self.color_row(canvas, x, y, w, e, "Emission", ColorTarget::MeshEmissive);
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Emission Strength",
                    Field::MeshEmissiveStrength,
                );
                y = self.float_row(canvas, x, y, w, e, "Glow Light", Field::MeshGlowLight);
                y = self.flag_row(canvas, x, y, w, e, "Glow Shadows", Flag::MeshGlowShadows);
                y += 6.0;
            }
        }
        if d.animation.is_some() {
            let kind = ComponentKind::Animation;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.animation_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }
        if d.directional_light.is_some() {
            let kind = ComponentKind::Light;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.float_row(canvas, x, y, w, e, "Time of Day", Field::LightTimeOfDay);
                y = self.float_row(canvas, x, y, w, e, "Azimuth", Field::LightAzimuth);
                y = self.float_row(canvas, x, y, w, e, "Elevation", Field::LightElevation);
                y = self.vec3_row(canvas, x, y, w, e, "Direction", Field::LightDirection);
                y = self.color_row(canvas, x, y, w, e, "Color", ColorTarget::Light);
                y = self.float_row(canvas, x, y, w, e, "Ambient", Field::LightAmbient);
                y = self.flag_row(canvas, x, y, w, e, "Cast Shadows", Flag::LightShadows);
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Shadow Distance",
                    Field::LightShadowDistance,
                );
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Shadow Softness",
                    Field::LightShadowSoftness,
                );
                y += 6.0;
            }
        }
        if d.point_light.is_some() {
            let kind = ComponentKind::PointLight;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.color_row(canvas, x, y, w, e, "Color", ColorTarget::PointLight);
                y = self.float_row(canvas, x, y, w, e, "Intensity", Field::PointIntensity);
                y = self.float_row(canvas, x, y, w, e, "Range", Field::PointRange);
                y = self.float_row(canvas, x, y, w, e, "Radius", Field::PointRadius);
                y = self.flag_row(canvas, x, y, w, e, "Cast Shadows", Flag::PointShadows);
                y += 6.0;
            }
        }
        if d.spot_light.is_some() {
            let kind = ComponentKind::SpotLight;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.color_row(canvas, x, y, w, e, "Color", ColorTarget::SpotLight);
                y = self.float_row(canvas, x, y, w, e, "Intensity", Field::SpotIntensity);
                y = self.float_row(canvas, x, y, w, e, "Range", Field::SpotRange);
                y = self.float_row(canvas, x, y, w, e, "Cone Angle", Field::SpotOuter);
                y = self.float_row(canvas, x, y, w, e, "Inner Angle", Field::SpotInner);
                y = self.float_row(canvas, x, y, w, e, "Radius", Field::SpotRadius);
                y = self.flag_row(canvas, x, y, w, e, "Cast Shadows", Flag::SpotShadows);
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
                y = self.sky_rows(canvas, x, y, w, e);
                y = self.choice_row(canvas, x, y, w, e, "Tone Mapping", Choice::Tonemapping);
                y = self.float_row(canvas, x, y, w, e, "Exposure", Field::EnvExposure);
                y = self.float_row(canvas, x, y, w, e, "Occlusion", Field::EnvAmbientOcclusion);
                y = self.flag_row(canvas, x, y, w, e, "Fog", Flag::EnvFog);
                y = self.color_row(canvas, x, y, w, e, "Fog Color", ColorTarget::EnvFog);
                y = self.float_row(canvas, x, y, w, e, "Fog Density", Field::EnvFogDensity);
                y = self.float_row(canvas, x, y, w, e, "Fog Start", Field::EnvFogStart);
                y = self.atmosphere_rows(canvas, x, y, w, e);
                y = self.choice_row(canvas, x, y, w, e, "Style", Choice::EnvStyle);
                y = self.style_rows(canvas, x, y, w, e);
                y = self.post_rows(canvas, x, y, w, e);
                y += 6.0;
            }
        }
        if d.water.is_some() {
            let kind = ComponentKind::Water;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.float_row(canvas, x, y, w, e, "Width", Field::WaterSize(0));
                y = self.float_row(canvas, x, y, w, e, "Length", Field::WaterSize(1));
                y = self.color_row(canvas, x, y, w, e, "Deep", ColorTarget::WaterDeep);
                y = self.color_row(canvas, x, y, w, e, "Shallow", ColorTarget::WaterShallow);
                y = self.float_row(canvas, x, y, w, e, "Clarity", Field::WaterClarity);
                y = self.float_row(canvas, x, y, w, e, "Wave Height", Field::WaterWaveHeight);
                y = self.float_row(canvas, x, y, w, e, "Wave Length", Field::WaterWaveLength);
                y = self.float_row(canvas, x, y, w, e, "Wave Speed", Field::WaterWaveSpeed);
                y = self.float_row(canvas, x, y, w, e, "Direction", Field::WaterWaveDirection);
                y = self.float_row(canvas, x, y, w, e, "Foam", Field::WaterFoam);
                y = self.float_row(canvas, x, y, w, e, "Density", Field::WaterDensity);
                y = self.float_row(canvas, x, y, w, e, "Drag", Field::WaterDrag);
                y = self.float_row(canvas, x, y, w, e, "Caustics", Field::WaterCaustics);
                y = self.float_row(canvas, x, y, w, e, "Wind", Field::WaterWind);
                y = self.flag_row(canvas, x, y, w, e, "Endless", Flag::WaterEndless);
                y += 6.0;
            }
        }
        if d.grass.is_some() {
            let kind = ComponentKind::Grass;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.brush_rows(canvas, x, y, w);
                y = self.float_row(canvas, x, y, w, e, "Area Width", Field::GrassArea(0));
                y = self.float_row(canvas, x, y, w, e, "Area Depth", Field::GrassArea(1));
                y = self.float_row(canvas, x, y, w, e, "Density", Field::GrassDensity);
                y = self.float_row(canvas, x, y, w, e, "Height", Field::GrassHeight);
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Height Vary",
                    Field::GrassHeightVariation,
                );
                y = self.float_row(canvas, x, y, w, e, "Blade Width", Field::GrassWidth);
                y = self.color_row(canvas, x, y, w, e, "Root Color", ColorTarget::GrassBase);
                y = self.color_row(canvas, x, y, w, e, "Tip Color", ColorTarget::GrassTip);
                y = self.float_row(canvas, x, y, w, e, "Color Vary", Field::GrassColorVariation);
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Draw Distance",
                    Field::GrassDrawDistance,
                );
                y = self.float_row(canvas, x, y, w, e, "Seed", Field::GrassSeed);
                y = self.foliage_note(canvas, x, y, w, e, true);
                y += 6.0;
            }
        }
        if d.scatter.is_some() {
            let kind = ComponentKind::Scatter;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.brush_rows(canvas, x, y, w);
                y = self.float_row(canvas, x, y, w, e, "Area Width", Field::ScatterArea(0));
                y = self.float_row(canvas, x, y, w, e, "Area Depth", Field::ScatterArea(1));
                y = self.float_row(canvas, x, y, w, e, "Density", Field::ScatterDensity);
                y = self.float_row(canvas, x, y, w, e, "Scale Min", Field::ScatterScale(0));
                y = self.float_row(canvas, x, y, w, e, "Scale Max", Field::ScatterScale(1));
                y = self.float_row(canvas, x, y, w, e, "Follow Slope", Field::ScatterAlign);
                y = self.float_row(canvas, x, y, w, e, "Seed", Field::ScatterSeed);
                y = self.foliage_note(canvas, x, y, w, e, false);
                y += 6.0;
            }
        }
        if let Some(emitter) = &d.particle_emitter {
            let kind = ComponentKind::Particles;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                let shape = emitter.shape;
                y = self.choice_row(canvas, x, y, w, e, "Preset", Choice::ParticlePreset);
                y = self.flag_row(canvas, x, y, w, e, "Emitting", Flag::ParticleEmitting);
                y = self.float_row(canvas, x, y, w, e, "Rate", Field::ParticleRate);
                y = self.float_row(canvas, x, y, w, e, "Burst", Field::ParticleBurst);
                y = self.float_row(canvas, x, y, w, e, "Duration", Field::ParticleDuration);
                y = self.flag_row(canvas, x, y, w, e, "One Shot", Flag::ParticleOneShot);
                y = self.float_row(canvas, x, y, w, e, "Life Min", Field::ParticleLifetime(0));
                y = self.float_row(canvas, x, y, w, e, "Life Max", Field::ParticleLifetime(1));
                y = self.choice_row(canvas, x, y, w, e, "Shape", Choice::ParticleShape);
                match shape {
                    automata_scene::EmitterShape::Point => {}
                    automata_scene::EmitterShape::Box => {
                        for (i, label) in ["Half X", "Half Y", "Half Z"].into_iter().enumerate() {
                            #[allow(clippy::cast_possible_truncation)] // 0..3
                            let field = Field::ParticleShapeSize(i as u8);
                            y = self.float_row(canvas, x, y, w, e, label, field);
                        }
                    }
                    _ => {
                        y = self.float_row(
                            canvas,
                            x,
                            y,
                            w,
                            e,
                            "Radius",
                            Field::ParticleShapeSize(0),
                        );
                    }
                }
                y = self.float_row(canvas, x, y, w, e, "Spread", Field::ParticleConeAngle);
                y = self.float_row(canvas, x, y, w, e, "Speed Min", Field::ParticleSpeed(0));
                y = self.float_row(canvas, x, y, w, e, "Speed Max", Field::ParticleSpeed(1));
                y = self.float_row(canvas, x, y, w, e, "Gravity", Field::ParticleGravity);
                y = self.float_row(canvas, x, y, w, e, "Drag", Field::ParticleDrag);
                y = self.float_row(canvas, x, y, w, e, "Start Size", Field::ParticleSize(0));
                y = self.float_row(canvas, x, y, w, e, "End Size", Field::ParticleSize(1));
                y = self.color_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Start Color",
                    ColorTarget::ParticleColor,
                );
                y = self.float_row(canvas, x, y, w, e, "Start Glow", Field::ParticleGlow);
                y = self.color_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "End Color",
                    ColorTarget::ParticleEndColor,
                );
                y = self.float_row(canvas, x, y, w, e, "End Glow", Field::ParticleEndGlow);
                y = self.choice_row(canvas, x, y, w, e, "Sprite", Choice::ParticleSprite);
                y = self.choice_row(canvas, x, y, w, e, "Blend", Choice::ParticleBlend);
                y = self.flag_row(canvas, x, y, w, e, "Lit", Flag::ParticleLit);
                y = self.float_row(canvas, x, y, w, e, "Soft Edges", Field::ParticleSoft);
                y = self.float_row(canvas, x, y, w, e, "Stretch", Field::ParticleStretch);
                y = self.float_row(canvas, x, y, w, e, "Spin", Field::ParticleSpin);
                y = self.float_row(canvas, x, y, w, e, "Max Alive", Field::ParticleMax);
                y = self.flag_row(canvas, x, y, w, e, "World Space", Flag::ParticleWorldSpace);
                y = self.flag_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Follow Camera",
                    Flag::ParticleFollowCamera,
                );
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
                y = self.collider_model_rows(canvas, x, y, w, e);
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

        if d.joint.is_some() {
            let kind = ComponentKind::Joint;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.joint_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }

        if d.attach.is_some() {
            let kind = ComponentKind::Attach;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.attach_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }

        y = self.nav_sections(canvas, doc, x, y, w, e);

        if d.audio_source.is_some() {
            let kind = ComponentKind::AudioSource;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.audio_rows(canvas, doc, x, y, w, e);
                y = self.float_row(canvas, x, y, w, e, "Volume", Field::AudioVolume);
                y = self.float_row(canvas, x, y, w, e, "Pitch", Field::AudioPitch);
                y = self.flag_row(canvas, x, y, w, e, "Loop", Flag::AudioLoop);
                y = self.flag_row(canvas, x, y, w, e, "Play On Start", Flag::AudioPlayOnStart);
                y = self.flag_row(canvas, x, y, w, e, "3D (fade)", Flag::AudioSpatial);
                if e.data.audio_source.as_ref().is_some_and(|a| a.spatial) {
                    y = self.float_row(canvas, x, y, w, e, "Range", Field::AudioRange);
                }
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

    /// The Joint's rows: what it joins, then only the settings its kind
    /// uses (an axis for hinges and sliders, a length for ropes …).
    fn joint_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(joint) = e.data.joint.as_ref() else {
            return y;
        };
        let kind = joint.kind;
        let slider = kind == automata_physics::JointKind::Slider;
        y = self.choice_row(canvas, x, y, w, e, "Type", Choice::JointKind);
        y = self.joint_target_row(canvas, doc, x, y, w, e);
        y = self.vec3_row(canvas, x, y, w, e, "Anchor", Field::JointAnchor);
        if kind.has_axis() {
            y = self.vec3_row(canvas, x, y, w, e, "Axis", Field::JointAxis);
        }
        if kind.has_length() {
            y = self.float_row(canvas, x, y, w, e, "Length", Field::JointLength);
        }
        if kind == automata_physics::JointKind::Spring {
            y = self.float_row(canvas, x, y, w, e, "Stiffness", Field::JointStiffness);
            y = self.float_row(canvas, x, y, w, e, "Damping", Field::JointDamping);
        }
        if kind.has_limits() {
            y = self.flag_row(canvas, x, y, w, e, "Limits", Flag::JointLimits);
            if joint.limits.is_some() {
                let (lo, hi) = if slider {
                    ("Min (m)", "Max (m)")
                } else {
                    ("Min (°)", "Max (°)")
                };
                y = self.float_row(canvas, x, y, w, e, lo, Field::JointLimitMin);
                y = self.float_row(canvas, x, y, w, e, hi, Field::JointLimitMax);
            }
        }
        if kind.has_motor() {
            y = self.flag_row(canvas, x, y, w, e, "Motor", Flag::JointMotor);
            if let Some(motor) = joint.motor {
                y = self.flag_row(canvas, x, y, w, e, "Pull To Target", Flag::JointMotorTarget);
                if motor.target.is_some() {
                    let label = if slider { "Target (m)" } else { "Target (°)" };
                    y = self.float_row(canvas, x, y, w, e, label, Field::JointMotorTarget);
                } else {
                    let label = if slider {
                        "Speed (m/s)"
                    } else {
                        "Speed (°/s)"
                    };
                    y = self.float_row(canvas, x, y, w, e, label, Field::JointMotorSpeed);
                }
                y = self.float_row(canvas, x, y, w, e, "Max Force", Field::JointMotorMaxForce);
            }
        }
        y = self.float_row(canvas, x, y, w, e, "Break Force", Field::JointBreakForce);
        y = self.flag_row(canvas, x, y, w, e, "Bodies Collide", Flag::JointCollide);
        y = self.flag_row(canvas, x, y, w, e, "Enabled", Flag::JointEnabled);
        let hint = if joint.break_force > 0.0 {
            "Snaps past Break Force (newtons)."
        } else {
            "Break Force 0 never snaps."
        };
        if self.row_visible(Rect::new(x, y, w, 16.0)) {
            canvas.text(x, y, layout::SMALL_SIZE, hint, color::text_dim());
        }
        y + 16.0
    }

    /// Attach To Bone: the entity followed and which of its bones.
    /// The Nav Agent, Navigation and Nav Link sections.
    fn nav_sections(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let id = e.id;
        if e.data.nav_agent.is_some() {
            let kind = ComponentKind::NavAgent;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), true);
            y = ny;
            if open {
                y = self.float_row(canvas, x, y, w, e, "Speed", Field::Nav(NavField::Speed));
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Acceleration",
                    Field::Nav(NavField::Acceleration),
                );
                y = self.float_row(canvas, x, y, w, e, "Radius", Field::Nav(NavField::Radius));
                if e.data.character_controller.is_none() {
                    y = self.float_row(canvas, x, y, w, e, "Hover", Field::Nav(NavField::Hover));
                }
                y = self.float_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Stop Distance",
                    Field::Nav(NavField::StopDistance),
                );
                y = self.flag_row(canvas, x, y, w, e, "Avoid Others", Flag::NavAvoid);
                y = self.flag_row(canvas, x, y, w, e, "Face Movement", Flag::NavTurn);
                y = self.dim_lines(canvas, x, y, w, &["Scripts: entity.nav.go(target)."]);
                y += 6.0;
            }
        }
        if e.data.navigation.is_some() {
            let kind = ComponentKind::Navigation;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), true);
            y = ny;
            if open {
                y = self.navigation_rows(canvas, doc, x, y, w, e);
                y += 6.0;
            }
        }
        if let Some(link) = e.data.nav_link.clone() {
            let kind = ComponentKind::NavLink;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                let found = doc
                    .entities
                    .iter()
                    .any(|o| o.id != id && o.name() == link.to);
                let text = match () {
                    () if link.to.is_empty() => ("Pick an entity".to_string(), true),
                    () if !found => (format!("{} (missing)", link.to), true),
                    () => (link.to.clone(), false),
                };
                let items = || {
                    let mut names: Vec<&str> = doc
                        .entities
                        .iter()
                        .filter(|o| o.id != id && o.data.name.is_some())
                        .map(EntityEntry::name)
                        .collect();
                    names.sort_unstable();
                    names.dedup();
                    names
                        .into_iter()
                        .map(|n| menus::MenuItem {
                            label: if n == link.to {
                                format!("✓ {n}")
                            } else {
                                format!("   {n}")
                            },
                            shortcut: "",
                            action: Some(menus::Action::Intent(Intent::SetNavLink {
                                id,
                                to: n.to_string(),
                            })),
                            enabled: true,
                        })
                        .collect()
                };
                y = self.menu_row(canvas, x, y, w, "To", (&text.0, text.1), items);
                y = self.flag_row(canvas, x, y, w, e, "Both Ways", Flag::NavLinkTwoWay);
                y += 6.0;
            }
        }
        y
    }

    /// Dim help lines under a section's rows.
    fn dim_lines(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        mut y: f32,
        w: f32,
        lines: &[&str],
    ) -> f32 {
        for line in lines {
            if self.row_visible(Rect::new(x, y, w, 16.0)) {
                canvas.text(x, y, layout::SMALL_SIZE, line, color::text_dim());
            }
            y += 16.0;
        }
        y
    }

    /// The Navigation section: bake settings, Bake and Show Mesh.
    /// Loads the scene's saved navigation mesh for the overlay, once per
    /// file (a Bake replaces it directly).
    fn refresh_nav_overlay(&mut self, doc: &Document) {
        let Some(file) = doc
            .entities
            .iter()
            .find_map(|e| e.data.navigation.as_ref())
            .map(|n| n.file.clone())
            .filter(|f| !f.is_empty())
        else {
            return;
        };
        let path = doc.resolve(&file);
        if self.nav_overlay.as_ref().is_none_or(|(p, _)| *p != path)
            && self.nav_missing.as_ref() != Some(&path)
        {
            match automata_nav::NavMesh::load(&path) {
                Ok(mesh) => self.nav_overlay = Some((path, std::sync::Arc::new(mesh))),
                // Don't try the disk again every frame.
                Err(_) => self.nav_missing = Some(path),
            }
        }
    }

    fn navigation_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(nav) = e.data.navigation.clone() else {
            return y;
        };
        for (label, f) in [
            ("Agent Radius", NavBakeField::Radius),
            ("Agent Height", NavBakeField::Height),
            ("Max Climb", NavBakeField::Climb),
            ("Max Slope", NavBakeField::Slope),
            ("Jump Distance", NavBakeField::Jump),
            ("Jump Height", NavBakeField::JumpHeight),
            ("Drop Height", NavBakeField::Drop),
            ("Cell Size", NavBakeField::Cell),
        ] {
            y = self.float_row(canvas, x, y, w, e, label, Field::NavBake(f));
        }
        self.refresh_nav_overlay(doc);
        y += 4.0;
        let half = (w - 6.0) * 0.5;
        let bake = Rect::new(x, y, half, 26.0);
        let show = Rect::new(x + half + 6.0, y, half, 26.0);
        if self.row_visible(bake) {
            let style = if self.play == PlayState::Editing {
                ButtonStyle::Accent
            } else {
                ButtonStyle::Disabled
            };
            self.tip(
                bake,
                "Work out where agents can walk from the static colliders, and save it \
                 next to the scene",
            );
            if self.button(canvas, bake, "Bake", style) && style != ButtonStyle::Disabled {
                self.out.push(Intent::BakeNav(e.id));
            }
            let label = if self.show_nav {
                "Hide Mesh"
            } else {
                "Show Mesh"
            };
            if self.button(canvas, show, label, ButtonStyle::Normal) {
                self.show_nav = !self.show_nav;
            }
        }
        y += 32.0;
        let status = match &self.nav_overlay {
            _ if nav.file.is_empty() => "Not baked yet (bakes when the game starts).".to_string(),
            Some((path, mesh)) if *path == doc.resolve(&nav.file) => format!(
                "{}: {:.0} m², {} areas, {} links",
                nav.file,
                mesh.area(),
                mesh.regions,
                mesh.links.len()
            ),
            _ => format!("{} (missing: bake again)", nav.file),
        };
        self.dim_lines(canvas, x, y, w, &[&status])
    }

    fn attach_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(attach) = e.data.attach.clone() else {
            return y;
        };
        let target = doc
            .entities
            .iter()
            .find(|o| o.id != e.id && !attach.to.is_empty() && o.name() == attach.to);
        // Follow: every other named entity.
        let (text, warn) = match target {
            _ if attach.to.is_empty() => ("Pick an entity".to_string(), true),
            None => (format!("{} (missing)", attach.to), true),
            Some(_) => (attach.to.clone(), false),
        };
        let items = || {
            let mut names: Vec<&str> = doc
                .entities
                .iter()
                .filter(|o| o.id != e.id && o.data.name.is_some())
                .map(EntityEntry::name)
                .collect();
            names.sort_unstable();
            names.dedup();
            names
                .into_iter()
                .map(|n| menus::MenuItem {
                    label: if n == attach.to {
                        format!("✓ {n}")
                    } else {
                        format!("   {n}")
                    },
                    shortcut: "",
                    action: Some(menus::Action::Intent(Intent::SetAttach {
                        id: e.id,
                        to: Some(n.to_string()),
                        bone: None,
                    })),
                    enabled: true,
                })
                .collect()
        };
        y = self.menu_row(canvas, x, y, w, "Follow", (&text, warn), items);
        // Bone: the followed model's bones, nested.
        let rig = target.and_then(|t| crate::doc::model_rig(&t.data));
        let known =
            attach.bone.is_empty() || rig.as_ref().is_some_and(|r| r.node(&attach.bone).is_some());
        let text = match (&rig, attach.bone.is_empty()) {
            (_, true) => "None (its origin)".to_string(),
            (Some(_), false) if known => attach.bone.clone(),
            (_, false) => format!("{} (missing)", attach.bone),
        };
        let items = || {
            let Some(rig) = &rig else {
                return Vec::new();
            };
            let depth = |mut i: usize| {
                let mut d = 0;
                while let Some(p) = rig.nodes[i].parent {
                    d += 1;
                    i = p;
                }
                d
            };
            let item = |label: String, bone: &str| menus::MenuItem {
                label: if bone == attach.bone {
                    format!("✓ {label}")
                } else {
                    format!("   {label}")
                },
                shortcut: "",
                action: Some(menus::Action::Intent(Intent::SetAttach {
                    id: e.id,
                    to: None,
                    bone: Some(bone.to_string()),
                })),
                enabled: true,
            };
            let mut out = vec![
                item("None (its origin)".into(), ""),
                menus::MenuItem::separator(),
            ];
            out.extend(rig.nodes.iter().enumerate().map(|(i, n)| {
                let indent = "  ".repeat(depth(i).min(12));
                item(format!("{indent}{}", n.name), &n.name)
            }));
            out
        };
        y = self.menu_row(canvas, x, y, w, "Bone", (&text, !known), items);
        let hint = "Its Transform is now its offset from the bone";
        if self.row_visible(Rect::new(x, y, w, 16.0)) {
            canvas.text(x, y, layout::SMALL_SIZE, hint, color::text_dim());
        }
        y + 16.0
    }

    /// A row showing `text` (in the warning colour when `warn`) that opens
    /// the menu `items` makes when clicked.
    #[allow(clippy::too_many_arguments)]
    fn menu_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        (text, warn): (&str, bool),
        items: impl FnOnce() -> Vec<menus::MenuItem>,
    ) -> f32 {
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
            let tint = if warn { color::warn() } else { color::text() };
            canvas.text_clipped(
                b.x + 6.0,
                b.y + 3.0,
                layout::FONT_SIZE,
                text,
                tint,
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
                    let items = items();
                    if !items.is_empty() {
                        self.open_menu(b.x, b.y + b.h + 2.0, items);
                    }
                }
            }
        }
        y + layout::ROW_STEP
    }

    /// "Connected To": a menu of World and every other named entity.
    fn joint_target_row(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(joint) = e.data.joint.as_ref() else {
            return y;
        };
        let current = joint.connected.clone();
        let missing = current
            .as_deref()
            .is_some_and(|n| !doc.entities.iter().any(|o| o.id != e.id && o.name() == n));
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, "Connected To");
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
            let text = match &current {
                None => "World".to_string(),
                Some(n) if missing => format!("{n} (missing)"),
                Some(n) => n.clone(),
            };
            canvas.text_clipped(
                b.x + 6.0,
                b.y + 3.0,
                layout::FONT_SIZE,
                &text,
                if missing {
                    color::warn()
                } else {
                    color::text()
                },
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
                    let item = |label: String, target: Option<String>| menus::MenuItem {
                        label: if target == current {
                            format!("✓ {label}")
                        } else {
                            format!("   {label}")
                        },
                        shortcut: "",
                        action: Some(menus::Action::Intent(Intent::SetJointTarget {
                            id: e.id,
                            target,
                        })),
                        enabled: true,
                    };
                    let mut items = vec![item("World".into(), None)];
                    let mut names: Vec<&str> = doc
                        .entities
                        .iter()
                        .filter(|o| o.id != e.id && o.data.name.is_some())
                        .map(EntityEntry::name)
                        .collect();
                    names.sort_unstable();
                    names.dedup();
                    if !names.is_empty() {
                        items.push(menus::MenuItem::separator());
                    }
                    items.extend(
                        names
                            .into_iter()
                            .map(|n| item(n.to_string(), Some(n.to_string()))),
                    );
                    self.open_menu(b.x, b.y + b.h + 2.0, items);
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
                        .collect::<Vec<_>>();
                    let mut items = items;
                    if choice == Choice::MeshPrimitive {
                        items.push(menus::MenuItem::separator());
                        items.push(menus::MenuItem {
                            label: "   Model…".into(),
                            shortcut: "",
                            action: Some(menus::Action::Ui(menus::UiAction::PickModel(Some(e.id)))),
                            enabled: true,
                        });
                    }
                    self.open_menu(b.x, b.y + b.h + 2.0, items);
                }
            }
        }
        y + layout::ROW_STEP
    }

    /// The Animation's rows: its clip (a menu of the model's), speed, loop
    /// and play-on-start, and a preview that plays or scrubs the clip in
    /// the scene view without pressing Play.
    fn animation_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(anim) = e.data.animation.as_ref() else {
            return y;
        };
        let rig = crate::doc::model_rig(&e.data).filter(|r| !r.clips.is_empty());
        let editing = self.play == PlayState::Editing;
        let clip = rig.as_ref().and_then(|r| r.clip(&anim.clip));
        let b = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
        if self.row_visible(b) {
            self.row_label(canvas, x, y, "Clip");
            let hot = self.hot(b) && rig.is_some();
            canvas.fill_rounded(
                b.x,
                b.y,
                b.w,
                b.h,
                3.0,
                if hot { color::hover() } else { color::raised() },
            );
            let (shown, tint) = match (&rig, clip) {
                (None, _) => ("Needs an animated model", color::danger()),
                (Some(_), None) if !anim.clip.is_empty() => (anim.clip.as_str(), color::danger()),
                (Some(_), None) => ("None (rest pose)", color::text_dim()),
                (Some(_), Some(c)) => (c.name.as_str(), color::text()),
            };
            canvas.text_clipped(
                b.x + 6.0,
                b.y + 3.0,
                layout::FONT_SIZE,
                shown,
                tint,
                b.w - 24.0,
            );
            let (cx, cy) = (b.x + b.w - 11.0, b.y + b.h / 2.0);
            canvas.fill_polygon(
                &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
                color::text_dim(),
            );
            if rig.is_none() {
                self.tip(
                    b,
                    "Pick a model with animations in the Mesh Renderer (a .glb or .gltf)",
                );
            }
            if let (true, Some(rig)) = (hot, &rig) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed && editing {
                    let names = std::iter::once(("", "None"))
                        .chain(rig.clips.iter().map(|c| (c.name.as_str(), c.name.as_str())));
                    let items = names
                        .map(|(name, label)| menus::MenuItem {
                            label: if name == anim.clip {
                                format!("✓ {label}")
                            } else {
                                format!("   {label}")
                            },
                            shortcut: "",
                            action: Some(menus::Action::Intent(Intent::SetAnimationClip {
                                id: e.id,
                                clip: name.to_string(),
                            })),
                            enabled: true,
                        })
                        .collect();
                    self.open_menu(b.x, b.y + b.h + 2.0, items);
                }
            }
        }
        y += layout::ROW_STEP;
        y = self.float_row(canvas, x, y, w, e, "Speed", Field::AnimSpeed);
        y = self.flag_row(canvas, x, y, w, e, "Loop", Flag::AnimLoop);
        y = self.flag_row(canvas, x, y, w, e, "Play On Start", Flag::AnimPlayOnStart);
        let root = Rect::new(x, y, w, layout::FIELD_H);
        y = self.flag_row(canvas, x, y, w, e, "Root Motion", Flag::AnimRootMotion);
        if self.row_visible(root) {
            self.tip(
                root,
                "Let the clip's own steps move the entity (through its Character Controller, if it has one)",
            );
        }
        let feet = Rect::new(x, y, w, layout::FIELD_H);
        y = self.flag_row(canvas, x, y, w, e, "Feet On Ground", Flag::AnimFootIk);
        if self.row_visible(feet) {
            self.tip(
                feet,
                "Plant each foot on what's under it (slopes, stairs), dropping the hips for the lower one",
            );
        }
        y = self.anim_graph_row(canvas, doc, x, y, w, e);
        if let (true, Some(clip)) = (editing, clip) {
            y = self.anim_preview_row(canvas, x, y, w, e.id, clip);
        }
        y
    }

    /// The Animation's Graph: the `.anim.ron` file picking its clips from
    /// script parameters. Click to choose or make one, double-click (or
    /// Edit) to open it, × to play the clip again.
    fn anim_graph_row(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(anim) = e.data.animation.as_ref() else {
            return y;
        };
        let rect = Rect::new(x, y, w, layout::FIELD_H);
        if !self.row_visible(rect) {
            return y + layout::ROW_STEP;
        }
        let editing = self.play == PlayState::Editing;
        let rel = anim.graph.trim();
        let file = (!rel.is_empty()).then(|| doc.resolve(rel));
        self.row_label(canvas, x, y, "Graph");
        let extra = if rel.is_empty() { 0.0 } else { 40.0 + 26.0 };
        let r = Rect::new(
            x + layout::LABEL_W,
            y,
            w - layout::LABEL_W - extra,
            layout::FIELD_H,
        );
        let hot = editing && self.hot(r);
        canvas.fill_rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            3.0,
            if hot { color::hover() } else { color::field() },
        );
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::border(), 1.0);
        let (text, tint) = if rel.is_empty() {
            ("None · plays the clip".to_string(), color::text_faint())
        } else if file.as_ref().is_some_and(|f| f.is_file())
            && automata_assets::load_anim_graph(rel).is_ok()
        {
            (rel.to_string(), color::accent_soft())
        } else {
            (rel.to_string(), color::danger())
        };
        canvas.text_clipped(
            r.x + 6.0,
            r.y + 3.0,
            layout::FONT_SIZE,
            &text,
            tint,
            r.w - 12.0,
        );
        self.tip(
            r,
            "An animation graph (.anim.ron) picks the clips from parameters your script \
             sets · click to choose or make one · double-click to edit",
        );
        if hot {
            self.cursor = CursorHint::Pointer;
            if self.p.double_click {
                if let Some(f) = file.as_ref().filter(|f| f.exists()) {
                    self.out.push(Intent::OpenScript(f.clone()));
                }
            } else if self.p.pressed {
                self.open_file_dialog(menus::DialogKind::ChooseAnimGraph(e.id), Some(doc));
            }
        }
        if !rel.is_empty() {
            let style = if editing {
                ButtonStyle::Ghost
            } else {
                ButtonStyle::Disabled
            };
            let edit = Rect::new(r.x + r.w + 2.0, y, 38.0, layout::FIELD_H);
            if self.button(canvas, edit, "Edit", style) {
                if let Some(f) = file.as_ref().filter(|f| f.exists()) {
                    self.out.push(Intent::OpenScript(f.clone()));
                }
            }
            self.tip(edit, "Edit the graph");
            let clear = Rect::new(edit.x + 40.0, y, 24.0, layout::FIELD_H);
            if self.button(canvas, clear, "×", style) {
                self.out.push(Intent::SetAnimGraph {
                    id: e.id,
                    path: None,
                });
            }
            self.tip(clear, "Play the clip instead");
        }
        y + layout::ROW_STEP
    }

    /// Play/pause and a time bar for the Animation preview, with a tick
    /// for each of the clip's events (hover one for its name).
    fn anim_preview_row(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        id: u64,
        clip: &automata_assets::Clip,
    ) -> f32 {
        let duration = clip.duration;
        let mut preview = self
            .anim_preview
            .filter(|p| p.id == id)
            .unwrap_or(AnimPreview {
                id,
                playing: false,
                time: 0.0,
            });
        let button = Rect::new(x + layout::LABEL_W, y + 1.0, 18.0, 18.0);
        let bar = Rect::new(
            button.x + 24.0,
            y + 1.0,
            (w - layout::LABEL_W - 24.0).max(10.0),
            18.0,
        );
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "Preview");
            let hot = self.hot(button);
            canvas.fill_rounded(
                button.x,
                button.y,
                button.w,
                button.h,
                3.0,
                if preview.playing {
                    color::accent()
                } else if hot {
                    color::hover()
                } else {
                    color::raised()
                },
            );
            let ink = if preview.playing {
                color::on_accent()
            } else {
                color::text()
            };
            let (bx, by) = (button.x, button.y);
            if preview.playing {
                canvas.fill_rect(bx + 5.0, by + 4.5, 3.0, 9.0, ink);
                canvas.fill_rect(bx + 10.0, by + 4.5, 3.0, 9.0, ink);
            } else {
                canvas.fill_polygon(
                    &[
                        (bx + 6.0, by + 4.0),
                        (bx + 6.0, by + 14.0),
                        (bx + 14.0, by + 9.0),
                    ],
                    ink,
                );
            }
            self.tip(
                button,
                "Play the clip in the scene view (no need to press Play)",
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    preview.playing = !preview.playing;
                    if preview.playing && preview.time >= duration {
                        preview.time = 0.0;
                    }
                    self.anim_preview = Some(preview);
                }
            }
            // The time bar: filled up to the playhead; drag to scrub.
            canvas.fill_rounded(bar.x, bar.y, bar.w, bar.h, 3.0, color::field());
            let share = if duration > 0.0 {
                (preview.time / duration).clamp(0.0, 1.0)
            } else {
                0.0
            };
            canvas.fill_rounded(
                bar.x,
                bar.y,
                (bar.w * share).max(3.0),
                bar.h,
                3.0,
                color::accent_soft(),
            );
            canvas.text_clipped(
                bar.x + 6.0,
                bar.y + 2.0,
                layout::SMALL_SIZE,
                &format!("{:.2} / {:.2} s", preview.time, duration),
                color::text(),
                bar.w - 8.0,
            );
            if duration > 0.0 {
                for event in &clip.events {
                    let ex = bar.x + bar.w * (event.time / duration).clamp(0.0, 1.0);
                    let ex = ex.clamp(bar.x + 1.0, bar.x + bar.w - 3.0);
                    canvas.fill_rect(ex, bar.y + bar.h - 6.0, 2.0, 6.0, color::warn());
                    let near = Rect::new(ex - 3.0, bar.y + bar.h - 7.0, 8.0, 7.0);
                    if self.hot(near) {
                        self.tip(near, &format!("{} at {:.2} s", event.name, event.time));
                    }
                }
            }
            if self.hot(bar) {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    self.anim_scrub = true;
                }
            }
        }
        if self.anim_scrub {
            if self.p.down {
                preview.playing = false;
                preview.time = ((self.p.x - bar.x) / bar.w).clamp(0.0, 1.0) * duration;
                self.anim_preview = Some(preview);
            } else {
                self.anim_scrub = false;
            }
        }
        y + layout::ROW_STEP
    }

    /// A Mesh / Convex Hull collider's model (read-only: Fit Collider or
    /// a mesh swap updates it) and anything wrong with the collider.
    fn collider_model_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(c) = &e.data.collider_3d else {
            return y;
        };
        if let Some(model) = c.shape.model() {
            if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
                self.row_label(canvas, x, y, "Model");
                let r = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
                canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::field());
                canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::border(), 1.0);
                let tint = if automata_assets::load(model).is_ok() {
                    color::text()
                } else {
                    color::danger()
                };
                canvas.text_clipped(
                    r.x + 6.0,
                    r.y + 3.0,
                    layout::FONT_SIZE,
                    model,
                    tint,
                    r.w - 12.0,
                );
                self.tip(
                    r,
                    "The shape is built from this model at the entity's scale",
                );
            }
            y += layout::ROW_STEP;
        }
        let body = e.data.rigid_body.as_ref().map(|b| b.body_type);
        if let Some(problem) = automata_physics::collider_problem(&c.shape, body) {
            for chunk in wrap_words(canvas, problem, w - layout::LABEL_W) {
                if self.row_visible(Rect::new(x, y, w, 16.0)) {
                    canvas.text(
                        x + layout::LABEL_W,
                        y,
                        layout::SMALL_SIZE,
                        &chunk,
                        color::danger(),
                    );
                }
                y += 16.0;
            }
            y += 4.0;
        }
        y
    }

    /// The Mesh section's model rows (file, summary, actions) when the mesh
    /// is a model file. Returns the y after them.
    fn model_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(automata_core::PrimitiveData::Model { path: rel }) =
            e.data.mesh_visual.as_ref().map(|m| &m.primitive)
        else {
            return y;
        };
        let editing = self.play == PlayState::Editing;
        let loaded = automata_assets::load(rel);
        let file = doc.resolve(rel);
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "File");
            let r = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
            canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::field());
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::border(), 1.0);
            canvas.text_clipped(
                r.x + 6.0,
                r.y + 3.0,
                layout::FONT_SIZE,
                rel,
                if loaded.is_ok() {
                    color::accent_soft()
                } else {
                    color::danger()
                },
                r.w - 12.0,
            );
            self.tip(
                r,
                "Double-click to show it in Files · drop a model here to swap",
            );
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
                if self.p.double_click && file.exists() {
                    self.reveal_in_files(&file);
                }
            }
        }
        y += layout::ROW_STEP;
        // What was loaded, or why it wasn't.
        let (line, tint) = match &loaded {
            Ok(model) => (crate::models::describe(model), color::text_dim()),
            Err(automata_assets::ModelError::NotFound(_)) => {
                ("File not found".to_string(), color::danger())
            }
            Err(err) => (err.to_string(), color::danger()),
        };
        let label_w = w - layout::LABEL_W;
        for chunk in wrap_words(canvas, &line, label_w).into_iter().take(3) {
            if self.row_visible(Rect::new(x, y, w, 16.0)) {
                canvas.text(x + layout::LABEL_W, y, layout::SMALL_SIZE, &chunk, tint);
            }
            y += 16.0;
        }
        y += 4.0;
        if let Ok(model) = &loaded {
            y = self.import_rows(canvas, x, y, w, e.id, &file, editing);
            if let Some(rig) = model.rig.as_ref().filter(|r| !r.joints.is_empty()) {
                y = self.clip_files_rows(canvas, x, y, w, &file, rig, editing);
                y = self.springs_rows(canvas, x, y, w, e.id, &file, rig, editing);
            }
        }
        let bw = (label_w - 6.0) / 2.0;
        let change = Rect::new(x + layout::LABEL_W, y, bw, 22.0);
        if self.row_visible(change) {
            let style = if editing {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, change, "Change…", style) && editing {
                self.open_model_picker(Some(e.id));
            }
            let fit = Rect::new(change.x + bw + 6.0, y, bw, 22.0);
            let has_collider = e.data.collider_3d.is_some();
            let ok = editing && loaded.is_ok();
            let style = if ok {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            let label = if has_collider {
                "Fit Collider"
            } else {
                "Add Collider"
            };
            self.tip(
                fit,
                if has_collider {
                    "Refit the collider to the model (a Mesh or Convex Hull takes the new model)"
                } else {
                    "Add a collider: a box if the model is boxy, else its triangles (static) \
                     or convex hull (moving)"
                },
            );
            if self.button(canvas, fit, label, style) && ok {
                self.out.push(if has_collider {
                    Intent::FitCollider(e.id)
                } else {
                    Intent::AddComponent(e.id, ComponentKind::Collider)
                });
            }
        }
        y + 28.0
    }

    /// A model file's import settings: scale, rotation and pivot, baked
    /// into the file's geometry for every entity that uses it.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn import_rows(
        &mut self,
        canvas: &mut Canvas,
        x: f32,
        mut y: f32,
        w: f32,
        id: u64,
        file: &Path,
        editing: bool,
    ) -> f32 {
        use automata_assets::import::{read_settings, ImportSettings, Pivot};
        let settings = read_settings(file);
        let fx = x + layout::LABEL_W;
        let fw = w - layout::LABEL_W;
        if self.row_visible(Rect::new(x, y, w, 18.0)) {
            canvas.text(x, y + 2.0, layout::SMALL_SIZE, "Import", color::text_dim());
            if !settings.is_identity() {
                let reset = Rect::new(x + w - 48.0, y, 48.0, 18.0);
                let style = if editing {
                    ButtonStyle::Ghost
                } else {
                    ButtonStyle::Disabled
                };
                self.tip(reset, "Back to the file as it is");
                if self.button(canvas, reset, "Reset", style) && editing {
                    self.out.push(Intent::SetImport {
                        model: file.to_path_buf(),
                        settings: ImportSettings::default(),
                    });
                }
            }
            let hint = Rect::new(x, y, w - 52.0, 18.0);
            self.tip(
                hint,
                "Fixes applied to the model file itself: every entity using it updates",
            );
        }
        y += 22.0;
        // Scale, with quick unit fixes.
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "Scale");
            let quick = [("×0.01", 0.01), ("×10", 10.0)];
            let qw = 44.0;
            let field_w = (fw - (qw + 4.0) * quick.len() as f32).max(40.0);
            let r = Rect::new(fx, y, field_w, layout::FIELD_H);
            let scale = springs::ImportField::Scale;
            self.import_field(
                canvas,
                r,
                id,
                scale,
                settings.scale,
                file,
                &settings,
                editing,
            );
            for (i, (label, k)) in quick.iter().enumerate() {
                let b = Rect::new(
                    fx + field_w + 4.0 + (qw + 4.0) * i as f32,
                    y,
                    qw,
                    layout::FIELD_H,
                );
                let style = if editing {
                    ButtonStyle::Normal
                } else {
                    ButtonStyle::Disabled
                };
                self.tip(b, "Multiply the scale (centimetres ↔ metres, tiny kits)");
                if self.button(canvas, b, label, style) && editing {
                    let mut s = settings.clone();
                    s.scale *= k;
                    self.out.push(Intent::SetImport {
                        model: file.to_path_buf(),
                        settings: s,
                    });
                }
            }
        }
        y += layout::ROW_STEP;
        // Rotation: X Y Z, and a quarter turn for models facing away.
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "Rotation");
            let turn_w = 44.0;
            let cw = ((fw - turn_w - 4.0 - 8.0) / 3.0).max(24.0);
            for i in 0..3u8 {
                let r = Rect::new(fx + (cw + 4.0) * f32::from(i), y, cw, layout::FIELD_H);
                let v = settings.rotation[usize::from(i)];
                let which = springs::ImportField::Rotation(i);
                self.import_field(canvas, r, id, which, v, file, &settings, editing);
            }
            let b = Rect::new(fx + fw - turn_w, y, turn_w, layout::FIELD_H);
            let style = if editing {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            self.tip(b, "Turn 90° around Y (models facing the wrong way)");
            if self.button(canvas, b, "+90°", style) && editing {
                let mut s = settings.clone();
                s.rotation[1] = (s.rotation[1] + 90.0 + 180.0).rem_euclid(360.0) - 180.0;
                self.out.push(Intent::SetImport {
                    model: file.to_path_buf(),
                    settings: s,
                });
            }
        }
        y += layout::ROW_STEP;
        // Pivot: one button per option.
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "Pivot");
            let n = Pivot::ALL.len() as f32;
            let bw = (fw - 4.0 * (n - 1.0)) / n;
            for (i, pivot) in Pivot::ALL.into_iter().enumerate() {
                let b = Rect::new(fx + (bw + 4.0) * i as f32, y, bw, layout::FIELD_H);
                let style = if !editing {
                    ButtonStyle::Disabled
                } else if settings.pivot == pivot {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Ghost
                };
                let label = match pivot {
                    Pivot::File => "File",
                    Pivot::BottomCenter => "Bottom",
                    Pivot::Center => "Centre",
                };
                self.tip(b, pivot.label());
                if self.button(canvas, b, label, style) && editing && settings.pivot != pivot {
                    let mut s = settings.clone();
                    s.pivot = pivot;
                    self.out.push(Intent::SetImport {
                        model: file.to_path_buf(),
                        settings: s,
                    });
                }
            }
        }
        y + layout::ROW_STEP + 4.0
    }

    /// One number of a model's import settings; click to type.
    #[allow(clippy::too_many_arguments)]
    fn import_field(
        &mut self,
        canvas: &mut Canvas,
        r: Rect,
        id: u64,
        which: springs::ImportField,
        value: f32,
        file: &Path,
        settings: &automata_assets::import::ImportSettings,
        editing: bool,
    ) {
        let active = self.is_editing(id, EditTarget::Import(which))
            && self.import_edit.as_ref().is_some_and(|(f, _)| f == file);
        let hot = self.hot(r);
        let bg = if active {
            color::viewport_bg()
        } else if hot && editing {
            color::hover()
        } else {
            color::field()
        };
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, bg);
        if active {
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::accent(), 1.0);
            self.draw_edit_text(canvas, r, r.x + 5.0);
            self.cursor = CursorHint::Text;
            return;
        }
        canvas.text_clipped(
            r.x + 5.0,
            r.y + (r.h - layout::FONT_SIZE) / 2.0 - 1.0,
            layout::FONT_SIZE,
            &fields::format_value(value),
            color::text(),
            r.w - 8.0,
        );
        if hot && editing {
            self.cursor = CursorHint::Text;
            if self.p.pressed {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
                self.import_edit = Some((file.to_path_buf(), settings.clone()));
                self.edit = Some(TextEdit {
                    id,
                    target: EditTarget::Import(which),
                    buffer: fields::format_value(value),
                    rect: r,
                    fresh: true,
                    attr: None,
                });
            }
        }
    }

    /// The Audio Source's clip: file, length and waveform, and buttons to
    /// preview or change it.
    fn audio_rows(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        x: f32,
        mut y: f32,
        w: f32,
        e: &EntityEntry,
    ) -> f32 {
        let Some(source) = e.data.audio_source.as_ref() else {
            return y;
        };
        let editing = self.play == PlayState::Editing;
        let rel = source.clip.clone();
        let file = doc.resolve(&rel);
        let info = if rel.is_empty() {
            Err("No clip: drop a sound here or choose one".to_string())
        } else {
            self.clips.get(&file).clone()
        };
        if self.row_visible(Rect::new(x, y, w, layout::FIELD_H)) {
            self.row_label(canvas, x, y, "Clip");
            let r = Rect::new(x + layout::LABEL_W, y, w - layout::LABEL_W, layout::FIELD_H);
            canvas.fill_rounded(r.x, r.y, r.w, r.h, 3.0, color::field());
            canvas.stroke_rounded(r.x, r.y, r.w, r.h, 3.0, color::border(), 1.0);
            let shown = if rel.is_empty() { "None" } else { rel.as_str() };
            canvas.text_clipped(
                r.x + 6.0,
                r.y + 3.0,
                layout::FONT_SIZE,
                shown,
                if info.is_ok() {
                    color::accent_soft()
                } else {
                    color::danger()
                },
                r.w - 12.0,
            );
            self.tip(
                r,
                "Double-click to show it in Files · drop a sound here to swap",
            );
            if self.hot(r) {
                self.cursor = CursorHint::Pointer;
                if self.p.double_click && file.exists() {
                    self.reveal_in_files(&file);
                }
            }
        }
        y += layout::ROW_STEP;
        let label_w = w - layout::LABEL_W;
        match &info {
            Ok(clip) => {
                // The waveform: one bar per slice, mirrored about the middle.
                let wave = Rect::new(x + layout::LABEL_W, y, label_w, 28.0);
                if self.row_visible(wave) {
                    canvas.fill_rounded(wave.x, wave.y, wave.w, wave.h, 3.0, color::field());
                    let n = clip.peaks.len().max(1);
                    #[allow(clippy::cast_precision_loss)]
                    let step = (wave.w - 8.0) / n as f32;
                    let mid = wave.y + wave.h / 2.0;
                    for (i, peak) in clip.peaks.iter().enumerate() {
                        #[allow(clippy::cast_precision_loss)]
                        let bx = wave.x + 4.0 + i as f32 * step;
                        let h = (peak * (wave.h - 8.0)).max(1.0);
                        canvas.fill_rect(
                            bx,
                            mid - h / 2.0,
                            (step - 0.6).max(0.6),
                            h,
                            color::accent_soft(),
                        );
                    }
                }
                y += 32.0;
                if self.row_visible(Rect::new(x, y, w, 16.0)) {
                    canvas.text(
                        x + layout::LABEL_W,
                        y,
                        layout::SMALL_SIZE,
                        &crate::sounds::describe(clip),
                        color::text_dim(),
                    );
                }
                y += 16.0;
            }
            Err(err) => {
                for chunk in wrap_words(canvas, err, label_w).into_iter().take(3) {
                    if self.row_visible(Rect::new(x, y, w, 16.0)) {
                        canvas.text(
                            x + layout::LABEL_W,
                            y,
                            layout::SMALL_SIZE,
                            &chunk,
                            color::danger(),
                        );
                    }
                    y += 16.0;
                }
            }
        }
        y += 4.0;
        let bw = (label_w - 6.0) / 2.0;
        let preview = Rect::new(x + layout::LABEL_W, y, bw, 22.0);
        if self.row_visible(preview) {
            let ok = info.is_ok();
            let style = if ok {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            self.tip(preview, "Hear the clip (click again to stop)");
            if self.button(canvas, preview, "Preview", style) && ok {
                self.out.push(Intent::PreviewSound(file.clone()));
            }
            let change = Rect::new(preview.x + bw + 6.0, y, bw, 22.0);
            let style = if editing {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(canvas, change, "Change…", style) && editing {
                self.open_sound_picker(Some(e.id));
            }
        }
        y + 28.0
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
        let image = frame.map(|(_, fw, fh)| (fw, fh)).or(self.composited_view);
        if let Some((fw, fh)) = image {
            let s = canvas.scale();
            let (lw, lh) = (fw as f32 / s, fh as f32 / s);
            let dx = rect.x + ((rect.w - lw) / 2.0).max(0.0);
            let dy = rect.y + ((rect.h - lh) / 2.0).max(0.0);
            match frame {
                Some((rgba, ..)) => canvas.blit_rgba(dx, dy, fw, fh, rgba),
                // The GPU shows the scene through a transparent hole; the
                // overlays below draw over it as usual.
                None => self.composited_origin = Some(canvas.clear_physical(dx, dy, fw, fh)),
            }
        }

        let proj = Projector::new(&self.camera, rect);
        let p = self.p;
        let raw = self.raw;
        let editing = self.play == PlayState::Editing;
        // The floating tool switch owns its corner of the view.
        let over_tools = editing
            && (Self::tool_group_rect(canvas, rect).contains(p.x, p.y)
                || Self::snap_group_rect(canvas, rect).contains(p.x, p.y));
        let inside = rect.contains(p.x, p.y) && !over_tools;
        let selected = doc.selected_index().map(|i| &doc.entities[i]);
        // The foliage brush takes the left button while it's on.
        let brushing = self.brush_target(doc);
        let gizmo = if editing && brushing.is_none() {
            selected
                .and_then(|e| doc.world_transform(e))
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
            Drag::Gizmo {
                id,
                axis,
                last,
                start,
                mut free,
            } => {
                if raw.down {
                    let delta = (raw.x - last.0, raw.y - last.1);
                    // The gizmo works in the world; an attached entity
                    // stores its offset from the bone it follows.
                    let entry = doc.index_of(id).map(|i| &doc.entities[i]);
                    let t = entry.and_then(|e| e.data.transform);
                    if let (Some(t), Some(entry)) = (t, entry) {
                        if delta != (0.0, 0.0) {
                            if let Some(g) = Gizmo::build(self.gizmo, &free, &proj) {
                                free = g.drag(axis, &free, (raw.x, raw.y), delta);
                            }
                        }
                        // Holding Ctrl flips snapping for this drag.
                        let snapping = self.prefs.snap.enabled != raw.ctrl;
                        let transform = if snapping {
                            self.prefs.snap.apply(self.gizmo, axis, &start, &free)
                        } else {
                            free
                        };
                        let transform = doc.local_from_world(entry, transform);
                        if transform != t {
                            self.out.push(Intent::SetTransform { id, transform });
                        }
                    }
                    self.drag = Drag::Gizmo {
                        id,
                        axis,
                        last: (raw.x, raw.y),
                        start,
                        free,
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
                adding,
            } => {
                let dist = ((raw.x - start.0).powi(2) + (raw.y - start.1).powi(2)).sqrt();
                let boxing = adding && !orbiting && dist > 4.0;
                if raw.down {
                    let orbiting = orbiting || (!adding && dist > 4.0);
                    if orbiting {
                        self.camera.orbit(raw.x - last.0, raw.y - last.1);
                        self.cursor = CursorHint::Move;
                    }
                    self.drag = Drag::ViewPress {
                        start,
                        last: (raw.x, raw.y),
                        orbiting,
                        double,
                        adding,
                    };
                } else if boxing {
                    self.drag = Drag::None;
                    let picked = view::pick_box(doc, &proj, start, (raw.x, raw.y));
                    let mut ids = doc.selection();
                    for id in picked {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    self.out.push(Intent::SelectMany(ids));
                } else {
                    self.drag = Drag::None;
                    let hit = (!orbiting).then(|| view::pick(doc, &proj, start.0, start.1));
                    if let (Some(Some(id)), true) = (hit, adding) {
                        self.out.push(Intent::SelectMore { id, range: false });
                    } else if let (Some(hit), false) = (hit, adding) {
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
            Drag::Brush { id } => {
                if raw.down {
                    self.brush_stroke(id);
                } else {
                    self.drag = Drag::None;
                    self.brush_end();
                }
            }
            Drag::None if inside && p.pressed && !p.alt && brushing.is_some() => {
                if let Some(id) = brushing {
                    self.drag = Drag::Brush { id };
                    self.brush_start(id);
                }
            }
            Drag::None if inside && p.pressed => {
                let handle = if p.alt {
                    None
                } else {
                    gizmo.as_ref().and_then(|g| g.hit(p.x, p.y))
                };
                let held = selected.and_then(|e| Some((e.id, doc.world_transform(e)?)));
                self.drag = match (handle, held) {
                    (Some(axis), Some((id, t))) => Drag::Gizmo {
                        id,
                        axis,
                        last: (p.x, p.y),
                        start: t,
                        free: t,
                    },
                    _ => Drag::ViewPress {
                        start: (p.x, p.y),
                        last: (p.x, p.y),
                        orbiting: p.alt,
                        double: p.double_click,
                        adding: !p.alt && (p.ctrl || p.shift),
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

        // --- Overlays (editor helpers only: while playing, the view is the
        // game's — often through the scene's own camera, which the editor
        // camera's projection wouldn't even line up with). ---
        if editing {
            if self.show_nav && doc.entities.iter().any(|e| e.data.navigation.is_some()) {
                self.refresh_nav_overlay(doc);
                if let Some((_, mesh)) = &self.nav_overlay {
                    draw_nav_mesh(canvas, mesh, &proj);
                }
            }
            draw_markers(canvas, doc, &proj, rect);
            draw_camera_frustums(canvas, doc, &proj, selected.map(|e| e.id));
            for id in doc.selection() {
                if let Some(i) = doc.index_of(id) {
                    let lead = Some(id) == doc.selected;
                    let frame = doc.attach_frame(&doc.entities[i]);
                    draw_selection(canvas, &doc.entities[i].data, frame, &proj, lead);
                }
            }
            if let Drag::ViewPress {
                start,
                adding: true,
                orbiting: false,
                ..
            } = self.drag
            {
                let (x0, y0) = (start.0.min(raw.x), start.1.min(raw.y));
                let (w, h) = ((raw.x - start.0).abs(), (raw.y - start.1).abs());
                if w.max(h) > 4.0 {
                    let mut fill = color::accent();
                    fill.apply_opacity(0.15);
                    canvas.fill_rect(x0, y0, w, h, fill);
                    canvas.stroke_rect(x0, y0, w, h, color::accent(), 1.0);
                }
            }
            if let Some(g) = &gizmo {
                draw_gizmo(canvas, g, self.hover_axis);
            }
            if brushing.is_some() && inside {
                self.draw_brush(canvas, &proj);
            }
            self.draw_axes_widget(canvas, rect);
        }

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
            BottomTab::Assets => self.assets_body(canvas, doc, body),
            BottomTab::Profiler => self.profiler_body(canvas, body),
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
        let mut x = x;
        for (tab, label) in [
            (BottomTab::Console, console_label),
            (BottomTab::Assets, "Assets".to_string()),
            (BottomTab::Profiler, "Profiler".to_string()),
        ] {
            let tw = canvas.measure_text(layout::TITLE_SIZE, &label) + 22.0;
            let r = Rect::new(x, rect.y, tw, layout::TITLEBAR_H);
            let on = self.bottom_tab == tab;
            let hot = self.hot(r);
            if on {
                canvas.fill_rect(r.x + 8.0, r.y + r.h - 2.0, r.w - 16.0, 2.0, color::accent());
            }
            canvas.text(
                r.x + 11.0,
                r.y + (r.h - layout::TITLE_SIZE) / 2.0 - 1.0,
                layout::TITLE_SIZE,
                &label,
                if on || hot {
                    color::text()
                } else {
                    color::text_dim()
                },
            );
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    self.bottom_tab = tab;
                }
            }
            x += tw;
        }
        if self.bottom_tab == BottomTab::Assets {
            let strip = Rect::new(x, rect.y, rect.x + rect.w - x, layout::TITLEBAR_H);
            self.assets_toolbar(canvas, doc, strip);
        }
        if self.bottom_tab == BottomTab::Profiler {
            let strip = Rect::new(x, rect.y, rect.x + rect.w - x, layout::TITLEBAR_H);
            self.profiler_toolbar(canvas, strip);
        }
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
            let n = doc.selection_len();
            if n > 1 {
                left.push_str(&n.to_string());
                left.push_str(" selected (last: ");
                left.push_str(doc.entities[i].name());
                left.push(')');
            } else {
                left.push_str(doc.entities[i].name());
            }
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

/// What's wrong with an entity's name for script keys: shared with another
/// entity, or containing `:`.
fn name_problem(doc: &Document, e: &EntityEntry) -> Option<String> {
    let name = e.data.name.as_deref()?;
    if name.contains(':') {
        return Some("name has ':' (rename it)".to_string());
    }
    let twins = doc
        .entities
        .iter()
        .filter(|o| o.data.name.as_deref() == Some(name))
        .count();
    (twins > 1).then(|| format!("{twins} entities are called this"))
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
    } else if let Some(emitter) = &d.particle_emitter {
        // A flame with sparks flying off it, in the particles' color.
        let [r, g, b, _] = emitter.color;
        let c = light_color([r, g, b]);
        canvas.fill_polygon(
            &[
                (cx, cy - 8.0),
                (cx + 5.0, cy + 1.0),
                (cx + 3.5, cy + 6.0),
                (cx - 3.5, cy + 6.0),
                (cx - 5.0, cy + 1.0),
            ],
            c,
        );
        canvas.fill_circle(cx + 6.5, cy - 5.0, 1.3, c);
        canvas.fill_circle(cx - 6.5, cy - 3.0, 1.1, c);
    } else if let Some(spot) = d.spot_light {
        // A lamp head with its beam spreading out.
        let c = light_color(spot.color);
        canvas.fill_polygon(
            &[
                (cx - 7.0, cy - 7.0),
                (cx - 2.0, cy - 7.0),
                (cx + 7.0, cy + 6.0),
                (cx - 7.0, cy + 6.0),
            ],
            c,
        );
        canvas.fill_circle(cx - 4.5, cy - 6.0, 2.8, c);
    } else if let Some(point) = d.point_light {
        // A bulb: a bright core in a soft halo.
        let c = light_color(point.color);
        canvas.stroke_circle(cx, cy, 7.0, c, 1.2);
        canvas.fill_circle(cx, cy, 4.0, c);
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
    } else if matches!(
        d.mesh_visual.as_ref().map(|m| &m.primitive),
        Some(automata_core::PrimitiveData::Model { .. })
    ) {
        project_panel::draw_model_icon(canvas, cx, cy, color::text_dim());
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

/// A placed light's icon color: its own hue, kept bright enough to see.
fn light_color(rgb: [f32; 3]) -> Color {
    let peak = rgb.into_iter().fold(0.0_f32, f32::max).max(1e-3);
    let [r, g, b] = rgb.map(|c| (0.35 + 0.65 * (c / peak)).clamp(0.0, 1.0));
    Color::from_rgba(r, g, b, 1.0).unwrap_or_else(color::light_icon)
}

/// Draws a world-space polyline, skipping the parts behind the camera.
fn world_polyline(canvas: &mut Canvas, proj: &Projector, points: &[Vec3], c: Color, width: f32) {
    let mut run: Vec<(f32, f32)> = Vec::new();
    for p in points {
        match proj.project(*p) {
            Some(s) => run.push(s),
            None => {
                if run.len() > 1 {
                    canvas.polyline(&run, c, width);
                }
                run.clear();
            }
        }
    }
    if run.len() > 1 {
        canvas.polyline(&run, c, width);
    }
}

/// The navigation mesh: each polygon outlined a little above the floor,
/// the jump links as arcs.
fn draw_nav_mesh(canvas: &mut Canvas, mesh: &automata_nav::NavMesh, proj: &Projector) {
    let lift = Vec3::Y * 0.04;
    let mut fill = Color::from_rgba8(70, 190, 255, 255);
    fill.apply_opacity(0.18);
    let edge = Color::from_rgba8(110, 210, 255, 200);
    for i in 0..mesh.polys.len() {
        let c = mesh.poly_corners(i).map(|p| p + lift);
        let screen: Option<Vec<(f32, f32)>> = c.iter().map(|p| proj.project(*p)).collect();
        if let Some(pts) = screen {
            canvas.fill_polygon(&pts, fill);
        }
        world_polyline(canvas, proj, &[c[0], c[1], c[2], c[3], c[0]], edge, 1.0);
    }
    let link = Color::from_rgba8(255, 200, 80, 255);
    for l in &mesh.links {
        let rise = (l.start.distance(l.end) * 0.25).max(0.4) + (l.end.y - l.start.y).max(0.0);
        let arc: Vec<Vec3> = (0..=12)
            .map(|k| {
                #[allow(clippy::cast_precision_loss)] // small counter
                let t = k as f32 / 12.0;
                l.start.lerp(l.end, t) + Vec3::Y * (rise * 4.0 * t * (1.0 - t))
            })
            .collect();
        world_polyline(canvas, proj, &arc, link, 1.5);
        if let Some((x, y)) = proj.project(l.end) {
            canvas.fill_circle(x, y, 3.0, link);
        }
    }
}

/// A circle of `radius` around `center` in the plane spanned by `u` and
/// `v` (unit, perpendicular), as a closed list of points.
fn world_circle(center: Vec3, u: Vec3, v: Vec3, radius: f32) -> Vec<Vec3> {
    (0..=48)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)] // small counter
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            center + (u * a.cos() + v * a.sin()) * radius
        })
        .collect()
}

/// A placed light's reach: a point light's range as three rings, a
/// spotlight's cone out to its range (edge solid, core fainter).
fn draw_light_reach(canvas: &mut Canvas, d: &EntityData, frame: glam::Mat4, proj: &Projector) {
    let Some(t) = d.transform else {
        return;
    };
    let model = frame * view::model_matrix(&t);
    let at = model.w_axis.truncate();
    if let Some(point) = d.point_light {
        let range = point.to_light(at).effective_range();
        let c = light_color(point.color);
        for (u, v) in [(Vec3::X, Vec3::Y), (Vec3::X, Vec3::Z), (Vec3::Y, Vec3::Z)] {
            world_polyline(canvas, proj, &world_circle(at, u, v, range), c, 1.2);
        }
    }
    if let Some(spot) = d.spot_light {
        let forward = (-model.z_axis.truncate()).normalize_or(Vec3::NEG_Z);
        let light = spot.to_light(at, forward);
        let range = light.effective_range();
        let (inner, outer) = light.spot.map_or((0.0, 0.5), |cone| cone.angles());
        let u = forward.any_orthonormal_vector();
        let v = forward.cross(u);
        let c = light_color(spot.color);
        let edge = world_circle(
            at + forward * range * outer.cos(),
            u,
            v,
            range * outer.sin(),
        );
        world_polyline(canvas, proj, &edge, c, 1.5);
        for i in 0..4 {
            world_polyline(canvas, proj, &[at, edge[i * 12]], c, 1.5);
        }
        let core = world_circle(
            at + forward * range * inner.cos(),
            u,
            v,
            range * inner.sin(),
        );
        world_polyline(canvas, proj, &core, color::text_faint(), 1.0);
    }
}

/// The selection outline (oriented box), plus a light's direction arrow
/// or reach.
fn draw_selection(
    canvas: &mut Canvas,
    d: &EntityData,
    frame: glam::Mat4,
    proj: &Projector,
    lead: bool,
) {
    if lead {
        draw_light_reach(canvas, d, frame, proj);
    }
    // The lead's box is bright; the rest of a multi-selection is softer.
    let mut edge = color::gizmo_hot();
    if !lead {
        edge.apply_opacity(0.6);
    }
    if d.mesh_visual.is_some() {
        if let Some(corners) = view::box_corners(d, frame) {
            let screen: Vec<Option<(f32, f32)>> =
                corners.iter().map(|c| proj.project(*c)).collect();
            for i in 0..8 {
                for bit in [1usize, 2, 4] {
                    if i & bit == 0 {
                        if let (Some(a), Some(b)) = (screen[i], screen[i | bit]) {
                            canvas.line(a.0, a.1, b.0, b.1, edge, 1.5);
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
/// The project's string tables (`lang/*.csv`), re-read at most once a
/// second, and the language fonts they name.
#[derive(Debug, Default)]
pub struct ProjectStrings {
    checked: Option<std::time::Instant>,
    base: std::path::PathBuf,
    /// Every table, merged.
    pub table: automata_core::StringTable,
    /// The `@font` files given to the canvas as fallbacks.
    fonts: Vec<std::path::PathBuf>,
}

impl Ui {
    /// Re-reads the string tables when a second has passed (or the scene
    /// folder changed), and hands their language fonts to the canvas so
    /// the editor can show every language's text.
    fn refresh_strings(&mut self, canvas: &mut Canvas, doc: &Document) {
        let base = doc.base_dir();
        let s = &mut self.strings;
        if s.base == base
            && s.checked
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        s.checked = Some(std::time::Instant::now());
        s.table = automata_core::StringTable::load(&base);
        s.base = base;
        let mut fonts: Vec<std::path::PathBuf> = Vec::new();
        for f in s.table.fonts.iter().filter(|f| !f.is_empty()) {
            let path = automata_assets::join_relative(&s.base, f);
            if !fonts.contains(&path) {
                fonts.push(path);
            }
        }
        if fonts != s.fonts {
            let loaded = fonts
                .iter()
                .filter_map(|p| automata_renderer::text::load_font_file(p).ok())
                .collect();
            canvas.set_fallback_fonts(loaded);
            s.fonts = fonts;
        }
    }

    /// The string-table keys with their first language's text, for
    /// `ctx.tr("…")` completion.
    fn string_keys(&self) -> Vec<(String, String)> {
        let table = &self.strings.table;
        let first = table.languages.first().map_or("", String::as_str);
        table
            .entries
            .keys()
            .map(|k| {
                (
                    k.clone(),
                    table.get(k, first).unwrap_or_default().to_string(),
                )
            })
            .collect()
    }
}

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
/// Splits `text` into lines no wider than `max` at word boundaries (a word
/// longer than a line gets a line of its own).
fn wrap_words(canvas: &mut Canvas, text: &str, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && canvas.measure_text(layout::SMALL_SIZE, &candidate) > max {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Where a dragged file would land.
#[derive(Debug, Clone, Copy)]
enum FileDrop {
    /// The scene view: add a new entity.
    Scene(Rect),
    /// The Inspector: use it on this entity.
    Entity(u64, Rect),
    /// Nowhere useful.
    Nothing,
}

/// Whether an entity's Mesh Renderer has its own colour and surface (no
/// material file overrides them).
fn material_free(d: &EntityData) -> bool {
    d.mesh_visual
        .as_ref()
        .is_some_and(|m| m.material.trim().is_empty())
}

/// An accent outline around a drop target with a label at its top.
fn draw_drop_zone(canvas: &mut Canvas, r: Rect, label: &str) {
    canvas.stroke_rounded(
        r.x + 3.0,
        r.y + 3.0,
        r.w - 6.0,
        r.h - 6.0,
        6.0,
        color::accent(),
        1.5,
    );
    let tw = canvas.measure_text(layout::SMALL_SIZE, label) + 20.0;
    let x = r.x + (r.w - tw) / 2.0;
    let y = r.y + 12.0;
    canvas.fill_rounded(x, y, tw, 24.0, 12.0, color::accent());
    canvas.text(
        x + 10.0,
        y + 5.0,
        layout::SMALL_SIZE,
        label,
        color::background(),
    );
}

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
    fn play_mode_hides_editor_overlays() {
        // The viewport's pixels with and without a light (a marker + label
        // in the editor): different while editing, identical while playing.
        let view = |play: PlayState, with_sun: bool| {
            let mut doc = Document::starter();
            doc.select(None);
            if !with_sun {
                doc.entities.retain(|e| e.data.directional_light.is_none());
            }
            let mut ui = Ui::new();
            ui.play = play;
            let mut c = canvas();
            ui.frame(&mut c, &doc, Pointer::default(), None);
            let r = lay().viewport;
            let (w, px) = (c.size().0 as usize, c.pixels());
            let mut out = Vec::new();
            for y in r.y as usize + 2..(r.y + r.h) as usize - 2 {
                let row = y * w * 4;
                out.extend_from_slice(
                    &px[row + (r.x as usize + 2) * 4..row + ((r.x + r.w) as usize - 2) * 4],
                );
            }
            out
        };
        assert_ne!(
            view(PlayState::Editing, true),
            view(PlayState::Editing, false)
        );
        assert_eq!(
            view(PlayState::Playing, true),
            view(PlayState::Playing, false)
        );
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

    /// A project folder holding one model, `rock.obj`. Returns (root, model).
    fn model_project(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("oberon-ui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let model = root.join("rock.obj");
        std::fs::write(&model, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        (root, model)
    }

    /// Drags from `from` to `to` over a few frames and releases.
    fn drag(
        ui: &mut Ui,
        c: &mut Canvas,
        doc: &Document,
        from: (f32, f32),
        to: (f32, f32),
    ) -> Vec<Intent> {
        let at = |x, y, down, pressed| Pointer {
            x,
            y,
            down,
            pressed,
            ..Pointer::default()
        };
        let mut out = ui.frame(c, doc, at(from.0, from.1, true, true), None);
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            let p = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            out.extend(ui.frame(c, doc, at(p.0, p.1, true, false), None));
        }
        out.extend(ui.frame(c, doc, at(to.0, to.1, false, false), None));
        out
    }

    #[test]
    fn dragging_a_model_from_files_into_the_scene_adds_it_there() {
        let (root, model) = model_project("drag");
        let mut c = canvas();
        let mut doc = Document::starter();
        doc.select(None);
        let mut ui = Ui::new();
        ui.project_root = root.clone();
        let files = lay().files;
        let row = (files.x + 60.0, files.y + layout::TITLEBAR_H + 4.0 + 11.0);
        let (vx, vy) = center_of(lay().viewport);
        let target = (vx + 40.0, vy + 60.0);
        let out = drag(&mut ui, &mut c, &doc, row, target);
        assert!(
            out.contains(&Intent::AddModel {
                path: model.clone(),
                at: Some(target),
            }),
            "{out:?}"
        );
        assert!(ui.file_drag.is_none());
        assert_eq!(ui.viewport_rect(), lay().viewport);

        // Dropped on the Inspector, it goes onto the selected entity.
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        doc.select(Some(cube));
        let insp = center_of(lay().inspector);
        let out = drag(&mut ui, &mut c, &doc, row, insp);
        assert!(
            out.contains(&Intent::SetModel {
                id: cube,
                path: model.clone()
            }),
            "{out:?}"
        );

        // A plain click (no movement) only selects the file.
        let out = click(&mut ui, &mut c, &doc, row.0, row.1);
        assert!(
            !out.iter().any(|i| matches!(i, Intent::AddModel { .. })),
            "{out:?}"
        );
        // Double-clicking adds it where the camera looks.
        let press = Pointer {
            x: row.0,
            y: row.1,
            down: true,
            pressed: true,
            double_click: true,
            ..Pointer::default()
        };
        let out = ui.frame(&mut c, &doc, press, None);
        assert!(
            out.contains(&Intent::AddModel {
                path: model,
                at: None
            }),
            "{out:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dragging_a_material_onto_an_object_puts_it_on() {
        let root = std::env::temp_dir().join(format!("oberon-ui-matdrag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mat = root.join("brick.mat.ron");
        automata_assets::MaterialFile::default().save(&mat).unwrap();
        let mut c = canvas();
        let mut doc = Document::starter();
        doc.select(None);
        let mut ui = Ui::new();
        ui.project_root = root.clone();
        let files = lay().files;
        let row = (files.x + 60.0, files.y + layout::TITLEBAR_H + 4.0 + 11.0);
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        let set = |out: &[Intent], id: u64| {
            out.contains(&Intent::SetMaterial {
                id,
                path: Some(mat.clone()),
            })
        };
        // Onto the object in the scene view.
        let t = doc.entities[doc.index_of(cube).unwrap()]
            .data
            .transform
            .unwrap();
        let proj = view::Projector::new(&ui.camera, lay().viewport);
        let on_cube = proj.project(Vec3::from(t.position)).unwrap();
        let out = drag(&mut ui, &mut c, &doc, row, on_cube);
        assert!(set(&out, cube), "{out:?}");
        // Onto empty sky: nothing.
        let sky = (lay().viewport.x + 30.0, lay().viewport.y + 30.0);
        let out = drag(&mut ui, &mut c, &doc, row, sky);
        assert!(
            !out.iter().any(|i| matches!(i, Intent::SetMaterial { .. })),
            "{out:?}"
        );
        // Onto its Hierarchy row.
        let i = doc.index_of(cube).unwrap();
        let h = lay().hierarchy;
        let at = (
            h.x + 60.0,
            h.y + layout::TITLEBAR_H + 4.0 + (i as f32 + 0.5) * layout::ROW_H,
        );
        let out = drag(&mut ui, &mut c, &doc, row, at);
        assert!(set(&out, cube), "{out:?}");
        // Onto the Inspector, for the selected object.
        doc.select(Some(cube));
        let out = drag(&mut ui, &mut c, &doc, row, center_of(lay().inspector));
        assert!(set(&out, cube), "{out:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Clicks down the Inspector at `x`, scrolling, until `found` says
    /// yes. Every click starts from a fresh `make()` (a stray click on a
    /// header would fold the section away), so the Ui that found it is
    /// returned too.
    fn sweep_inspector(
        make: impl Fn() -> Ui,
        c: &mut Canvas,
        doc: &Document,
        x: f32,
        found: impl Fn(&Ui, &[Intent]) -> bool,
    ) -> Option<(Ui, Vec<Intent>)> {
        let insp = lay().inspector;
        for scroll in [0.0, 400.0] {
            for row in 0..((insp.h / 10.0) as usize) {
                let mut ui = make();
                ui.inspector_scroll = scroll;
                ui.frame(c, doc, Pointer::default(), None);
                let y = insp.y + layout::TITLEBAR_H + 5.0 + row as f32 * 10.0;
                let out = click(&mut ui, c, doc, x, y);
                if found(&ui, &out) {
                    return Some((ui, out));
                }
            }
        }
        None
    }

    #[test]
    fn the_springs_section_picks_bones_and_edits_chains() {
        use automata_assets::import::{read_settings, write_settings};
        use springs::{ChainField, ImportField};
        let root = std::env::temp_dir().join(format!("oberon-ui-springs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let model = root.join("runner.glb");
        std::fs::copy(
            "../../examples/oberon/ponytail_park/models/runner.glb",
            &model,
        )
        .unwrap();
        let mut c = canvas();
        let mut doc = Document::new();
        let id = doc.create_model(&model.to_string_lossy(), "Runner", [0.0; 3]);
        doc.select(Some(id));
        let make = || {
            let mut ui = Ui::new();
            ui.project_root = root.clone();
            ui
        };
        let insp = lay().inspector;

        // "+ Chain" opens the bone picker, likely roots first.
        let x = insp.x + layout::PAD + layout::LABEL_W + 16.0;
        let chain_picker = |ui: &Ui, _: &[Intent]| {
            ui.palette.as_ref().and_then(|p| p.placeholder)
                == Some("Choose the first bone of the chain…")
        };
        let (mut ui, _) = sweep_inspector(make, &mut c, &doc, x, chain_picker).expect("+ Chain");
        let matches = ui.palette.as_ref().unwrap().matches();
        let top: Vec<(&str, &str)> = matches
            .iter()
            .take(3)
            .map(|i| (i.label.as_str(), i.group))
            .collect();
        assert_eq!(top[..2], [("Hair1", "Suggested"), ("Skirt", "Suggested")]);
        assert_eq!(top[2].1, "Bone");
        ui.palette_key(palette::PaletteKey::Enter, &doc);
        let out = ui.frame(&mut c, &doc, Pointer::default(), None);
        let Some(Intent::SetImport { model: m, settings }) = out
            .iter()
            .find(|i| matches!(i, Intent::SetImport { .. }))
            .cloned()
        else {
            panic!("{out:?}");
        };
        assert_eq!(m, model);
        assert_eq!(
            settings.springs.chains,
            [automata_assets::springs::SpringChain::new("Hair1")]
        );
        write_settings(&model, &settings).unwrap();

        // Typing a chain's value commits just that value.
        ui.import_edit = Some((model.clone(), settings.clone()));
        ui.edit = Some(TextEdit {
            id,
            target: EditTarget::Import(ImportField::Chain(0, ChainField::Stiffness)),
            buffer: "0.8".into(),
            rect: Rect::default(),
            fresh: false,
            attr: None,
        });
        let Some(Intent::SetImport {
            settings: stiffer, ..
        }) = ui.commit_edit()
        else {
            panic!("no commit");
        };
        assert_eq!(stiffer.springs.chains[0].stiffness, 0.8);
        assert_eq!(
            stiffer.springs.chains[0].drag,
            settings.springs.chains[0].drag
        );

        // The chain's × removes it.
        let removed = |_: &Ui, out: &[Intent]| {
            out.iter().any(|i| {
                matches!(i, Intent::SetImport { settings, .. }
                    if settings.is_identity())
            })
        };
        let close_x = insp.x + insp.w - layout::PAD - 11.0;
        let found = sweep_inspector(make, &mut c, &doc, close_x, removed).is_some();
        assert!(found, "the chain has a remove button");
        assert_eq!(
            read_settings(&model).springs.chains.len(),
            1,
            "UI only asks"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_model_picker_lists_project_models_and_imports() {
        let (root, model) = model_project("picker");
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root = root.clone();
        ui.open_model_picker(Some(7));
        let labels: Vec<String> = ui
            .palette
            .as_ref()
            .unwrap()
            .matches()
            .iter()
            .map(|i| i.label.clone())
            .collect();
        assert_eq!(labels, ["rock.obj", "Import from file…"]);
        ui.palette_key(palette::PaletteKey::Enter, &doc);
        let out = ui.frame(&mut c, &doc, Pointer::default(), None);
        assert!(
            out.contains(&Intent::SetModel { id: 7, path: model }),
            "{out:?}"
        );

        // "Import from file…" opens the model file browser.
        ui.open_model_picker(None);
        ui.palette_text("import");
        ui.palette_key(palette::PaletteKey::Enter, &doc);
        assert!(matches!(&ui.dialog, Some(dialogs::Dialog::File(_))));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_mesh_section_shows_the_model_and_its_actions() {
        let (root, model) = model_project("inspector");
        let mut c = canvas();
        let mut doc = Document::new();
        let id = doc.create_model(&model.to_string_lossy(), "Rock", [0.0; 3]);
        let mut ui = Ui::new();
        ui.project_root = root.clone();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        // The selection box hugs the model (centred off the origin).
        let e = &doc.entities[doc.index_of(id).unwrap()];
        let (center, half) = crate::view::local_bounds(&e.data);
        assert!((center.x - 0.5).abs() < 1e-5 && (half.y - 0.5).abs() < 1e-5);
        // "Add Collider" adds a fitted collider.
        let insp = lay().inspector;
        let mut found = None;
        for y in (insp.y as i32..(insp.y + insp.h) as i32).step_by(4) {
            let x = insp.x + insp.w - 40.0;
            let out = click(&mut ui, &mut c, &doc, x, y as f32);
            if out.contains(&Intent::AddComponent(id, ComponentKind::Collider)) {
                found = Some(y);
                break;
            }
        }
        assert!(found.is_some(), "Add Collider button");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_attach_section_picks_an_entity_and_its_bones() {
        let mut c = canvas();
        let mut doc = Document::new();
        doc.create_model(crate::play::tests::ROBOT, "Robot", [0.0; 3]);
        let i = doc.add_entity_named("Hat");
        let hat = doc.entities[i].id;
        doc.modify(hat, false, |d| ComponentKind::Attach.add_to(d));
        doc.select(Some(hat));
        let mut ui = Ui::new();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let insp = lay().inspector;
        let value_x = insp.x + layout::PAD + layout::LABEL_W;
        // Find the Follow menu: it offers the robot.
        let mut follow_y = None;
        for y in (insp.y as i32..(insp.y + insp.h) as i32).step_by(4) {
            ui.menu = None;
            click(&mut ui, &mut c, &doc, value_x + 40.0, y as f32);
            let robot = ui
                .menu
                .as_ref()
                .and_then(|m| m.items.iter().find(|i| i.label.trim() == "Robot").cloned());
            if let Some(item) = robot {
                assert_eq!(
                    item.action,
                    Some(menus::Action::Intent(Intent::SetAttach {
                        id: hat,
                        to: Some("Robot".into()),
                        bone: None
                    }))
                );
                follow_y = Some(y as f32);
                break;
            }
        }
        let follow_y = follow_y.expect("the Follow menu");
        // Following the robot, the Bone menu (a row down) lists its bones.
        doc.modify(hat, false, |d| {
            d.attach.as_mut().unwrap().to = "Robot".into();
        });
        ui.menu = None;
        click(
            &mut ui,
            &mut c,
            &doc,
            value_x + 40.0,
            follow_y + layout::ROW_STEP,
        );
        let menu = ui.menu.as_ref().expect("the Bone menu");
        let head = menu
            .items
            .iter()
            .find(|i| i.label.trim() == "Head")
            .expect("the robot's head");
        assert_eq!(
            head.action,
            Some(menus::Action::Intent(Intent::SetAttach {
                id: hat,
                to: None,
                bone: Some("Head".into())
            }))
        );
        assert!(menu.items.len() > 40, "every bone: {}", menu.items.len());
        // Its transform is an offset from the bone's frame.
        let frame = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_y(1.0),
            glam::Vec3::new(1.0, 2.0, 3.0),
        );
        doc.set_attach_frames(std::iter::once((hat, frame)).collect());
        let e = &doc.entities[doc.index_of(hat).unwrap()];
        let world = doc.world_transform(e).unwrap();
        assert!(
            (glam::Vec3::from(world.position) - glam::Vec3::new(1.0, 2.0, 3.0)).length() < 1e-4
        );
        let moved = TransformData {
            position: [1.0, 3.0, 3.0],
            ..world
        };
        let local = doc.local_from_world(e, moved);
        assert!(
            (glam::Vec3::from(local.position) - glam::Vec3::Y).length() < 1e-4,
            "{local:?}"
        );
    }

    #[test]
    fn the_animation_section_picks_clips_and_previews_them() {
        let mut c = canvas();
        let mut doc = Document::new();
        let id = doc.create_model(crate::play::tests::ROBOT, "Robot", [0.0; 3]);
        doc.select(Some(id));
        let mut ui = Ui::new();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let insp = lay().inspector;
        let value_x = insp.x + layout::PAD + layout::LABEL_W;
        // Scrolled to the bottom, where the Animation section is; then up
        // to the preview's play button (the first click on it plays).
        let wheel = Pointer {
            x: insp.x + 40.0,
            y: insp.y + 200.0,
            scroll: -100.0,
            ..Pointer::default()
        };
        ui.frame(&mut c, &doc, wheel, None);
        let mut button_y = None;
        for y in (insp.y as i32..(insp.y + insp.h) as i32).rev().step_by(5) {
            ui.menu = None;
            click(&mut ui, &mut c, &doc, value_x + 9.0, y as f32);
            if ui.anim_preview.is_some_and(|p| p.playing && p.id == id) {
                button_y = Some(y as f32);
                break;
            }
        }
        let button_y = button_y.expect("a preview play button");
        // Scrubbing the time bar pauses at the pointer's share of the clip.
        let bar_x = value_x + 24.0;
        let bar_w = insp.w - 2.0 * layout::PAD - layout::LABEL_W - 24.0;
        click(&mut ui, &mut c, &doc, bar_x + bar_w * 0.5, button_y);
        let p = ui.anim_preview.unwrap();
        assert!(!p.playing, "{p:?}");
        assert!((p.time - 3.3333 * 0.5).abs() < 0.1, "half of Idle: {p:?}");
        // One row up, the Graph row offers a graph for the robot.
        ui.menu = None;
        click(
            &mut ui,
            &mut c,
            &doc,
            value_x + 40.0,
            button_y - layout::ROW_STEP,
        );
        assert!(ui.file_dialog_open(), "the graph picker");
        assert_eq!(ui.file_dialog_name(), Some("robot.anim.ron"));
        ui.close_file_dialog();
        // Seven rows up (past Graph, Feet On Ground, Root Motion, Play On
        // Start, Loop and Speed), the clip menu lists the model's clips.
        click(
            &mut ui,
            &mut c,
            &doc,
            value_x + 40.0,
            button_y - 7.0 * layout::ROW_STEP,
        );
        let menu = ui.menu.as_ref().expect("the clip menu");
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.trim()).collect();
        assert_eq!(labels.len(), 15, "None + 14 clips: {labels:?}");
        assert!(
            labels.contains(&"✓ Idle") && labels.contains(&"Wave"),
            "{labels:?}"
        );
        let wave = menu
            .items
            .iter()
            .find(|i| i.label.trim() == "Wave")
            .unwrap();
        assert_eq!(
            wave.action,
            Some(menus::Action::Intent(Intent::SetAnimationClip {
                id,
                clip: "Wave".into()
            }))
        );
    }

    #[test]
    fn the_joint_section_picks_what_it_connects_to() {
        let mut c = canvas();
        let mut doc = Document::new();
        doc.add_entity_named("Frame");
        let idx = doc.add_entity_named("Door");
        let id = doc.entities[idx].id;
        doc.modify(id, false, |d| ComponentKind::Joint.add_to(d));
        doc.select(Some(id));
        let mut ui = Ui::new();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let insp = lay().inspector;
        let value_x = insp.x + layout::PAD + layout::LABEL_W + 40.0;
        let mut menu = None;
        'scroll: for _ in 0..30 {
            for y in (insp.y as i32..(insp.y + insp.h) as i32).step_by(6) {
                ui.menu = None;
                click(&mut ui, &mut c, &doc, value_x, y as f32);
                if let Some(m) = ui
                    .menu
                    .as_ref()
                    .filter(|m| m.items.iter().any(|i| i.label.trim() == "✓ World"))
                {
                    menu = Some(m.clone());
                    break 'scroll;
                }
            }
            let wheel = Pointer {
                x: insp.x + 40.0,
                y: insp.y + 200.0,
                scroll: -3.0,
                ..Pointer::default()
            };
            ui.frame(&mut c, &doc, wheel, None);
        }
        let menu = menu.expect("a Connected To menu");
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.trim()).collect();
        assert!(!labels.contains(&"Door"), "not itself: {labels:?}");
        let frame = menu
            .items
            .iter()
            .find(|i| i.label.trim() == "Frame")
            .expect("Frame is offered");
        assert_eq!(
            frame.action,
            Some(menus::Action::Intent(Intent::SetJointTarget {
                id,
                target: Some("Frame".into())
            }))
        );
    }

    #[test]
    fn renaming_an_entity_keeps_joints_pointed_at_it() {
        let mut doc = Document::new();
        let i = doc.add_entity_named("Frame");
        let frame = doc.entities[i].id;
        let i = doc.add_entity_named("Door");
        let door = doc.entities[i].id;
        doc.modify(door, false, |d| {
            d.joint = Some(automata_physics::Joint::hinge(glam::Vec3::Y).connected_to("Frame"));
        });
        assert!(doc.rename_entity(frame, "Door Frame"));
        let connected = |doc: &Document| {
            doc.entities
                .iter()
                .find(|e| e.id == door)
                .and_then(|e| e.data.joint.as_ref())
                .and_then(|j| j.connected.clone())
        };
        assert_eq!(connected(&doc).as_deref(), Some("Door Frame"));
        doc.undo();
        assert_eq!(
            connected(&doc).as_deref(),
            Some("Frame"),
            "undo restores both"
        );
    }

    #[test]
    fn shared_and_colon_names_are_flagged_for_script_keys() {
        let mut doc = Document::new();
        for name in ["Crate", "Crate", "Player", "Odd:One"] {
            doc.add_entity_named(name);
        }
        let problems: Vec<Option<String>> = doc
            .entities
            .iter()
            .map(|e| super::name_problem(&doc, e))
            .collect();
        assert_eq!(
            problems,
            [
                Some("2 entities are called this".to_string()),
                Some("2 entities are called this".to_string()),
                None,
                Some("name has ':' (rename it)".to_string()),
            ]
        );
    }

    #[test]
    fn the_import_section_saves_settings_for_the_file() {
        use automata_assets::import::{ImportSettings, Pivot};
        let (root, model) = model_project("import");
        let mut c = canvas();
        let mut doc = Document::new();
        let id = doc.create_model(&model.to_string_lossy(), "Rock", [0.0; 3]);
        doc.select(Some(id));
        let mut ui = Ui::new();
        ui.project_root = root.clone();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        // Sweep the Inspector's right edge and middle: the quick buttons.
        // A click that opens a dialog (the Material row's chooser) would
        // swallow the rest, so close it each time.
        let insp = lay().inspector;
        let mut saved = Vec::new();
        for x in [insp.x + insp.w - 30.0, insp.x + insp.w * 0.62] {
            for y in (insp.y as i32..(insp.y + insp.h) as i32).step_by(4) {
                for intent in click(&mut ui, &mut c, &doc, x, y as f32) {
                    if let Intent::SetImport { model: m, settings } = intent {
                        assert_eq!(m, model);
                        saved.push(settings);
                    }
                }
                ui.dialog = None;
            }
        }
        let turned = ImportSettings {
            rotation: [0.0, 90.0, 0.0],
            ..Default::default()
        };
        let bigger = ImportSettings {
            scale: 10.0,
            ..Default::default()
        };
        let bottom = ImportSettings {
            pivot: Pivot::BottomCenter,
            ..Default::default()
        };
        for want in [turned.clone(), bigger, bottom] {
            assert!(saved.contains(&want), "{want:?} in {saved:?}");
        }
        // Typing a value commits the edited field only.
        ui.import_edit = Some((model.clone(), turned.clone()));
        ui.edit = Some(TextEdit {
            id,
            target: EditTarget::Import(springs::ImportField::Scale),
            buffer: "0.5".into(),
            rect: Rect::default(),
            fresh: false,
            attr: None,
        });
        let intent = ui.commit_edit();
        assert_eq!(
            intent,
            Some(Intent::SetImport {
                model: model.clone(),
                settings: ImportSettings {
                    scale: 0.5,
                    ..turned
                },
            })
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_assets_tab_shows_thumbnails_and_adds_models() {
        let (root, model) = model_project("assets");
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.project_root = root.clone();
        ui.toggle_assets();
        assert_eq!(ui.bottom_tab, BottomTab::Assets);
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let console = ui.layout(1600.0, 950.0).console;
        let tile = (
            console.x + 10.0 + 52.0,
            console.y + layout::TITLEBAR_H + 8.0 + 40.0,
        );
        let px = (80.0 * c.scale()).round() as u32;
        assert!(
            matches!(ui.assets.thumbs.get(&model, px), Some(Some(_))),
            "the thumbnail was drawn"
        );
        // Double-click adds it; dragging drops it where it lands.
        let press = Pointer {
            x: tile.0,
            y: tile.1,
            down: true,
            pressed: true,
            double_click: true,
            ..Pointer::default()
        };
        let out = ui.frame(&mut c, &doc, press, None);
        assert!(
            out.contains(&Intent::AddModel {
                path: model.clone(),
                at: None
            }),
            "{out:?}"
        );
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let (vx, vy) = center_of(lay().viewport);
        let out = drag(&mut ui, &mut c, &doc, tile, (vx, vy - 100.0));
        assert!(
            out.contains(&Intent::AddModel {
                path: model.clone(),
                at: Some((vx, vy - 100.0)),
            }),
            "{out:?}"
        );
        // The filter hides what doesn't match; Sounds lists none here.
        ui.assets.query = "tree".into();
        ui.assets.selected = None;
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let out = click(&mut ui, &mut c, &doc, tile.0, tile.1);
        assert!(ui.assets.selected.is_none(), "{out:?}");
        ui.assets.query.clear();
        ui.assets.filter = assets_panel::AssetFilter::Sounds;
        ui.frame(&mut c, &doc, Pointer::default(), None);
        click(&mut ui, &mut c, &doc, tile.0, tile.1);
        assert!(ui.assets.selected.is_none());
        let _ = std::fs::remove_dir_all(&root);
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
        let doc = Document::starter();
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
        // The starter's sky lives on its sun; the presets emit
        // SetEnvironment for it.
        let env_id = doc
            .entities
            .iter()
            .find(|e| e.data.environment.is_some())
            .expect("the starter has a sky")
            .id;
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
                    if *id == env_id && **env == automata_renderer::Environment::night())
            }) {
                found_preset = true;
                break;
            }
        }
        assert!(found_preset, "the Night preset is clickable");
    }

    #[test]
    fn graphics_tab_dials_in_a_post_look() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.right_tab = RightTab::Graphics;
        let insp = lay().inspector;
        let env_id = doc
            .entities
            .iter()
            .find(|e| e.data.environment.is_some())
            .expect("the starter has a sky")
            .id;
        let bleached = automata_renderer::PostEffects::bleached();
        let _ = ui.frame(&mut c, &doc, Pointer::default(), None);
        // The looks row spans the panel: Bleached is the second of five.
        let x = insp.x + layout::PAD + (insp.w - 2.0 * layout::PAD) * 0.3;
        let mut found = false;
        'scan: for scroll in [0.0, 300.0, 600.0, 900.0] {
            for row in 0..70 {
                ui.inspector_scroll = scroll;
                let y = insp.y + layout::TITLEBAR_H + 10.0 + row as f32 * 12.0;
                let out = click(&mut ui, &mut c, &doc, x, y);
                ui.right_tab = RightTab::Graphics;
                ui.picker = None;
                ui.menu = None;
                if out.iter().any(|i| {
                    matches!(i, Intent::SetEnvironment(id, env)
                        if *id == env_id && env.post == bleached)
                }) {
                    found = true;
                    break 'scan;
                }
            }
        }
        assert!(found, "the Bleached look is clickable");
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
        // Selecting is fine while playing; prefab edits aren't.
        assert!(!Intent::SelectMore { id: 1, range: true }.edits_document());
        assert!(!Intent::SelectMany(vec![1, 2]).edits_document());
        assert!(!Intent::SelectInstances(1).edits_document());
        assert!(Intent::ApplyPrefab(1).edits_document());
        assert!(Intent::RevertPrefab { id: 1, paths: None }.edits_document());
        assert!(Intent::UnlinkPrefab(1).edits_document());
        assert!(Intent::DuplicateAlong {
            id: 1,
            count: 2,
            step: [1.0, 0.0, 0.0],
            turn_y: 0.0
        }
        .edits_document());
    }

    #[test]
    fn edits_fan_out_to_every_selected_entity() {
        // Retargeting keeps everything but the entity.
        let set = Intent::SetColor {
            id: 1,
            target: picker::ColorTarget::Mesh,
            rgba: [1.0, 0.0, 0.0, 1.0],
            gesture: false,
        };
        assert_eq!(set.fan_target(), Some(1));
        assert_eq!(
            set.retarget(7),
            Some(Intent::SetColor {
                id: 7,
                target: picker::ColorTarget::Mesh,
                rgba: [1.0, 0.0, 0.0, 1.0],
                gesture: false,
            })
        );
        assert_eq!(Intent::Delete(3).retarget(4), Some(Intent::Delete(4)));
        // Selection and one-off commands stay put.
        assert_eq!(Intent::Select(Some(1)).fan_target(), None);
        assert_eq!(Intent::ApplyPrefab(1).fan_target(), None);
        assert_eq!(
            Intent::RevertPrefab {
                id: 1,
                paths: Some(vec!["mesh_visual.color".into()])
            }
            .fan_target(),
            None
        );
    }

    fn click_with(
        ui: &mut Ui,
        c: &mut Canvas,
        doc: &Document,
        (x, y): (f32, f32),
        ctrl: bool,
        shift: bool,
    ) -> Vec<Intent> {
        let press = Pointer {
            x,
            y,
            down: true,
            pressed: true,
            shift,
            ctrl,
            ..Pointer::default()
        };
        let mut out = ui.frame(c, doc, press, None);
        let release = Pointer {
            x,
            y,
            shift,
            ctrl,
            ..Pointer::default()
        };
        out.extend(ui.frame(c, doc, release, None));
        out
    }

    #[test]
    fn ctrl_and_shift_clicks_extend_the_selection() {
        let mut c = canvas();
        let mut doc = Document::starter();
        let ids: Vec<u64> = doc.entities.iter().map(|e| e.id).collect();
        assert!(ids.len() >= 3);
        doc.select(Some(ids[0]));
        let mut ui = Ui::new();
        let hier = lay().hierarchy;
        #[allow(clippy::cast_precision_loss)]
        let row = |i: usize| {
            (
                hier.x + 60.0,
                hier.y + layout::TITLEBAR_H + 4.0 + layout::ROW_H * (i as f32 + 0.5),
            )
        };
        let out = click_with(&mut ui, &mut c, &doc, row(2), true, false);
        assert!(
            out.contains(&Intent::SelectMore {
                id: ids[2],
                range: false
            }),
            "{out:?}"
        );
        let out = click_with(&mut ui, &mut c, &doc, row(1), false, true);
        assert!(
            out.contains(&Intent::SelectMore {
                id: ids[1],
                range: true
            }),
            "{out:?}"
        );
        // Right-clicking inside a multi-selection keeps it.
        doc.select_many(&[ids[0], ids[2]]);
        let out = right_click(&mut ui, &mut c, &doc, row(0).0, row(0).1);
        assert!(
            !out.iter().any(|i| matches!(i, Intent::Select(_))),
            "{out:?}"
        );
        let menu = ui.menu.as_ref().expect("context menu");
        assert!(menu.items.iter().any(|i| i.label == "Duplicate Along…"));
        // In the scene view, Ctrl+click on the cube adds it.
        ui.menu = None;
        doc.select(Some(ids[0]));
        let cube = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        let proj = Projector::new(&ui.camera, lay().viewport);
        let at = proj.project(Vec3::new(0.0, 0.5, 0.0)).unwrap();
        let mut ui = Ui::new();
        let out = click_with(&mut ui, &mut c, &doc, at, true, false);
        assert!(
            out.contains(&Intent::SelectMore {
                id: cube,
                range: false
            }),
            "{out:?}"
        );
    }

    #[test]
    fn snapping_the_move_gizmo_lands_on_the_grid() {
        let mut c = canvas();
        let mut doc = Document::starter();
        let cube = select_named(&mut doc, "Cube");
        let mut ui = Ui::new();
        // The scene view's Snap toggle and step.
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let g = Ui::snap_group_rect(&mut c, lay().viewport);
        click(&mut ui, &mut c, &doc, g.x + 12.0, g.y + g.h / 2.0);
        assert!(ui.prefs.snap.enabled);
        click(&mut ui, &mut c, &doc, g.x + g.w - 12.0, g.y + g.h / 2.0);
        assert!((ui.prefs.snap.step - 1.0).abs() < 1e-6, "0.5 m → 1 m");
        assert!(ui.take_prefs_dirty());
        ui.prefs.snap.step = 0.5;

        let proj = Projector::new(&ui.camera, lay().viewport);
        let t = doc.entities[doc.index_of(cube).unwrap()]
            .data
            .transform
            .unwrap();
        let gz = Gizmo::build(GizmoMode::Move, &t, &proj).unwrap();
        let tip = gz.tips[1].unwrap();
        let grab = (
            f32::midpoint(gz.origin.0, tip.0),
            f32::midpoint(gz.origin.1, tip.1),
        );
        let at = |dy: f32, down: bool, pressed: bool| Pointer {
            x: grab.0,
            y: grab.1 - dy,
            down,
            pressed,
            ..Pointer::default()
        };
        ui.frame(&mut c, &doc, at(0.0, true, true), None);
        // A nudge too small to reach the next grid line: nothing yet.
        let out = ui.frame(&mut c, &doc, at(3.0, true, false), None);
        assert!(
            !out.iter().any(|i| matches!(i, Intent::SetTransform { .. })),
            "{out:?}"
        );
        // Further: the drag accumulates and lands on a whole step.
        let out = ui.frame(&mut c, &doc, at(90.0, true, false), None);
        let moved = out
            .iter()
            .find_map(|i| match i {
                Intent::SetTransform { id, transform } if *id == cube => Some(*transform),
                _ => None,
            })
            .expect("a snapped move");
        let y = moved.position[1];
        assert!(y > t.position[1]);
        assert!(
            ((y / 0.5) - (y / 0.5).round()).abs() < 1e-4,
            "{y} on the grid"
        );
        assert_eq!(moved.position[0], t.position[0]);
        ui.frame(&mut c, &doc, at(90.0, false, false), None);
    }

    #[test]
    fn the_inspector_shows_prefab_overrides_and_multi_selection() {
        let mut c = canvas();
        let dir = std::env::temp_dir().join(format!("oberon-ui-prefab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        let cube = select_named(&mut doc, "Cube");
        let file = doc.prefab_of(cube).unwrap();
        std::fs::write(dir.join("cube.prefab"), file.to_ron().unwrap()).unwrap();
        assert!(doc.link_prefab(cube, "cube.prefab"));
        doc.modify(cube, false, |d| {
            d.mesh_visual.as_mut().unwrap().color = [1.0, 0.0, 0.0, 1.0];
        });
        let mut ui = Ui::new();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        // Find the per-override Revert button by sweeping the Inspector.
        let ins = lay().inspector;
        let mut reverted = None;
        let mut applied = false;
        let mut y = ins.y + layout::TITLEBAR_H;
        while y < ins.y + ins.h && (reverted.is_none() || !applied) {
            for x in [ins.x + ins.w - 40.0, ins.x + 40.0] {
                let out = click(&mut ui, &mut c, &doc, x, y);
                ui.dialog = None;
                ui.menu = None;
                for i in out {
                    match i {
                        Intent::RevertPrefab { id, paths: Some(p) } if id == cube => {
                            reverted = Some(p);
                        }
                        Intent::ApplyPrefab(id) if id == cube => applied = true,
                        _ => {}
                    }
                }
            }
            y += 6.0;
        }
        assert_eq!(reverted, Some(vec!["mesh_visual.color".to_string()]));
        assert!(applied, "the Apply button");
        // Multi-selection draws its note without trouble.
        let light = doc.entities.iter().find(|e| e.id != cube).unwrap().id;
        doc.select_many(&[light, cube]);
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_profiler_tab_draws_its_graph() {
        let mut c = canvas();
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.panels.show_bottom = true;
        ui.bottom_tab = BottomTab::Profiler;
        for i in 0..50 {
            #[allow(clippy::cast_precision_loss)]
            ui.profiler.record(profiler::FrameSample {
                scripts: 1.0,
                physics: 2.0,
                render: 4.0 + (i % 5) as f32,
                ui: 3.0,
                total: 16.0,
            });
        }
        let before = c.pixels().to_vec();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        assert_ne!(before, c.pixels().to_vec());
        // Hovering a bar shows that frame's figures; Pause holds the graph.
        let dock = ui.layout(1600.0, 950.0).console;
        ui.frame(
            &mut c,
            &doc,
            Pointer {
                x: dock.x + dock.w - 30.0,
                y: dock.y + dock.h / 2.0,
                ..Pointer::default()
            },
            None,
        );
        ui.profiler.paused = true;
        ui.profiler.record(profiler::FrameSample::default());
        assert_eq!(ui.profiler.len(), 50);
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
