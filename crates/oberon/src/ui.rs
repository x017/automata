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

use automata_core::{EntityData, TransformData};
use glam::Vec3;
use tiny_skia::Color;

use crate::canvas::Canvas;
use crate::doc::{ComponentKind, CreateKind, Document, EntityEntry};
use crate::fields::{self, Choice, Field, Flag};
use crate::theme::{color, layout};
use crate::view::{self, Gizmo, GizmoMode, Projector};

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

/// The editor's panel rectangles for a window of a given logical size.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Top toolbar.
    pub toolbar: Rect,
    /// Left hierarchy panel.
    pub hierarchy: Rect,
    /// Right inspector panel.
    pub inspector: Rect,
    /// Center scene view.
    pub viewport: Rect,
    /// Console under the scene view.
    pub console: Rect,
    /// Bottom status bar.
    pub status: Rect,
}

impl Layout {
    /// Computes the layout for a `w`×`h` logical window.
    #[must_use]
    pub fn compute(w: f32, h: f32) -> Self {
        let body_y = layout::TOOLBAR_H;
        let body_h = (h - layout::TOOLBAR_H - layout::STATUS_H).max(1.0);
        let hier_w = layout::HIERARCHY_W.min(w * 0.25);
        let insp_w = layout::INSPECTOR_W.min(w * 0.32);
        let center_w = (w - hier_w - insp_w).max(1.0);
        let console_h = layout::CONSOLE_H.min(body_h * 0.4);
        let view_h = (body_h - console_h).max(1.0);
        Self {
            toolbar: Rect::new(0.0, 0.0, w, layout::TOOLBAR_H),
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
}

impl Intent {
    /// Whether this intent edits the document (blocked during play mode).
    #[must_use]
    pub fn edits_document(&self) -> bool {
        !matches!(
            self,
            Self::Select(_) | Self::EndGesture | Self::Save | Self::Play | Self::Pause | Self::Stop
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Over a scrubbable number field.
    ResizeH,
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
}

#[derive(Debug, Clone)]
struct TextEdit {
    id: u64,
    target: EditTarget,
    buffer: String,
    rect: Rect,
    /// True until the first keystroke: typing replaces the whole value.
    fresh: bool,
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

#[derive(Debug, Clone, Copy)]
enum Menu {
    Create { x: f32, y: f32 },
    AddComponent { x: f32, y: f32, id: u64 },
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
    menu: Option<Menu>,
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
                self.edit = None;
                None
            }
        }
    }

    /// Starts renaming the selected entity (F2).
    pub fn begin_rename(&mut self, doc: &Document) {
        if let Some(i) = doc.selected_index() {
            let e = &doc.entities[i];
            self.edit = Some(TextEdit {
                id: e.id,
                target: EditTarget::Name,
                buffer: e.name().to_string(),
                rect: Rect::default(),
                fresh: true,
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
        }
    }

    // ----- frame ---------------------------------------------------------

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
        let lay = Layout::compute(w, h);
        self.cursor = CursorHint::Default;
        self.raw = pointer;
        self.p = pointer;
        self.clip = (0.0, h);

        // Clicking outside the focused text field commits it.
        if pointer.pressed {
            let outside = self
                .edit
                .as_ref()
                .is_some_and(|e| !e.rect.contains(pointer.x, pointer.y));
            if outside {
                if let Some(intent) = self.commit_edit() {
                    self.out.push(intent);
                }
            }
        }

        // An open menu captures the pointer; clicking outside closes it.
        let menu_rect = self.menu_rect(doc, w, h);
        if let Some(r) = menu_rect {
            let over = r.contains(pointer.x, pointer.y);
            if pointer.pressed && !over {
                self.menu = None;
                self.p.pressed = false;
            } else if over {
                self.p.pressed = false;
                self.p.scroll = 0.0;
                self.p.x = -1.0e6;
                self.p.y = -1.0e6;
            }
        }

        self.update_scrub();

        canvas.clear(color::background());
        // Scene view first: its overlays may spill outside its rect and are
        // covered by the panels drawn after it.
        self.scene_view(canvas, doc, lay.viewport, viewport);
        self.console_panel(canvas, lay.console);
        self.hierarchy(canvas, doc, lay.hierarchy);
        self.inspector(canvas, doc, lay.inspector);
        self.toolbar(canvas, doc, lay.toolbar);
        self.status_bar(canvas, doc, lay.status);
        self.clip = (0.0, h);
        self.draw_menu(canvas, doc, w, h);

        std::mem::take(&mut self.out)
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

        // Logo.
        let cy = rect.h / 2.0;
        canvas.fill_polygon(
            &[(12.0, cy), (19.0, cy - 7.0), (26.0, cy), (19.0, cy + 7.0)],
            color::accent(),
        );
        canvas.text(32.0, cy - 8.0, 15.0, "OBERON", color::accent_soft());

        let by = 8.0;
        let bh = rect.h - 16.0;
        let mut x = 120.0;
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

        // Document name + unsaved marker, right-aligned.
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
            format!("{name} *")
        } else {
            name
        };
        let room = rect.w - (px + group + 24.0);
        if room > 40.0 {
            let tw = canvas.measure_text(layout::FONT_SIZE, &label).min(room);
            canvas.text_clipped(
                rect.w - tw - layout::PAD,
                cy - 8.0,
                layout::FONT_SIZE,
                &label,
                color::text_dim(),
                room,
            );
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
        // Clicking empty space clears the selection.
        if !row_clicked && self.hot(list) && self.p.pressed {
            self.out.push(Intent::Select(None));
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
            self.menu = Some(Menu::Create {
                x: add.x,
                y: add.y + add.h + 2.0,
            });
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
            Some(idx) => self.inspector_body(canvas, &doc.entities[idx], x, start, w),
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
                y = self.color_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Color",
                    &[
                        Field::MeshColor(0),
                        Field::MeshColor(1),
                        Field::MeshColor(2),
                        Field::MeshColor(3),
                    ],
                );
                y += 6.0;
            }
        }
        if d.directional_light.is_some() {
            let kind = ComponentKind::Light;
            let (ny, open) = self.section(canvas, x, y, w, kind.label(), id, Some(kind), false);
            y = ny;
            if open {
                y = self.vec3_row(canvas, x, y, w, e, "Direction", Field::LightDirection);
                y = self.color_row(
                    canvas,
                    x,
                    y,
                    w,
                    e,
                    "Color",
                    &[
                        Field::LightColor(0),
                        Field::LightColor(1),
                        Field::LightColor(2),
                    ],
                );
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
                self.menu = Some(Menu::AddComponent {
                    x: add.x,
                    y: add.y + add.h + 2.0,
                    id,
                });
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
            if resettable {
                let r = Rect::new(right - 50.0, y + 3.0, 50.0, 18.0);
                let style = if editing {
                    ButtonStyle::Normal
                } else {
                    ButtonStyle::Disabled
                };
                if self.button(canvas, r, "Reset", style) {
                    self.out.push(Intent::ResetTransform(id));
                }
                right -= 54.0;
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
        channels: &[Field],
    ) -> f32 {
        let values: Vec<f32> = channels
            .iter()
            .filter_map(|f| fields::get(&e.data, *f))
            .collect();
        if values.len() != channels.len() || values.is_empty() {
            return y;
        }
        let r = Rect::new(x, y, w, layout::FIELD_H);
        if self.row_visible(r) {
            self.row_label(canvas, x, y, label);
            let sw = layout::FIELD_H;
            let sx = x + layout::LABEL_W;
            let rgb = |i: usize| values.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let swatch = Color::from_rgba(rgb(0), rgb(1), rgb(2), 1.0).unwrap_or(Color::WHITE);
            canvas.fill_rounded(sx, y, sw, sw, 3.0, swatch);
            canvas.stroke_rounded(sx, y, sw, sw, 3.0, color::border(), 1.0);
            let gap = 3.0;
            let n = channels.len() as f32;
            let fw = (w - layout::LABEL_W - sw - gap * n) / n;
            for (i, (f, v)) in channels.iter().zip(values.iter()).enumerate() {
                let fx = sx + sw + gap + i as f32 * (fw + gap);
                self.float_field(
                    canvas,
                    Rect::new(fx, y, fw, layout::FIELD_H),
                    e.id,
                    *f,
                    *v,
                    None,
                );
            }
        }
        y + layout::ROW_STEP
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
                    self.out.push(Intent::Cycle { id: e.id, choice });
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

    fn console_panel(&mut self, canvas: &mut Canvas, rect: Rect) {
        canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, color::surface());
        let line_h = 17.0;
        let body_top = rect.y + layout::TITLEBAR_H + 4.0;
        let max_lines = ((rect.h - layout::TITLEBAR_H - 8.0) / line_h)
            .floor()
            .max(0.0) as usize;
        let start = self.console.len().saturating_sub(max_lines);
        let mut y = body_top;
        for line in &self.console[start..] {
            let (c, tag) = match line.level {
                LogLevel::Info => (color::text_dim(), "•"),
                LogLevel::Warn => (color::warn(), "!"),
                LogLevel::Error => (color::danger(), "×"),
            };
            canvas.text(rect.x + layout::PAD, y, layout::SMALL_SIZE, tag, c);
            canvas.text_clipped(
                rect.x + layout::PAD + 14.0,
                y,
                layout::SMALL_SIZE,
                &line.text,
                c,
                rect.w - 2.0 * layout::PAD - 14.0,
            );
            y += line_h;
        }
        let title = format!("Console  ({})", self.console.len());
        self.panel_title(canvas, rect, &title);
        let clear = Rect::new(
            rect.x + rect.w - 64.0,
            rect.y + 4.0,
            56.0,
            layout::TITLEBAR_H - 8.0,
        );
        if self.button(canvas, clear, "Clear", ButtonStyle::Normal) {
            self.console.clear();
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

    // ----- menus -----------------------------------------------------------

    fn menu_items(&self, doc: &Document) -> Vec<(&'static str, Intent)> {
        match self.menu {
            None => Vec::new(),
            Some(Menu::Create { .. }) => CreateKind::ALL
                .iter()
                .map(|k| (k.label(), Intent::Create(*k)))
                .collect(),
            Some(Menu::AddComponent { id, .. }) => match doc.index_of(id) {
                Some(i) => ComponentKind::ALL
                    .iter()
                    .filter(|k| !k.present_in(&doc.entities[i].data))
                    .map(|k| (k.label(), Intent::AddComponent(id, *k)))
                    .collect(),
                None => Vec::new(),
            },
        }
    }

    fn menu_rect(&self, doc: &Document, w: f32, h: f32) -> Option<Rect> {
        let (x, y) = match self.menu? {
            Menu::Create { x, y } | Menu::AddComponent { x, y, .. } => (x, y),
        };
        let n = self.menu_items(doc).len().max(1) as f32;
        let mh = n * layout::ROW_H + 8.0;
        let mw = layout::MENU_W;
        let x = x.min(w - mw - 4.0).max(0.0);
        let y = if y + mh > h {
            (y - mh - 30.0).max(0.0)
        } else {
            y
        };
        Some(Rect::new(x, y, mw, mh))
    }

    fn draw_menu(&mut self, canvas: &mut Canvas, doc: &Document, w: f32, h: f32) {
        let Some(r) = self.menu_rect(doc, w, h) else {
            return;
        };
        let items = self.menu_items(doc);
        canvas.fill_rounded(r.x + 3.0, r.y + 4.0, r.w, r.h, 6.0, color::shadow());
        canvas.fill_rounded(r.x, r.y, r.w, r.h, 6.0, color::raised());
        canvas.stroke_rounded(r.x, r.y, r.w, r.h, 6.0, color::border(), 1.0);
        let raw = self.raw;
        if items.is_empty() {
            canvas.text(
                r.x + 12.0,
                r.y + 9.0,
                layout::FONT_SIZE,
                "All components added",
                color::text_dim(),
            );
            return;
        }
        let mut chosen = None;
        for (i, (label, intent)) in items.into_iter().enumerate() {
            let row = Rect::new(
                r.x + 4.0,
                r.y + 4.0 + i as f32 * layout::ROW_H,
                r.w - 8.0,
                layout::ROW_H,
            );
            let hot = row.contains(raw.x, raw.y);
            if hot {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, 4.0, color::select());
                self.cursor = CursorHint::Pointer;
            }
            canvas.text(
                row.x + 10.0,
                row.y + 5.0,
                layout::FONT_SIZE,
                label,
                color::text(),
            );
            if hot && raw.pressed {
                chosen = Some(intent);
            }
        }
        if let Some(intent) = chosen {
            self.out.push(intent);
            self.menu = None;
        }
    }
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
        let r = ui.menu_rect(&doc, 1600.0, 950.0).unwrap();
        // Second item: Cube.
        let item = (r.x + 30.0, r.y + 4.0 + layout::ROW_H * 1.5);
        let out = click(&mut ui, &mut c, &doc, item.0, item.1);
        assert!(out.contains(&Intent::Create(CreateKind::Cube)), "{out:?}");
        assert!(ui.menu.is_none());
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
