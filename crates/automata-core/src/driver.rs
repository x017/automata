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
use automata_renderer::{DrawList, Renderer};
use tracing::{debug, info};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::app::{AppContext, WorldState};
use crate::config::EngineConfig;
use crate::error::{Error, Result};
use crate::time::Time;
use crate::window::WindowConfig;

/// Runs the windowed main loop to completion.
///
/// This is the implementation of [`crate::Application::run`] for
/// [`WindowMode::Windowed`]; see it for the contract.
pub(super) fn run_windowed(
    config: &EngineConfig,
    window_config: &WindowConfig,
    time: &mut Time,
    world: &mut WorldState,
    quit_requested: &mut bool,
    update: impl FnMut(&mut AppContext<'_>),
) -> Result<()> {
    info!("creating event loop");
    let event_loop = EventLoop::new().map_err(Error::EventLoop)?;

    let mut draw = DrawList::new();
    draw.set_clear_color(config.clear_color);

    let mut driver = WinitDriver {
        config,
        window_config,
        time,
        world,
        quit_requested,
        input: InputState::new(),
        gamepads: automata_input::Gamepads::new(),
        focused: true,
        cursor_applied: false,
        window: None,
        renderer: None,
        draw,
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
    world: &'a mut WorldState,
    quit_requested: &'a mut bool,
    input: InputState,
    gamepads: automata_input::Gamepads,
    /// Whether the window has keyboard focus (raw mouse motion is ignored
    /// otherwise: X11 reports it for the whole desktop).
    focused: bool,
    /// The cursor lock last applied to the window.
    cursor_applied: bool,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    draw: DrawList,
    update: F,
    last_frame_start: Option<Instant>,
    next_frame_deadline: Option<Instant>,
    fatal_error: Option<Error>,
}

impl<F: FnMut(&mut AppContext<'_>)> WinitDriver<'_, F> {
    /// Runs one frame: advance time, age events, update, propagate scene
    /// state, record the draw list, and request a redraw.
    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let delta = self
            .last_frame_start
            .map_or(Duration::ZERO, |start| now.saturating_duration_since(start));
        self.last_frame_start = Some(now);
        self.time.advance(delta);
        self.world.events_mut().update();
        debug!(frame = self.time.frame_count(), "frame begin");

        self.gamepads.update();
        for event in self.gamepads.drain_events() {
            self.world.events_mut().send(event);
        }
        if let Some(timestep) = self.config.physics_timestep {
            self.world.step_physics(delta.as_secs_f32(), timestep);
        }
        self.world.pump_audio(delta.as_secs_f64());

        let renderer_ref = self.renderer.as_ref();
        (self.update)(&mut AppContext::new(
            self.time,
            &self.input,
            &self.gamepads,
            &mut self.draw,
            renderer_ref,
            self.world,
            self.quit_requested,
        ));
        self.input.end_frame();
        self.apply_cursor_lock();
        if let Some(quality) = self.world.take_graphics_change() {
            if let Some(renderer) = &mut self.renderer {
                info!(?quality, "graphics quality changed");
                renderer.set_quality(quality);
            }
        }

        // Built-in scene systems (transform propagation) and the
        // scene-to-renderer bridge run after the game update. Plain-data
        // MeshVisual components become GPU-backed MeshRenderers here, the
        // first frame a renderer is available.
        self.world.scene_mut().update();
        if let Some(renderer) = &self.renderer {
            automata_scene::materialize_visuals(self.world.scene_mut().world_mut(), renderer);
        }
        automata_scene::animate(self.world.scene_mut().world_mut(), delta.as_secs_f32());
        automata_scene::record_draw_list(self.world.scene().world(), &mut self.draw);
        // Waves run on the physics clock, so floating bodies ride exactly
        // the waves that are drawn.
        self.draw.set_time(self.world.physics3d_ref().water_time());

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

impl<F: FnMut(&mut AppContext<'_>)> WinitDriver<'_, F> {
    /// Applies the game's cursor-lock request to the window when it changed.
    fn apply_cursor_lock(&mut self) {
        let wanted = self.world.cursor_locked() && self.focused;
        if wanted == self.cursor_applied {
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        self.cursor_applied = wanted;
        lock_cursor(window, wanted);
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

impl<F: FnMut(&mut AppContext<'_>)> ApplicationHandler for WinitDriver<'_, F> {
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            if self.focused {
                self.input.handle_mouse_motion(delta.0, delta.1);
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let (width, height) = self.window_config.size;
        let attributes = Window::default_attributes()
            .with_title(self.config.title.as_str())
            .with_inner_size(LogicalSize::new(f64::from(width), f64::from(height)))
            .with_resizable(self.window_config.resizable);

        let result = (|| -> Result<(Arc<Window>, Renderer)> {
            let window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .map_err(Error::Window)?,
            );
            let size = window.inner_size();
            let renderer = pollster::block_on(Renderer::new(
                window.clone(),
                (size.width, size.height),
                self.window_config.vsync,
                self.world.graphics_quality(),
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
                    match renderer.render(&self.draw) {
                        Ok(()) => self.draw.clear(),
                        Err(error) => {
                            self.fatal_error = Some(Error::Renderer(error));
                            event_loop.exit();
                        }
                    }
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
                }
                self.apply_cursor_lock();
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
