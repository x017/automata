//! Application lifecycle and the main loop.
//!
//! An [`Application`] moves through a strict lifecycle:
//!
//! ```text
//! Created ──run()──▶ Running ──quit requested / window closed──▶ Shutdown
//! ```
//!
//! Each state transition is logged. Running an application more than once is
//! an error: resources are torn down during shutdown and cannot be reused.
//!
//! The loop is driven by the winit event loop in
//! [`WindowMode::Windowed`] and by a sleep-paced headless loop in
//! [`WindowMode::Headless`]; the public API is identical either way.

use std::thread;
use std::time::{Duration, Instant};

use automata_input::InputState;
use automata_scene::{Events, Resources, Scene};
use tracing::{debug, info};

use crate::config::EngineConfig;
use crate::driver;
use crate::error::{Error, Result};
use crate::logging;
use crate::time::Time;
use crate::window::WindowMode;

/// Lifecycle state of an [`Application`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    /// The application has been created but its main loop has not run yet.
    Created,
    /// The main loop is currently running.
    Running,
    /// The main loop has exited and the application has shut down.
    Shutdown,
}

/// Engine state shared with the game each frame: the ECS scene, the event
/// queue, and the resource store.
///
/// Owned by [`Application`]; borrowed by [`AppContext`] during the update
/// callback. This is a single aggregate so the frame-context constructor
/// stays small.
#[derive(Default)]
pub struct WorldState {
    scene: Scene,
    events: Events,
    resources: Resources,
}

impl WorldState {
    /// The active scene.
    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The active scene, mutably.
    #[must_use]
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// The event queue.
    #[must_use]
    pub fn events(&self) -> &Events {
        &self.events
    }

    /// The event queue, mutably.
    #[must_use]
    pub fn events_mut(&mut self) -> &mut Events {
        &mut self.events
    }

    /// The shared resource store.
    #[must_use]
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// The shared resource store, mutably.
    #[must_use]
    pub fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }
}

/// Per-frame context handed to the update callback.
///
/// Exposes read-only timing and input information, the frame's
/// [`automata_renderer::DrawList`] for recording rendering commands, the
/// engine's [`WorldState`] (scene, events, resources), plus the frame
/// controls the game is allowed to touch, such as requesting shutdown.
pub struct AppContext<'a> {
    time: &'a Time,
    input: &'a InputState,
    draw: &'a mut automata_renderer::DrawList,
    renderer: Option<&'a automata_renderer::Renderer>,
    world: &'a mut WorldState,
    quit_requested: &'a mut bool,
}

impl<'a> AppContext<'a> {
    /// Creates a frame context. Called by the main loop implementations only.
    pub(crate) fn new(
        time: &'a Time,
        input: &'a InputState,
        draw: &'a mut automata_renderer::DrawList,
        renderer: Option<&'a automata_renderer::Renderer>,
        world: &'a mut WorldState,
        quit_requested: &'a mut bool,
    ) -> Self {
        Self {
            time,
            input,
            draw,
            renderer,
            world,
            quit_requested,
        }
    }

    /// Timing information for the current frame.
    #[must_use]
    pub fn time(&self) -> &Time {
        self.time
    }

    /// Input state for the current frame (keyboard, mouse).
    #[must_use]
    pub fn input(&self) -> &InputState {
        self.input
    }

    /// The frame's draw list: record sprites and meshes here.
    ///
    /// In headless mode the list is discarded each frame (no GPU exists to
    /// present it); recording is still allowed so game code runs identically.
    #[must_use]
    pub fn draw(&mut self) -> &mut automata_renderer::DrawList {
        self.draw
    }

    /// The renderer, when one exists.
    ///
    /// `Some` in windowed mode (where a GPU is present), `None` in headless
    /// mode. Use it to create textures, meshes, and materials.
    #[must_use]
    pub fn renderer(&self) -> Option<&automata_renderer::Renderer> {
        self.renderer
    }

    /// The active scene: spawn entities and edit components here.
    #[must_use]
    pub fn scene(&mut self) -> &mut Scene {
        &mut self.world.scene
    }

