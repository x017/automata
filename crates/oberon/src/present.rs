//! Window presentation: the winit event loop that hosts the editor.
//!
//! Oberon renders its whole UI — panels plus the live engine viewport — into
//! an in-memory RGBA pixmap each frame (see [`crate::ui`]). This module owns
//! the OS window and pushes that pixmap to the screen through the engine's
//! wgpu surface ([`Renderer::blit_rgba_to_surface`]), so the entire path is
//! safe Rust: no OpenGL context, no `unsafe`.
//!
//! A real window needs a display server (X11/Wayland). In a headless
//! environment `run` reports the failure and returns; the editor's logic is
//! exercised headless via the `screenshot` example and the unit tests.

// This module uses only safe APIs (winit + wgpu surface). The crate-level
// safety policy is documented in `lib.rs`; nothing here needs `unsafe`.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use automata_renderer::Renderer;

use crate::canvas::Canvas;
use crate::doc::Document;
use crate::render::ViewportRenderer;
use crate::ui::{Intent, Pointer, Ui};

/// The editor window's initial logical size.
pub const WINDOW_SIZE: (u32, u32) = (1600, 950);

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

/// The running editor: document, UI state, and the window/GPU once resumed.
struct EditorApp {
    doc: Document,
    ui: Ui,
    canvas: Option<Canvas>,
    viewport: ViewportRenderer,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// Cursor state tracked between events to build each frame's `Pointer`.
    cursor: (f32, f32),
    mouse_down: bool,
    /// Set when a button was pressed since the last frame.
    pressed_pending: bool,
    scroll_pending: f32,
    /// Scene file the editor opened/saves to.
    scene_path: Option<PathBuf>,
    last_frame: Instant,
    fatal_error: Option<String>,
}

impl EditorApp {
    fn new(scene: Option<&std::path::Path>) -> Result<Self, String> {
        let (doc, scene_path) = match scene {
            Some(path) => (Document::load(path)?, Some(path.to_path_buf())),
            None => (Document::starter(), None),
        };
        Ok(Self {
            doc,
            ui: Ui::new(),
            canvas: None,
            viewport: ViewportRenderer::new(),
            window: None,
            renderer: None,
            cursor: (0.0, 0.0),
            mouse_down: false,
            pressed_pending: false,
            scroll_pending: 0.0,
            scene_path,
            last_frame: Instant::now(),
            fatal_error: None,
        })
    }

    /// Applies a UI intent to the document.
    fn apply(&mut self, intent: &Intent) {
        match *intent {
            Intent::Select(id) => self.doc.select(Some(id)),
            Intent::AddEntity => {
                self.doc.add_entity();
            }
            Intent::DeleteSelected => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.delete_entity(id);
                }
            }
            Intent::AddCube => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.add_mesh(id, true);
                }
            }
            Intent::AddLight => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.add_light(id);
                }
            }
            Intent::AddCamera => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.add_camera_3d(id);
                }
            }
            Intent::RemoveMesh => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.remove_mesh(id);
                }
            }
            Intent::RemoveLight => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.remove_light(id);
                }
            }
            Intent::Undo => {
                self.doc.undo();
            }
            Intent::Redo => {
                self.doc.redo();
            }
            Intent::New => {
                self.doc = Document::new();
                self.scene_path = None;
            }
            Intent::Save => match self.doc.save(self.scene_path.as_deref()) {
                Ok(path) => {
                    self.scene_path = Some(path);
                }
                Err(err) => {
                    tracing::error!(error = %err, "save failed");
                }
            },
            Intent::Orbit(dx, dy) => {
                self.ui.camera.yaw += dx * 0.01;
                self.ui.camera.pitch = (self.ui.camera.pitch + dy * 0.01).clamp(-1.5, 1.5);
            }
            Intent::Zoom(amount) => {
                self.ui.camera.distance = (self.ui.camera.distance - amount * 0.5).clamp(2.0, 60.0);
            }
            Intent::NudgeSelected(delta) => {
                if let Some(idx) = self.doc.selected_index() {
                    let id = self.doc.entities[idx].id;
                    self.doc.nudge_entity(id, delta);
                }
            }
        }
    }

    /// Renders one frame and presents it.
    fn render_frame(&mut self) {
        if self.window.is_none() || self.renderer.is_none() {
            return;
        }
        let (w, h) = {
            let size = self.window.as_ref().unwrap().inner_size();
            (size.width, size.height)
        };
        if w == 0 || h == 0 {
            return;
        }

        // 1) Draw the editor into the CPU pixmap and collect UI intents.
        let intents = {
            let canvas = self.canvas.get_or_insert_with(|| Canvas::new(w, h));
            if canvas.size() != (w, h) {
                canvas.resize(w, h);
            }

            // Build this frame's pointer from tracked cursor state.
            let pointer = Pointer {
                x: self.cursor.0,
                y: self.cursor.1,
                down: self.mouse_down,
                pressed: self.pressed_pending,
                scroll: self.scroll_pending,
            };

            // Render the live viewport (driven by the editor orbit camera),
            // then composite the whole UI over it.
            let (vw, vh) = viewport_pixels(w, h);
            let cam = self.ui.camera;
            let (frame, fw, fh) = self.viewport.render(&self.doc, cam, vw, vh);
            self.ui
                .frame(canvas, &self.doc, pointer, Some((&frame, fw, fh)))
        };

        // 2) Apply intents (mutates the document / UI camera). Drawing already
        //    captured this frame; intents take effect next frame.
        for intent in &intents {
            self.apply(intent);
        }

        // 3) Present the UI pixmap straight from the canvas (no copy).
        let (Some(renderer), Some(canvas)) = (&mut self.renderer, &self.canvas) else {
            return;
        };
        renderer.blit_rgba_to_surface(canvas.pixels(), canvas.size());

        // Per-frame transient input is consumed.
        self.pressed_pending = false;
        self.scroll_pending = 0.0;
        self.last_frame = Instant::now();
    }
}

