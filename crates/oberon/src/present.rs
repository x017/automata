//! Window presentation: the winit event loop that hosts the editor.
//!
//! Oberon renders its whole UI — panels plus the live engine viewport — into
//! an in-memory RGBA pixmap each frame (see [`crate::ui`]). This module owns
//! the OS window, turns OS input into the UI's per-frame [`Pointer`] and
//! keyboard calls, applies the [`Intent`]s the UI returns to the document
//! (with undo), runs play mode, and pushes the pixmap to the screen through
//! the engine's wgpu surface ([`Renderer::blit_rgba_to_surface`]). The entire
//! path is safe Rust.
//!
//! A real window needs a display server (X11/Wayland). In a headless
//! environment `run` reports the failure and returns; the editor's logic is
//! exercised headless via the `screenshot` example and the unit tests.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use automata_renderer::Renderer;

use crate::canvas::Canvas;
use crate::doc::Document;
use crate::fields;
use crate::play::PlaySession;
use crate::render::ViewportRenderer;
use crate::ui::{CursorHint, EditKey, Intent, Layout, LogLevel, PlayState, Pointer, Ui};
use crate::view::GizmoMode;

/// The editor window's initial logical size.
pub const WINDOW_SIZE: (u32, u32) = (1600, 950);

/// Two presses closer than this (and a few pixels apart) are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Runs the editor window. Blocks until the window is closed.
///
/// `scene`, when given, is the RON scene file to open (and save back to).
/// Returns `Ok(())` on a clean exit, or an error string describing why the
/// window could not be presented (no display server, no GPU, …).
pub fn run(scene: Option<&std::path::Path>) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;

    let mut app = EditorApp::new(scene)?;
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop run: {e}"))?;

    if let Some(err) = app.fatal_error {
        return Err(err);
    }
    Ok(())
}

/// Mouse and keyboard state accumulated between frames.
#[derive(Debug, Default)]
struct Input {
    /// Cursor position in physical pixels.
    cursor: (f32, f32),
    left: bool,
    right: bool,
    middle: bool,
    /// A left press happened since the last frame.
    pressed: bool,
    /// That press was a double click.
    double_click: bool,
    last_press: Option<(Instant, (f32, f32))>,
    scroll: f32,
    modifiers: ModifiersState,
}

impl Input {
    fn ctrl(&self) -> bool {
        self.modifiers.control_key() || self.modifiers.super_key()
    }
}

/// The running editor: document, UI state, and the window/GPU once resumed.
struct EditorApp {
    doc: Document,
    ui: Ui,
    canvas: Option<Canvas>,
    viewport: ViewportRenderer,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    input: Input,
    /// The running simulation while in play mode.
    play: Option<PlaySession>,
    last_frame: Instant,
    last_title: String,
    last_cursor: CursorHint,
    fatal_error: Option<String>,
}