    /// The event queue: send and read frame-scoped events.
    #[must_use]
    pub fn events(&mut self) -> &mut Events {
        &mut self.world.events
    }

    /// The shared resource store.
    #[must_use]
    pub fn resources(&mut self) -> &mut Resources {
        &mut self.world.resources
    }

    /// Requests that the main loop exits after the current frame completes.
    pub fn request_quit(&mut self) {
        *self.quit_requested = true;
    }
}

/// The root of an AUTOMATA application.
///
/// `Application` owns the engine lifecycle: creation from an
/// [`EngineConfig`], exactly one run of the main loop, and a clean shutdown.
///
/// # Examples
///
/// ```
/// use automata_core::{AppState, Application, EngineConfig, WindowMode};
///
/// // Headless mode runs without a window or GPU — used here because test
/// // environments have no display.
/// let config = EngineConfig {
///     window_mode: WindowMode::Headless,
///     ..EngineConfig::new("demo")
/// };
/// let mut app = Application::new(config);
/// assert_eq!(app.state(), AppState::Created);
///
/// app.run(|ctx| ctx.request_quit()).unwrap();
///
/// assert_eq!(app.state(), AppState::Shutdown);
/// assert_eq!(app.time().frame_count(), 1);
/// ```
pub struct Application {
    config: EngineConfig,
    state: AppState,
    time: Time,
    quit_requested: bool,
    world: WorldState,
}

impl Application {
    /// Creates a new application and initializes engine-wide services
    /// (currently: logging).
    #[must_use]
    pub fn new(config: EngineConfig) -> Self {
        logging::init(&config.log_filter);
        info!(title = %config.title, version = %config.version, "automata initialized");
        Self {
            config,
            state: AppState::Created,
            time: Time::default(),
            quit_requested: false,
            world: WorldState::default(),
        }
    }

    /// The configuration this application was created with.
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The current lifecycle state.
    #[must_use]
    pub fn state(&self) -> AppState {
        self.state
    }

    /// Timing state accumulated by the main loop.
    #[must_use]
    pub fn time(&self) -> &Time {
        &self.time
    }

    /// The engine's world state (scene, events, resources).
    #[must_use]
    pub fn world(&self) -> &WorldState {
        &self.world
    }

    /// The engine's world state, mutably.
    #[must_use]
    pub fn world_mut(&mut self) -> &mut WorldState {
        &mut self.world
    }

    /// Requests shutdown from outside the update callback.
    ///
    /// In windowed mode the event loop owns the thread, so this is primarily
    /// an embedding hook (and the future home of OS signal handling).
    pub fn request_quit(&mut self) {
        self.quit_requested = true;
    }