/// The viewport's pixel dimensions within a window of `w`×`h`.
fn viewport_pixels(w: u32, h: u32) -> (u32, u32) {
    use crate::theme::layout;
    let body_h = h as f32 - layout::TOOLBAR_H - layout::STATUS_H;
    let view_w = w as f32 - layout::HIERARCHY_W - layout::INSPECTOR_W;
    let view_h = body_h - layout::CONSOLE_H;
    (view_w.max(1.0) as u32, view_h.max(1.0) as u32)
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
                (size.width, size.height),
                true, // vsync
                automata_renderer::GraphicsQuality::default(),
            ))
            .map_err(|e| format!("create renderer: {e}"))?;
            Ok((window, renderer))
        })();

        match result {
            Ok((window, renderer)) => {
                let (cw, ch) = size_of(&window);
                self.canvas = Some(Canvas::new(cw, ch));
                self.window = Some(window.clone());
                self.renderer = Some(renderer);
                event_loop.set_control_flow(ControlFlow::Poll);
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
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.mouse_down = state == ElementState::Pressed;
                if state == ElementState::Pressed {
                    self.pressed_pending = true;
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.05,
                };
                self.scroll_pending += amount;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Arrow keys nudge the selected entity in the camera's view
                // plane; PageUp/PageDown move it along world Y.
                if event.state == ElementState::Pressed {
                    let step = 0.25;
                    let cam = self.ui.camera;
                    let (sy, cy) = cam.yaw.sin_cos();
                    let delta = match event.physical_key {
                        // Camera-relative horizontal move on the ground plane.
                        PhysicalKey::Code(KeyCode::ArrowLeft) => Some([-cy * step, 0.0, sy * step]),
                        PhysicalKey::Code(KeyCode::ArrowRight) => Some([cy * step, 0.0, -sy * step]),
                        PhysicalKey::Code(KeyCode::ArrowUp) => Some([-sy * step, 0.0, -cy * step]),
                        PhysicalKey::Code(KeyCode::ArrowDown) => Some([sy * step, 0.0, cy * step]),
                        PhysicalKey::Code(KeyCode::PageUp) => Some([0.0, step, 0.0]),
                        PhysicalKey::Code(KeyCode::PageDown) => Some([0.0, -step, 0.0]),
                        _ => None,
                    };
                    if let Some(d) = delta {
                        self.apply(&Intent::NudgeSelected(d));
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Redraw at a steady cadence so hover/orbit animations stay live.
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        // Cap at ~60 FPS to avoid a busy loop when nothing changes.
        let frame = std::time::Duration::from_millis(16);
        let deadline = self.last_frame + frame;
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

/// Helper to read a window's current size for canvas allocation.
fn size_of(window: &Window) -> (u32, u32) {
    let s = window.inner_size();
    (s.width.max(1), s.height.max(1))
}
