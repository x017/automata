//! The winit-backed windowed runtime.
//!
//! [`WinitDriver`] implements winit's `ApplicationHandler` and drives the
//! engine's frame cycle from the OS event loop:
//!
//! 1. `resumed` — create the window and GPU context (once per process).
//! 2. `about_to_wait` — when the frame deadline arrives, advance time, run
//!    the game update, and request a redraw. The loop sleeps between
//!    deadlines via `ControlFlow::WaitUntil` when a target frame rate is
//!    configured, and spins with `ControlFlow::Poll` otherwise.
//! 3. `window_event` — feed input events to the input state, keep the
//!    surface size in sync, and present frames on `RedrawRequested`.
//!
//! Errors raised inside event-loop callbacks cannot be returned directly,
//! so the first fatal error is captured and reported by
//! [`crate::Application::run`] after the loop exits.

use std::sync::Arc;
use std::time::{Duration, Instant};

use automata_input::InputState;
use automata_renderer::RenderContext;
use tracing::{debug, info};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::app::AppContext;
use crate::config::EngineConfig;
use crate::error::{Error, Result};
use crate::time::Time;
use crate::window::WindowConfig;

/// The color the window is cleared to each frame until Phase 3 makes the
/// clear color configurable together with materials and cameras.
const CLEAR_COLOR: automata_renderer::Color = automata_renderer::Color {
    r: 0.02,
    g: 0.02,
    b: 0.04,
    a: 1.0,
};

/// Runs the windowed main loop to completion.
///
/// This is the implementation of [`crate::Application::run`] for
/// [`WindowMode::Windowed`]; see it for the contract.
pub(super) fn run_windowed(
    config: &EngineConfig,
    window_config: &WindowConfig,
    time: &mut Time,
    quit_requested: &mut bool,
    update: impl FnMut(&mut AppContext<'_>),
) -> Result<()> {
    info!("creating event loop");
    let event_loop = EventLoop::new().map_err(Error::EventLoop)?;

    let mut driver = WinitDriver {
        config,
        window_config,
        time,
        quit_requested,
        input: InputState::new(),
        window: None,
        renderer: None,
        update,
        last_frame_start: None,
        next_frame_deadline: None,
        fatal_error: None,
    };

    event_loop.run_app(&mut driver).map_err(Error::EventLoop)?;

    if let Some(error) = driver.fatal_error {
        return Err(error);
    }
    Ok(())
}

/// winit `ApplicationHandler` carrying the engine's runtime state.
struct WinitDriver<'a, F: FnMut(&mut AppContext<'_>)> {
    config: &'a EngineConfig,
    window_config: &'a WindowConfig,
    time: &'a mut Time,
    quit_requested: &'a mut bool,
    input: InputState,
    window: Option<Arc<Window>>,
    renderer: Option<RenderContext>,
    update: F,
    last_frame_start: Option<Instant>,
    next_frame_deadline: Option<Instant>,
    fatal_error: Option<Error>,
}

impl<F: FnMut(&mut AppContext<'_>)> WinitDriver<'_, F> {
    /// Runs one frame: advance time, update, clear per-frame input, redraw.
    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let delta = self
            .last_frame_start
            .map_or(Duration::ZERO, |start| now.saturating_duration_since(start));
        self.last_frame_start = Some(now);
        self.time.advance(delta);
        debug!(frame = self.time.frame_count(), "frame begin");

        self.update(&mut AppContext {
            time: self.time,
            input: &self.input,
            quit_requested: self.quit_requested,
        });
        self.input.end_frame();

        if *self.quit_requested {
            event_loop.exit();
            return;
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }

        if let Some(fps) = self.config.target_fps {
            let budget = Duration::from_secs_f64(1.0 / f64::from(fps));
            let deadline = now + budget;
            self.next_frame_deadline = Some(deadline);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            self.next_frame_deadline = None;
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }
}

impl<F: FnMut(&mut AppContext<'_>)> ApplicationHandler for WinitDriver<'_, F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let (width, height) = self.window_config.size;
        let attributes = Window::default_attributes()
            .with_title(self.config.title.as_str())
            .with_inner_size(LogicalSize::new(f64::from(width), f64::from(height)))
            .with_resizable(self.window_config.resizable);

        let result = (|| -> Result<(Arc<Window>, RenderContext)> {
            let window = Arc::new(event_loop.create_window(attributes).map_err(Error::Window)?);
            let size = window.inner_size();
            let renderer = pollster::block_on(RenderContext::new(
                window.clone(),
                (size.width, size.height),
                self.window_config.vsync,
            ))?;
            Ok((window, renderer))
        })();

        match result {
            Ok((window, renderer)) => {
                info!(
                    width = window.inner_size().width,
                    height = window.inner_size().height,
                    "window and GPU context ready"
                );
                self.window = Some(window);
                self.renderer = Some(renderer);
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
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    match renderer.render_clear(CLEAR_COLOR) {
                        Ok(()) => {}
                        Err(error) => {
                            self.fatal_error = Some(Error::Renderer(error));
                            event_loop.exit();
                        }
                    }
                }
            }
            event => self.input.handle_window_event(&event),
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