impl EditorApp {
    fn new(scene: Option<&std::path::Path>) -> Result<Self, String> {
        let doc = match scene {
            Some(path) => Document::load(path)?,
            None => Document::starter(),
        };
        let mut ui = Ui::new();
        ui.log(LogLevel::Info, "Oberon ready. Click an object to select it; drag the gizmo to move it.");
        ui.log(
            LogLevel::Info,
            "Edit values in the Inspector: drag a number to scrub it, or click to type.",
        );
        match &doc.path {
            Some(path) => ui.log(LogLevel::Info, format!("Opened {}", path.display())),
            None => ui.log(LogLevel::Info, "New scene (unsaved). Ctrl+S saves it."),
        }
        Ok(Self {
            doc,
            ui,
            canvas: None,
            viewport: ViewportRenderer::new(),
            window: None,
            renderer: None,
            input: Input::default(),
            play: None,
            last_frame: Instant::now(),
            last_title: String::new(),
            last_cursor: CursorHint::Default,
            fatal_error: None,
        })
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    // ----- intents -------------------------------------------------------

    /// Applies a UI intent to the document (or play state).
    fn apply(&mut self, intent: Intent) {
        if intent.edits_document() && self.ui.play != PlayState::Editing {
            return; // the scene is read-only while simulating
        }
        let doc = &mut self.doc;
        match intent {
            Intent::Select(id) => {
                doc.select(id);
                if let Some(play) = &mut self.play {
                    play.doc.select(id);
                }
            }
            Intent::Create(kind) => {
                let id = doc.create(kind);
                let name = doc.index_of(id).map(|i| doc.entities[i].name().to_string());
                self.ui.log(LogLevel::Info, format!("Created {}", name.unwrap_or_default()));
            }
            Intent::Duplicate(id) => {
                if let Some(new_id) = doc.duplicate_entity(id) {
                    let name = doc.index_of(new_id).map(|i| doc.entities[i].name().to_string());
                    self.ui.log(LogLevel::Info, format!("Duplicated as {}", name.unwrap_or_default()));
                }
            }
            Intent::Delete(id) => {
                let name = doc.index_of(id).map(|i| doc.entities[i].name().to_string());
                if doc.delete_entity(id) {
                    self.ui.log(LogLevel::Info, format!("Deleted {}", name.unwrap_or_default()));
                }
            }
            Intent::AddComponent(id, kind) => {
                if doc.add_component(id, kind) {
                    self.ui.log(LogLevel::Info, format!("Added {}", kind.label()));
                }
            }
            Intent::RemoveComponent(id, kind) => {
                if doc.remove_component(id, kind) {
                    self.ui.log(LogLevel::Info, format!("Removed {}", kind.label()));
                }
            }
            Intent::SetField {
                id,
                field,
                value,
                gesture,
            } => {
                doc.modify(id, gesture, |d| {
                    fields::set(d, field, value);
                });
            }
            Intent::SetTransform { id, transform } => {
                doc.modify(id, true, |d| d.transform = Some(transform));
            }
            Intent::Rename { id, name } => {
                doc.rename_entity(id, &name);
            }
            Intent::Toggle { id, flag } => {
                doc.modify(id, false, |d| {
                    fields::toggle(d, flag);
                });
            }
            Intent::Cycle { id, choice } => {
                doc.modify(id, false, |d| {
                    fields::cycle(d, choice);
                });
            }
            Intent::ResetTransform(id) => {
                doc.reset_transform(id);
            }
            Intent::EndGesture => doc.end_gesture(),
            Intent::NudgeSelected(delta) => {
                if let Some(i) = doc.selected_index() {
                    let id = doc.entities[i].id;
                    doc.nudge_entity(id, delta);
                }
            }
            Intent::Undo => {
                if !doc.undo() {
                    self.ui.log(LogLevel::Warn, "Nothing to undo");
                }
            }
            Intent::Redo => {
                if !doc.redo() {
                    self.ui.log(LogLevel::Warn, "Nothing to redo");
                }
            }
            Intent::Save => self.save(),
            Intent::New => {
                self.doc = Document::starter();
                self.viewport.invalidate();
                self.ui.log(LogLevel::Info, "New scene");
            }
            Intent::Play => self.play(),
            Intent::Pause => match self.ui.play {
                PlayState::Playing => self.set_paused(true),
                PlayState::Paused => self.set_paused(false),
                PlayState::Editing => {}
            },
            Intent::Stop => self.stop(),
        }
    }

    fn save(&mut self) {
        let path = self.doc.path.clone().unwrap_or_else(default_save_path);
        match self.doc.save(Some(path.as_path())) {
            Ok(path) => self.ui.log(LogLevel::Info, format!("Saved {}", path.display())),
            Err(err) => {
                tracing::error!(error = %err, "save failed");
                self.ui.log(LogLevel::Error, format!("Save failed: {err}"));
            }
        }
    }

    fn play(&mut self) {
        match self.ui.play {
            PlayState::Editing => {
                self.doc.end_gesture();
                let session = PlaySession::start(&self.doc);
                let bodies = session.body_count();
                self.play = Some(session);
                self.ui.play = PlayState::Playing;
                self.ui.log(
                    LogLevel::Info,
                    format!("Play mode: simulating {bodies} rigid bodies. Stop restores the scene."),
                );
                if bodies == 0 {
                    self.ui.log(
                        LogLevel::Warn,
                        "No rigid bodies: add Rigid Body + Collider components (or + Create → Physics Cube).",
                    );
                }
            }
            PlayState::Paused => self.set_paused(false),
            PlayState::Playing => {}
        }
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some(play) = &mut self.play {
            play.paused = paused;
            self.ui.play = if paused {
                PlayState::Paused
            } else {
                PlayState::Playing
            };
        }
    }

    fn stop(&mut self) {
        if let Some(play) = self.play.take() {
            self.ui.log(
                LogLevel::Info,
                format!("Stopped after {:.1}s. Scene restored.", play.elapsed),
            );
        }
        self.ui.play = PlayState::Editing;
    }

    // ----- keyboard ------------------------------------------------------

    fn keyboard(&mut self, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        if event.state == ElementState::Released {
            // Releasing an arrow key ends the nudge gesture (one undo step
            // per key hold).
            if is_nudge_key(code) {
                self.doc.end_gesture();
            }
            return;
        }

        // A focused text field takes the keyboard.
        if self.ui.is_editing_text() {
            let key = match code {
                KeyCode::Backspace => Some(EditKey::Backspace),
                KeyCode::Delete => Some(EditKey::Delete),
                KeyCode::Enter | KeyCode::NumpadEnter => Some(EditKey::Enter),
                KeyCode::Tab => Some(EditKey::Tab),
                KeyCode::Escape => Some(EditKey::Escape),
                _ => None,
            };
            match key {
                Some(key) => {
                    if let Some(intent) = self.ui.edit_key(key) {
                        self.apply(intent);
                    }
                }
                None if !self.input.ctrl() => {
                    if let Some(text) = &event.text {
                        self.ui.type_text(text);
                    }
                }
                None => {}
            }
            return;
        }

        let ctrl = self.input.ctrl();
        let shift = self.input.modifiers.shift_key();
        let selected = self.doc.selected_index().map(|i| self.doc.entities[i].id);

        // Nudging repeats while held.
        if !ctrl {
            if let Some(delta) = self.nudge_delta(code) {
                self.apply(Intent::NudgeSelected(delta));
                return;
            }
        }
        if event.repeat {
            return;
        }
        let intent = match (code, ctrl) {
            (KeyCode::KeyZ, true) if shift => Some(Intent::Redo),
            (KeyCode::KeyZ, true) => Some(Intent::Undo),
            (KeyCode::KeyY, true) => Some(Intent::Redo),
            (KeyCode::KeyS, true) => Some(Intent::Save),
            (KeyCode::KeyN, true) => Some(Intent::New),
            (KeyCode::KeyD, true) => selected.map(Intent::Duplicate),
            (KeyCode::KeyP, true) => Some(if self.ui.play == PlayState::Editing {
                Intent::Play
            } else {
                Intent::Stop
            }),
            (KeyCode::Delete | KeyCode::Backspace, false) => selected.map(Intent::Delete),
            (KeyCode::Escape, false) => Some(Intent::Select(None)),
            (KeyCode::KeyF, false) => {
                self.focus_selected();
                None
            }
            (KeyCode::F2, false) => {
                if self.ui.play == PlayState::Editing {
                    self.ui.begin_rename(&self.doc);
                }
                None
            }
            (KeyCode::KeyW, false) => {
                self.ui.gizmo = GizmoMode::Move;
                None
            }
            (KeyCode::KeyE, false) => {
                self.ui.gizmo = GizmoMode::Rotate;
                None
            }
            (KeyCode::KeyR, false) => {
                self.ui.gizmo = GizmoMode::Scale;
                None
            }
            _ => None,
        };
        if let Some(intent) = intent {
            self.apply(intent);
        }
    }

    fn focus_selected(&mut self) {
        match &self.play {
            Some(play) => self.ui.focus_selected(&play.doc),
            None => self.ui.focus_selected(&self.doc),
        }
    }

    /// Arrow keys move the selection on the ground plane relative to the
    /// camera; PageUp/PageDown move it along world Y. Shift = fine steps.
    fn nudge_delta(&self, code: KeyCode) -> Option<[f32; 3]> {
        let step = if self.input.modifiers.shift_key() { 0.05 } else { 0.25 };
        let (sy, cy) = self.ui.camera.yaw.sin_cos();
        Some(match code {
            KeyCode::ArrowLeft => [-cy * step, 0.0, sy * step],
            KeyCode::ArrowRight => [cy * step, 0.0, -sy * step],
            KeyCode::ArrowUp => [-sy * step, 0.0, -cy * step],
            KeyCode::ArrowDown => [sy * step, 0.0, cy * step],
            KeyCode::PageUp => [0.0, step, 0.0],
            KeyCode::PageDown => [0.0, -step, 0.0],
            _ => return None,
        })
    }

    // ----- frame ---------------------------------------------------------

    /// Steps play mode, renders one frame, applies the UI's intents, and
    /// presents.
    fn render_frame(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let (w, h) = (size.width, size.height);
        if w == 0 || h == 0 || self.renderer.is_none() {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if let Some(play) = &mut self.play {
            play.update(dt);
        }

        let scale = window.scale_factor() as f32;
        let pointer = self.pointer(scale);

        let intents = {
            let Self {
                doc,
                ui,
                canvas,
                viewport,
                renderer,
                play,
                ..
            } = self;
            let canvas = canvas.get_or_insert_with(|| Canvas::new(w, h));
            if canvas.size() != (w, h) {
                canvas.resize(w, h);
            }
            canvas.set_scale(scale);

            // The scene view's size in physical pixels: the GPU renders at
            // full resolution and the UI composites it 1:1.
            let (lw, lh) = canvas.logical_size();
            let view = Layout::compute(lw, lh).viewport;
            let s = canvas.scale();
            let vw = (view.w * s).round().max(1.0) as u32;
            let vh = (view.h * s).round().max(1.0) as u32;

            let cam = ui.camera;
            match play {
                Some(session) => {
                    let (frame, fw, fh) =
                        viewport.render_play(renderer.as_mut(), session, cam, vw, vh);
                    ui.frame(canvas, &session.doc, pointer, Some((frame, fw, fh)))
                }
                None => {
                    let (frame, fw, fh) = viewport.render(renderer.as_mut(), doc, cam, vw, vh);
                    ui.frame(canvas, doc, pointer, Some((frame, fw, fh)))
                }
            }
        };

        // Per-frame transient input is consumed.
        self.input.pressed = false;
        self.input.double_click = false;
        self.input.scroll = 0.0;

        // Intents take effect next frame (this one is already drawn).
        for intent in intents {
            self.apply(intent);
        }

        if let (Some(renderer), Some(canvas)) = (&mut self.renderer, &self.canvas) {
            renderer.blit_rgba_to_surface(canvas.pixels(), canvas.size());
        }
        self.update_window_chrome(&window);
    }

    /// This frame's pointer in logical pixels.
    fn pointer(&self, scale: f32) -> Pointer {
        let i = &self.input;
        let s = scale.max(0.1);
        Pointer {
            x: i.cursor.0 / s,
            y: i.cursor.1 / s,
            down: i.left,
            pressed: i.pressed,
            double_click: i.double_click,
            right_down: i.right,
            middle_down: i.middle,
            scroll: i.scroll,
            shift: i.modifiers.shift_key(),
            ctrl: i.ctrl(),
            alt: i.modifiers.alt_key(),
        }
    }

    /// Keeps the window title and mouse cursor in sync with the editor.
    fn update_window_chrome(&mut self, window: &Window) {
        let name = self.doc.path.as_ref().map_or_else(
            || "Untitled".to_string(),
            |p| {
                p.file_name()
                    .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
            },
        );
        let mode = match self.ui.play {
            PlayState::Editing => "",
            PlayState::Playing => " [Playing]",
            PlayState::Paused => " [Paused]",
        };
        let dirty = if self.doc.dirty() { "*" } else { "" };
        let title = format!("{name}{dirty}{mode} — Oberon");
        if title != self.last_title {
            window.set_title(&title);
            self.last_title = title;
        }
        let cursor = self.ui.cursor();
        if cursor != self.last_cursor {
            window.set_cursor(match cursor {
                CursorHint::Default => CursorIcon::Default,
                CursorHint::Pointer => CursorIcon::Pointer,
                CursorHint::Text => CursorIcon::Text,
                CursorHint::ResizeH => CursorIcon::EwResize,
                CursorHint::Grab => CursorIcon::Grab,
                CursorHint::Grabbing => CursorIcon::Grabbing,
                CursorHint::Move => CursorIcon::Move,
            });
            self.last_cursor = cursor;
        }
    }
}

fn is_nudge_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::ArrowUp
            | KeyCode::ArrowDown
            | KeyCode::PageUp
            | KeyCode::PageDown
    )
}

