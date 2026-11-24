//! The winit-backed desktop driver.
//!
//! [`WinitDriver`] implements winit's `ApplicationHandler` around the
//! shared [`Runtime`](crate::runtime::Runtime):
//!
//! 1. `resumed`: create the window and hand its surface to the runtime
//!    (once per process).
//! 2. `about_to_wait`: when the frame deadline arrives, run a frame and
//!    request a redraw. The loop sleeps between deadlines via
//!    `ControlFlow::WaitUntil` when a target frame rate is configured, and
//!    spins with `ControlFlow::Poll` otherwise.
//! 3. `window_event`: translate input (see [`crate::winit_input`]), keep
//!    the surface size in sync, and present frames on `RedrawRequested`.
//!
//! Errors raised inside event-loop callbacks cannot be returned directly,
//! so the first fatal error is captured and reported by
//! [`crate::Application::run`] after the loop exits.

use std::sync::Arc;
use std::time::Instant;

use automata_input::InputEvent;
use tracing::info;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::error::{Error, Result};
use crate::runtime::{FrameStep, Host};

/// Runs the windowed main loop to completion with winit.
pub(crate) fn run(host: &mut dyn Host) -> Result<()> {
    info!("creating event loop");
    let event_loop = EventLoop::new().map_err(Error::EventLoop)?;
    let mut driver = WinitDriver {
        host,
        focused: true,
        cursor_applied: false,
        window: None,
        next_frame_deadline: None,
        fatal_error: None,
    };
    event_loop.run_app(&mut driver).map_err(Error::EventLoop)?;
    match driver.fatal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// winit `ApplicationHandler` around the engine's runtime.
struct WinitDriver<'a> {
    host: &'a mut dyn Host,
    /// Whether the window has keyboard focus (raw mouse motion is ignored
    /// otherwise: X11 reports it for the whole desktop).
    focused: bool,
    /// The cursor lock last applied to the window.
    cursor_applied: bool,
    window: Option<Arc<Window>>,
    next_frame_deadline: Option<Instant>,
    fatal_error: Option<Error>,
}

impl WinitDriver<'_> {
    /// Runs one frame and schedules the next.
    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        let step = self.host.frame();
        self.apply_cursor_lock();
        match step {
            FrameStep::Quit => event_loop.exit(),
            FrameStep::Next(deadline) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                self.next_frame_deadline = deadline;
                event_loop
                    .set_control_flow(deadline.map_or(ControlFlow::Poll, ControlFlow::WaitUntil));
            }
        }
    }

    /// Applies the game's cursor-lock request to the window when it changed.
    fn apply_cursor_lock(&mut self) {
        let wanted = self.host.cursor_locked() && self.focused;
        if wanted == self.cursor_applied {
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        self.cursor_applied = wanted;
        lock_cursor(window, wanted);
    }

    /// Makes the window and gives its surface to the runtime.
    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<Arc<Window>> {
        let config = self.host.window_config();
        let (width, height) = config.size;
        let attributes = Window::default_attributes()
            .with_title(self.host.title())
            .with_inner_size(LogicalSize::new(f64::from(width), f64::from(height)))
            .with_resizable(config.resizable);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(Error::Window)?,
        );
        let size = window.inner_size();
        self.host
            .attach_surface(window.clone().into(), (size.width, size.height))?;
        Ok(window)
    }
}

/// Hides and captures the cursor (or releases it). Platforms without a
/// true lock (X11) get a confined cursor; raw mouse motion still drives
/// mouse-look either way.
fn lock_cursor(window: &Window, locked: bool) {
    use winit::window::CursorGrabMode;
    if locked {
        if window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
            let _ = window.set_cursor_grab(CursorGrabMode::Confined);
        }
    } else {
        let _ = window.set_cursor_grab(CursorGrabMode::None);
    }
    window.set_cursor_visible(!locked);
}

impl ApplicationHandler for WinitDriver<'_> {
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            if self.focused {
                self.host.input(&InputEvent::MouseMotion {
                    dx: delta.0,
                    dy: delta.1,
                });
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match self.create_window(event_loop) {
            Ok(window) => {
                self.window = Some(window);
                // Kick off the frame cycle; ControlFlow::Wait is the default
                // and would never call `about_to_wait`.
                event_loop.set_control_flow(ControlFlow::Poll);
            }
            Err(error) => {
                self.fatal_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                info!("window close requested");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => self.host.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.host.render() {
                    self.fatal_error = Some(error);
                    event_loop.exit();
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                // The OS drops the grab when focus leaves; re-apply on return.
                self.cursor_applied = false;
                if !focused {
                    if let Some(window) = &self.window {
                        lock_cursor(window, false);
                    }
                    // Releases that happen elsewhere never arrive.
                    self.host.input(&InputEvent::FocusLost);
                }
                self.apply_cursor_lock();
            }
            event => {
                if let Some(input) = crate::winit_input::input_event(&event) {
                    self.host.input(&input);
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Wait until the frame deadline; winit may wake us earlier for
        // unrelated events, in which case we go back to sleep.
        if let Some(deadline) = self.next_frame_deadline {
            if Instant::now() < deadline {
                return;
            }
        }
        self.run_frame(event_loop);
    }
}
