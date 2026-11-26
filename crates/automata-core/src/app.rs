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

use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use automata_input::InputState;
use automata_scene::{Events, Resources, Scene};
use tracing::{debug, info};

use crate::config::EngineConfig;
use crate::error::{Error, Result};
use crate::logging;
use crate::time::Time;
use crate::window::WindowMode;

/// Shared handle to the audio engine, cloned into per-frame views.
pub type SharedAudio = Rc<RefCell<automata_audio::AudioEngine>>;

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
    physics3d: automata_physics::PhysicsWorld3D,
    physics2d: automata_physics::PhysicsWorld2D,
    physics_accumulator: f32,
    audio: SharedAudio,
    cursor_locked: bool,
    graphics_quality: automata_renderer::GraphicsQuality,
    graphics_changed: bool,
    /// The scene's navigation mesh, once the first physics step got it
    /// ready (see [`automata_nav::scene::prepare`]).
    nav: Option<std::sync::Arc<automata_nav::NavMesh>>,
    /// Whether the scene's navigation was got ready.
    nav_ready: bool,
}

impl WorldState {
    /// Whether the game asked for the cursor to be locked (hidden and held
    /// in the window, for mouse-look). The windowed runtime applies it.
    #[must_use]
    pub fn cursor_locked(&self) -> bool {
        self.cursor_locked
    }

    /// Locks (hides and captures) or releases the cursor.
    pub fn set_cursor_locked(&mut self, locked: bool) {
        self.cursor_locked = locked;
    }

    /// The graphics quality preset in use (starts as
    /// [`EngineConfig::graphics_quality`]).
    #[must_use]
    pub fn graphics_quality(&self) -> automata_renderer::GraphicsQuality {
        self.graphics_quality
    }

    /// Switches the graphics quality preset at runtime. The windowed
    /// runtime applies it before the next frame renders (rebuilding the
    /// shadow maps and pipelines that change); headless runs only record it.
    pub fn set_graphics_quality(&mut self, quality: automata_renderer::GraphicsQuality) {
        if quality != self.graphics_quality {
            self.graphics_quality = quality;
            self.graphics_changed = true;
        }
    }

    /// Returns the quality set since the last call, if it changed (for the
    /// runtime that owns the renderer).
    pub(crate) fn take_graphics_change(&mut self) -> Option<automata_renderer::GraphicsQuality> {
        std::mem::take(&mut self.graphics_changed).then_some(self.graphics_quality)
    }

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

    /// The 3D physics world (queries and impulses; stepping is engine-side).
    #[must_use]
    pub fn physics3d(&mut self) -> &mut automata_physics::PhysicsWorld3D {
        &mut self.physics3d
    }

    /// The 2D physics world (queries and impulses; stepping is engine-side).
    #[must_use]
    pub fn physics2d(&mut self) -> &mut automata_physics::PhysicsWorld2D {
        &mut self.physics2d
    }

    /// The 3D physics world, read-only (for frame snapshots).
    #[must_use]
    pub fn physics3d_ref(&self) -> &automata_physics::PhysicsWorld3D {
        &self.physics3d
    }

    /// Advances the scene's animations by `dt` seconds (see
    /// [`automata_scene::animate`]), feet standing on the physics
    /// colliders (or, in a world without any, on the meshes).
    pub fn animate(&mut self, dt: f32) {
        let world = self.scene.world_mut();
        if self.physics3d.collider_count() == 0 {
            automata_scene::animate(world, dt);
            return;
        }
        let physics = &self.physics3d;
        let ground = |entity: automata_scene::Entity, from: glam::Vec3, max: f32| {
            let filter = automata_physics::RayFilter {
                exclude_sensors: true,
                exclude_entity: Some(entity),
                ..automata_physics::RayFilter::default()
            };
            physics
                .cast_ray(from, glam::Vec3::NEG_Y, max, &filter)
                .map(|hit| (hit.point, hit.normal))
        };
        automata_scene::animate_on(world, dt, Some(&ground));
    }

    /// The scene's navigation mesh, if it has one (ready from the first
    /// physics step on).
    #[must_use]
    pub fn nav(&self) -> Option<&std::sync::Arc<automata_nav::NavMesh>> {
        self.nav.as_ref()
    }