    /// Runs the main loop until shutdown is requested or the window closes.
    ///
    /// `update` is invoked once per frame with an [`AppContext`]. How the
    /// loop is driven depends on [`EngineConfig::window_mode`]:
    ///
    /// - [`WindowMode::Windowed`] — the winit event loop drives frames,
    ///   delivering OS input events and presenting rendered frames.
    /// - [`WindowMode::Headless`] — a sleep-paced loop runs frames without a
    ///   window or GPU; input state stays empty.
    ///
    /// When a quit is requested — by the callback, by closing the window, or
    /// by an embedding host — the loop finishes the current frame, shuts the
    /// application down, and returns.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidState`] — the application has already run.
    /// - [`Error::EventLoop`], [`Error::Window`] — the OS event loop or
    ///   window could not be created (e.g. no display available).
    /// - [`Error::Renderer`] — GPU initialization or presentation failed.
    pub fn run(&mut self, update: impl FnMut(&mut AppContext<'_>)) -> Result<()> {
        if self.state != AppState::Created {
            return Err(Error::InvalidState {
                expected: format!("{:?}", AppState::Created),
                found: format!("{:?}", self.state),
            });
        }
        self.state = AppState::Running;
        info!(mode = ?self.config.window_mode, "entering main loop");

        let result = match self.config.window_mode.clone() {
            WindowMode::Headless => {
                self.run_headless(update);
                Ok(())
            }
            WindowMode::Windowed(window_config) => driver::run_windowed(
                &self.config,
                &window_config,
                &mut self.time,
                &mut self.world,
                &mut self.quit_requested,
                update,
            ),
        };

        self.state = AppState::Shutdown;
        info!(
            frames = self.time.frame_count(),
            elapsed_seconds = self.time.elapsed_seconds(),
            "automata shut down"
        );
        result
    }

    /// The headless main loop: frames as fast as [`EngineConfig::target_fps`]
    /// allows, without a window, GPU, or input events. Draw commands are
    /// recorded into a throwaway list so game code runs identically.
    fn run_headless(&mut self, mut update: impl FnMut(&mut AppContext<'_>)) {
        let mut input = InputState::new();
        let mut draw = automata_renderer::DrawList::new();
        let mut frame_start = Instant::now();

        while !self.quit_requested {
            let delta = frame_start.elapsed();
            frame_start = Instant::now();
            self.time.advance(delta);
            self.world.events.update();
            debug!(frame = self.time.frame_count(), "frame begin");

            update(&mut AppContext::new(
                &self.time,
                &input,
                &mut draw,
                None, // no renderer in headless mode
                &mut self.world,
                &mut self.quit_requested,
            ));
            self.world.scene.update();
            // Record the scene into the throwaway draw list: headless frames
            // exercise the same scene-to-renderer bridge as windowed ones.
            automata_scene::record_draw_list(self.world.scene.world(), &mut draw);
            input.end_frame();

            if let Some(fps) = self.config.target_fps {
                let budget = Duration::from_secs_f64(1.0 / f64::from(fps));
                if let Some(remaining) = budget.checked_sub(frame_start.elapsed()) {
                    thread::sleep(remaining);
                }
            }
        }
    }
}

// The closures passed to `Application::run` guide lifetime inference for
// `AppContext<'_>`; clippy's suggested method-path rewrite does not typecheck.
#[cfg(test)]
#[allow(clippy::redundant_closure_for_method_calls)]
mod tests {
    use super::*;
    use crate::window::WindowMode;

    /// A headless configuration without frame pacing so tests run at full
    /// speed without a display.
    fn test_config() -> EngineConfig {
        EngineConfig {
            window_mode: WindowMode::Headless,
            target_fps: None,
            ..EngineConfig::new("test")
        }
    }

    #[test]
    fn new_application_starts_in_created_state() {
        let app = Application::new(test_config());
        assert_eq!(app.state(), AppState::Created);
        assert_eq!(app.time().frame_count(), 0);
    }

    #[test]
    fn run_transitions_through_running_to_shutdown() {
        let mut app = Application::new(test_config());
        let mut observed_state = None;

        app.run(|ctx| {
            observed_state = Some(AppState::Running);
            ctx.request_quit();
        })
        .unwrap();

        assert_eq!(observed_state, Some(AppState::Running));
        assert_eq!(app.state(), AppState::Shutdown);
        assert_eq!(app.time().frame_count(), 1);
    }

    #[test]
    fn running_twice_is_an_invalid_state_error() {
        let mut app = Application::new(test_config());
        app.run(|ctx| ctx.request_quit()).unwrap();

        let result = app.run(|_| {});
        assert!(matches!(result, Err(Error::InvalidState { .. })));
        assert_eq!(app.state(), AppState::Shutdown);
    }

    #[test]
    fn quit_requested_before_run_still_processes_one_frame() {
        let mut app = Application::new(test_config());
        app.request_quit();
        // A quit requested before the loop starts is honored immediately:
        // the loop body never executes.
        app.run(|ctx| ctx.request_quit()).unwrap();
        assert_eq!(app.time().frame_count(), 0);
    }

    #[test]
    fn update_receives_an_input_state() {
        let mut app = Application::new(test_config());
        app.run(|ctx| {
            assert!(!ctx.input().pressed(automata_input::KeyCode::Space));
            assert_eq!(ctx.input().cursor_position(), None);
            ctx.request_quit();
        })
        .unwrap();
    }
}