/// `scene.ron` in the working directory, or `scene-2.ron`, … if taken.
fn default_save_path() -> PathBuf {
    let first = PathBuf::from("scene.ron");
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|i| PathBuf::from(format!("scene-{i}.ron")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

impl ApplicationHandler for EditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Oberon — AUTOMATA Scene Editor")
            .with_inner_size(LogicalSize::new(
                f64::from(WINDOW_SIZE.0),
                f64::from(WINDOW_SIZE.1),
            ))
            .with_min_inner_size(LogicalSize::new(800.0, 500.0))
            .with_resizable(true);

        let result = (|| -> Result<(Arc<Window>, Renderer), String> {
            let window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .map_err(|e| format!("create window: {e}"))?,
            );
            let size = window.inner_size();
            let renderer = pollster::block_on(Renderer::new(
                window.clone(),
                (size.width.max(1), size.height.max(1)),
                true, // vsync
                automata_renderer::GraphicsQuality::default(),
            ))
            .map_err(|e| format!("create renderer: {e}"))?;
            Ok((window, renderer))
        })();

        match result {
            Ok((window, renderer)) => {
                let size = window.inner_size();
                let mut canvas = Canvas::new(size.width.max(1), size.height.max(1));
                canvas.set_scale(window.scale_factor() as f32);
                self.canvas = Some(canvas);
                self.window = Some(window.clone());
                self.renderer = Some(renderer);
                self.last_frame = Instant::now();
                window.request_redraw();
            }
            Err(err) => {
                self.fatal_error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if self.doc.dirty() {
                    tracing::warn!("closing with unsaved changes");
                }
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {}
            WindowEvent::Focused(false) => {
                // Buttons released outside the window never report; don't
                // leave a drag stuck.
                self.input.left = false;
                self.input.right = false;
                self.input.middle = false;
                self.input.modifiers = ModifiersState::empty();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.input.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        self.input.left = down;
                        if down {
                            let now = Instant::now();
                            let at = self.input.cursor;
                            let double = self.input.last_press.is_some_and(|(t, p)| {
                                now.duration_since(t) < DOUBLE_CLICK
                                    && (p.0 - at.0).abs() < 6.0
                                    && (p.1 - at.1).abs() < 6.0
                            });
                            self.input.pressed = true;
                            self.input.double_click = double;
                            // A third click starts a new sequence.
                            self.input.last_press = if double { None } else { Some((now, at)) };
                        }
                    }
                    MouseButton::Right => self.input.right = down,
                    MouseButton::Middle => self.input.middle = down,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.keyboard(&event);
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
                return;
            }
            _ => return,
        }
        self.request_redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Redraw at a steady ~60 FPS so play mode, hover and drags stay live
        // without spinning the CPU between frames.
        let deadline = self.last_frame + Duration::from_millis(16);
        if Instant::now() >= deadline {
            self.request_redraw();
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }
    }
}