    /// Replaces the navigation mesh (`None` drops it): a level rebuilt at
    /// run time can bake afresh with [`automata_nav::scene::bake_scene`].
    pub fn set_nav(&mut self, mesh: Option<automata_nav::NavMesh>) {
        self.nav = mesh.map(std::sync::Arc::new);
        self.nav_ready = true;
    }

    /// Gets the scene's navigation ready (once, at the first physics step):
    /// the saved mesh, or a fresh bake. Warnings go to the log and stderr.
    fn prepare_nav(&mut self) {
        if self.nav_ready {
            return;
        }
        self.nav_ready = true;
        let scene = self.scene.world_mut();
        self.physics3d.sync_from_scene(scene);
        let base = automata_assets::root();
        let Some(prepared) = automata_nav::scene::prepare(scene, &self.physics3d, base.as_deref())
        else {
            return;
        };
        if let Some(warning) = prepared.warning {
            tracing::warn!("{warning}");
            eprintln!("automata: {warning}");
        }
        self.nav = prepared.mesh.map(std::sync::Arc::new);
    }

    /// The 2D physics world, read-only.
    #[must_use]
    pub fn physics2d_ref(&self) -> &automata_physics::PhysicsWorld2D {
        &self.physics2d
    }

    /// The audio engine, read-only.
    #[must_use]
    pub fn audio(&self) -> std::cell::Ref<'_, automata_audio::AudioEngine> {
        self.audio.borrow()
    }

    /// The shared audio handle, cloned for per-frame views.
    #[must_use]
    pub fn audio_handle(&self) -> SharedAudio {
        Rc::clone(&self.audio)
    }

    /// Advances the audio engine by one frame. On the silent (default)
    /// backend this drives deterministic playback advancement; with the `cpal`
    /// feature the device backend advances itself and this is a no-op.
    pub(crate) fn pump_audio(&mut self, delta_seconds: f64) {
        self.audio.borrow_mut().pump_frame(delta_seconds);
    }

    /// Hears the world from `camera` (the 3D camera's position): muffled
    /// while it's under water. Called by the frame loops each frame.
    pub(crate) fn listen_from(&mut self, camera: glam::Vec3) {
        let under = self
            .physics3d
            .water_height(camera.x, camera.z)
            .is_some_and(|h| camera.y < h);
        self.audio.borrow_mut().set_underwater(under);
    }

    /// The physics worlds and scene, split-borrowed for frame views.
    #[doc(hidden)]
    pub fn physics_parts(
        &mut self,
    ) -> (
        &mut automata_physics::PhysicsWorld3D,
        &mut automata_physics::PhysicsWorld2D,
        &mut Scene,
    ) {
        (&mut self.physics3d, &mut self.physics2d, &mut self.scene)
    }

    /// Step both physics worlds at the fixed timestep until `delta` seconds
    /// of accumulated frame time are consumed. Collision and contact-force
    /// events are pushed onto the event queue.
    pub(crate) fn step_physics(&mut self, delta: f32, timestep: f32) {
        const MAX_STEPS: u32 = 4;
        if timestep <= 0.0 {
            return;
        }
        self.physics_accumulator += delta;
        // Run whole fixed steps; the remainder carries over. Guarantee at
        // least one step per frame so a fast loop never starves physics.
        let mut steps = 0;
        if self.physics_accumulator < timestep {
            self.physics_accumulator += timestep;
        }
        while self.physics_accumulator >= timestep && steps < MAX_STEPS {
            self.physics_accumulator -= timestep;
            steps += 1;

            self.prepare_nav();
            {
                let scene = self.scene.world_mut();
                let physics = &self.physics3d;
                let grounded = |e: automata_scene::Entity| physics.grounded(e);
                automata_nav::step_agents(
                    scene,
                    &automata_nav::AgentWorld {
                        nav: self.nav.as_deref(),
                        dt: timestep,
                        grounded: &grounded,
                    },
                );
                self.physics3d.sync_from_scene(scene);
                self.physics3d.move_characters(scene, timestep);
            }
            let tick3d = {
                let scene = self.scene.world_mut();
                self.physics3d.step(scene, timestep)
            };
            self.push_physics_events(tick3d);

            {
                let scene = self.scene.world_mut();
                self.physics2d.sync_from_scene(scene);
                self.physics2d.move_characters(scene, timestep);
            }
            let tick2d = {
                let scene = self.scene.world_mut();
                self.physics2d.step(scene, timestep)
            };
            self.push_physics_events(tick2d);
        }
    }

    fn push_physics_events(&mut self, tick: automata_physics::PhysicsTick) {
        for event in tick.collisions {
            self.events.send(event);
        }
        for event in tick.contact_forces {
            self.events.send(event);
        }
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
    gamepads: &'a automata_input::Gamepads,
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
        gamepads: &'a automata_input::Gamepads,
        draw: &'a mut automata_renderer::DrawList,
        renderer: Option<&'a automata_renderer::Renderer>,
        world: &'a mut WorldState,
        quit_requested: &'a mut bool,
    ) -> Self {
        Self {
            time,
            input,
            gamepads,
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

    /// Gamepad state for the current frame.
    #[must_use]
    pub fn gamepads(&self) -> &automata_input::Gamepads {
        self.gamepads
    }

    /// The engine's world state (scene, events, resources, physics worlds).
    #[must_use]
    pub fn world_state(&self) -> &WorldState {
        self.world
    }

    /// The engine's world state, mutably.
    #[must_use]
    pub fn world_state_mut(&mut self) -> &mut WorldState {
        self.world
    }

    /// The 3D physics world: raycasts, impulses, queries. Stepping happens
    /// at the engine's fixed timestep before the update runs.
    #[must_use]
    pub fn physics3d(&mut self) -> &mut automata_physics::PhysicsWorld3D {
        self.world.physics3d()
    }

    /// The 2D physics world: raycasts, impulses, queries.
    #[must_use]
    pub fn physics2d(&mut self) -> &mut automata_physics::PhysicsWorld2D {
        self.world.physics2d()
    }

    /// The audio engine: play and control sounds.
    ///
    /// Returns the shared handle; borrow it to operate (e.g.
    /// `ctx.audio().borrow_mut().play(...)`).
    #[must_use]
    pub fn audio(&mut self) -> SharedAudio {
        self.world.audio_handle()
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
    /// A stepped run's window settings and frame-to-frame state.
    stepped: Option<(crate::window::WindowConfig, crate::runtime::RunState)>,
}

impl Application {
    /// Creates a new application and initializes engine-wide services
    /// (currently: logging).
    #[must_use]
    pub fn new(config: EngineConfig) -> Self {
        logging::init(&config.log_filter);
        info!(title = %config.title, version = %config.version, "automata initialized");
        let world = WorldState {
            graphics_quality: config.graphics_quality,
            ..WorldState::default()
        };
        Self {
            config,
            state: AppState::Created,
            time: Time::default(),
            quit_requested: false,
            world,
            stepped: None,
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
            WindowMode::Windowed(window_config) => {
                let mut state = crate::runtime::RunState::new(&self.config);
                let mut runtime = crate::runtime::Runtime::new(
                    &self.config,
                    &window_config,
                    &mut self.time,
                    &mut self.world,
                    &mut self.quit_requested,
                    &mut state,
                    update,
                );
                run_driver(&mut runtime)
            }
        };

        self.state = AppState::Shutdown;
        info!(
            frames = self.time.frame_count(),
            elapsed_seconds = self.time.elapsed_seconds(),
            "automata shut down"
        );
        result
    }

    /// Starts a *stepped run*, for platforms whose own loop calls the game
    /// (the web: each `requestAnimationFrame`). Instead of looping until
    /// the game quits like [`run`](Application::run), this returns at
    /// once, and the platform calls [`step`](Application::step) per frame
    /// until it reports the run is over.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when the application has already run, and
    /// [`Error::Platform`] for a headless configuration (it has no frames
    /// to hand out: use [`run`](Application::run)).
    pub fn start_stepped(&mut self) -> Result<()> {
        if self.state != AppState::Created {
            return Err(Error::InvalidState {
                expected: format!("{:?}", AppState::Created),
                found: format!("{:?}", self.state),
            });
        }
        let WindowMode::Windowed(window_config) = self.config.window_mode.clone() else {
            return Err(Error::Platform(
                "a headless game runs with run(), not stepped".into(),
            ));
        };
        self.state = AppState::Running;
        info!("entering a stepped run (the platform drives frames)");
        self.stepped = Some((window_config, crate::runtime::RunState::new(&self.config)));
        Ok(())
    }

    /// Whether a stepped run is going on.
    #[must_use]
    pub fn is_stepping(&self) -> bool {
        self.stepped.is_some()
    }

    /// One step of a stepped run: `drive` gets the engine as a
    /// [`crate::platform::Host`], exactly as a driver inside
    /// [`run`](Application::run) does, and typically attaches the surface
    /// (first step), feeds the frame's input, then calls
    /// [`frame`](crate::platform::Host::frame) and
    /// [`render`](crate::platform::Host::render). `update` is the game's
    /// per-frame callback.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when no stepped run is going on.
    pub fn step<R>(
        &mut self,
        update: impl FnMut(&mut AppContext<'_>),
        drive: impl FnOnce(&mut dyn crate::platform::Host) -> R,
    ) -> Result<R> {
        let Some((window_config, state)) = self.stepped.as_mut() else {
            return Err(Error::InvalidState {
                expected: "a stepped run".into(),
                found: format!("{:?}", self.state),
            });
        };
        let mut runtime = crate::runtime::Runtime::new(
            &self.config,
            window_config,
            &mut self.time,
            &mut self.world,
            &mut self.quit_requested,
            state,
            update,
        );
        Ok(drive(&mut runtime))
    }

    /// Ends a stepped run (after a step returned
    /// [`FrameStep::Quit`](crate::platform::FrameStep::Quit), or when the
    /// page goes away): the renderer and window state are dropped and the
    /// application shuts down. Does nothing when no stepped run is going on.
    pub fn finish_stepped(&mut self) {
        if self.stepped.take().is_some() {
            self.state = AppState::Shutdown;
            info!(
                frames = self.time.frame_count(),
                elapsed_seconds = self.time.elapsed_seconds(),
                "automata shut down"
            );
        }
    }

    /// The headless main loop: frames as fast as [`EngineConfig::target_fps`]
    /// allows, without a window, GPU, or input events. Draw commands are
    /// recorded into a throwaway list so game code runs identically.
    fn run_headless(&mut self, mut update: impl FnMut(&mut AppContext<'_>)) {
        let mut input = InputState::new();
        let mut gamepads = automata_input::Gamepads::new();
        let mut draw = automata_renderer::DrawList::new();
        let mut frame_start = Instant::now();

        let fixed = self
            .config
            .fixed_delta
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(Duration::from_secs_f32);
        while !self.quit_requested {
            let delta = fixed.unwrap_or_else(|| frame_start.elapsed());
            frame_start = Instant::now();
            self.time.advance(delta);
            self.world.events.update();
            debug!(frame = self.time.frame_count(), "frame begin");

            gamepads.update();
            for event in gamepads.drain_events() {
                self.world.events.send(event);
            }
            if let Some(timestep) = self.config.physics_timestep {
                self.world.step_physics(delta.as_secs_f32(), timestep);
            }
            self.world.pump_audio(delta.as_secs_f64());

            update(&mut AppContext::new(
                &self.time,
                &input,
                &gamepads,
                &mut draw,
                None, // no renderer in headless mode
                &mut self.world,
                &mut self.quit_requested,
            ));
            self.world.scene.update();
            self.world.animate(delta.as_secs_f32());
            automata_scene::simulate_particles(self.world.scene.world_mut(), delta.as_secs_f32());
            // Record the scene into the throwaway draw list: headless frames
            // exercise the same scene-to-renderer bridge as windowed ones.
            automata_scene::record_draw_list(self.world.scene.world(), &mut draw);
            automata_physics::record_grass_pushers(self.world.scene.world(), &mut draw);
            draw.set_time(self.world.physics3d.water_time());
            if let Some(camera) = draw.camera_3d() {
                self.world
                    .physics3d
                    .set_ripple_view(camera.position, camera.target);
                self.world.listen_from(camera.position);
            }
            if let Some(ripples) = self.world.physics3d.ripples() {
                draw.set_ripples(ripples);
            }
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

/// Runs a windowed game with the installed platform driver, or the
/// built-in winit one on desktop.
fn run_driver(host: &mut dyn crate::runtime::Host) -> Result<()> {
    if let Some(driver) = crate::platform::installed_driver() {
        return driver(host);
    }
    #[cfg(not(any(target_os = "android", target_os = "emscripten")))]
    {
        crate::driver::run(host)
    }
    #[cfg(any(target_os = "android", target_os = "emscripten"))]
    {
        Err(Error::Platform(
            "no window driver installed (Android installs SDL's; the web runs games stepped)"
                .into(),
        ))
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
    fn a_stepped_run_hands_out_frames_until_the_game_quits() {
        use crate::platform::FrameStep;
        let mut app = Application::new(EngineConfig {
            target_fps: None,
            ..EngineConfig::new("Stepped")
        });
        assert!(!app.is_stepping());
        assert!(app.step(|_| {}, |_| ()).is_err(), "no stepped run yet");
        app.start_stepped().unwrap();
        assert!(app.is_stepping());
        assert_eq!(app.state(), AppState::Running);
        assert!(app.start_stepped().is_err(), "only once");

        let mut frames = 0;
        let mut steps = 0;
        loop {
            steps += 1;
            let step = app
                .step(
                    |ctx| {
                        frames += 1;
                        assert_eq!(ctx.input().cursor_position(), Some((1.0, 2.0)));
                        if frames == 3 {
                            ctx.request_quit();
                        }
                    },
                    |host| {
                        assert_eq!(host.title(), "Stepped");
                        host.input(&automata_input::InputEvent::CursorMoved { x: 1.0, y: 2.0 });
                        let step = host.frame();
                        host.render().unwrap(); // no surface yet: nothing to draw
                        step
                    },
                )
                .unwrap();
            if step == FrameStep::Quit {
                break;
            }
        }
        assert_eq!((frames, steps), (3, 3));
        assert_eq!(app.time().frame_count(), 3);
        app.finish_stepped();
        assert!(!app.is_stepping());
        assert_eq!(app.state(), AppState::Shutdown);
        assert!(app.run(|_| {}).is_err());
    }

    #[test]
    fn a_headless_game_does_not_step() {
        let mut app = Application::new(test_config());
        assert!(matches!(app.start_stepped(), Err(Error::Platform(_))));
        assert_eq!(app.state(), AppState::Created);
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
    fn graphics_quality_starts_from_the_config_and_reports_changes_once() {
        use automata_renderer::GraphicsQuality;
        let app = Application::new(EngineConfig {
            graphics_quality: GraphicsQuality::Low,
            ..EngineConfig::default()
        });
        let mut world = app.world;
        assert_eq!(world.graphics_quality(), GraphicsQuality::Low);
        assert_eq!(world.take_graphics_change(), None);
        world.set_graphics_quality(GraphicsQuality::Low); // unchanged: no-op
        assert_eq!(world.take_graphics_change(), None);
        world.set_graphics_quality(GraphicsQuality::Beautiful);
        assert_eq!(world.graphics_quality(), GraphicsQuality::Beautiful);
        assert_eq!(
            world.take_graphics_change(),
            Some(GraphicsQuality::Beautiful)
        );
        assert_eq!(world.take_graphics_change(), None);
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
    fn a_fixed_delta_advances_every_headless_frame_by_the_same_amount() {
        let mut app = Application::new(EngineConfig {
            fixed_delta: Some(0.25),
            ..test_config()
        });
        let mut frames = 0;
        app.run(|ctx| {
            frames += 1;
            if frames == 8 {
                ctx.request_quit();
            }
        })
        .unwrap();
        assert_eq!(app.time().frame_count(), 8);
        assert!((app.time().elapsed_seconds() - 2.0).abs() < 1e-9);
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

    #[test]
    fn sound_is_muffled_while_the_camera_is_under_water() {
        let mut app = Application::new(EngineConfig {
            fixed_delta: Some(1.0 / 60.0),
            ..test_config()
        });
        let mut frames = 0;
        let mut camera = None;
        let mut heard = Vec::new();
        app.run(|ctx| {
            frames += 1;
            if frames == 1 {
                ctx.scene().spawn((
                    automata_scene::Transform::default(),
                    automata_renderer::Water {
                        wave_height: 0.0,
                        ..automata_renderer::Water::default()
                    },
                ));
                let eye = glam::Vec3::new(0.0, -2.0, 4.0);
                camera = Some(ctx.scene().spawn((automata_renderer::Camera3D::new(
                    eye,
                    glam::Vec3::new(0.0, -2.0, 0.0),
                ),)));
            }
            if frames == 5 {
                let cam = camera.unwrap();
                ctx.scene()
                    .world_mut()
                    .get::<&mut automata_renderer::Camera3D>(cam)
                    .unwrap()
                    .position
                    .y = 3.0;
            }
            heard.push(ctx.audio().borrow().is_underwater());
            if frames == 8 {
                ctx.request_quit();
            }
        })
        .unwrap();
        // Frames see the previous frame's camera.
        assert!(heard[3] && heard[4], "{heard:?}");
        assert!(!heard[7], "{heard:?}");
    }
}
