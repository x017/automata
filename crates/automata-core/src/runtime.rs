//! The windowed frame cycle shared by every platform driver.
//!
//! [`Runtime`] reaches what a frame needs (time, world, the game's update,
//! and the [`RunState`] kept between frames: input, renderer, draw list)
//! and runs one frame at a time. A driver (winit on desktop, SDL on
//! Android) owns the window and the OS event loop: it feeds input, hands
//! over window surfaces, and calls [`Host::frame`] and [`Host::render`]
//! when it's time. It reaches the runtime through the public [`Host`]
//! trait, so drivers can live outside this crate.
//!
//! On the web the browser owns the loop instead: a *stepped run* keeps the
//! [`RunState`] in the application, and each animation frame builds a
//! runtime for one step (see [`crate::Application::start_stepped`]).

use std::time::{Duration, Instant};

use automata_input::{InputEvent, InputState};
use automata_renderer::{DrawList, GraphicsQuality, Renderer, SurfaceTarget};
use tracing::{debug, info};

use crate::app::{AppContext, WorldState};
use crate::config::EngineConfig;
use crate::error::Result;
use crate::time::Time;
use crate::window::WindowConfig;

/// What a driver should do after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStep {
    /// The game asked to quit: close the window and return.
    Quit,
    /// Run the next frame at this time (`None`: as soon as possible).
    Next(Option<Instant>),
}

/// The engine as a platform driver sees it: what to put in the window,
/// where input goes, and when to run and draw a frame.
///
/// A driver runs inside [`crate::Application::run`] (see
/// [`crate::platform::install_driver`]). Its loop is:
///
/// 1. Make the window, then [`attach_surface`](Host::attach_surface).
/// 2. Feed events with [`input`](Host::input); keep the size in sync with
///    [`resize`](Host::resize).
/// 3. When the [`FrameStep`] deadline passes, call [`frame`](Host::frame)
///    then [`render`](Host::render). Stop on [`FrameStep::Quit`] or when
///    the user closes the window.
/// 4. If the OS takes the window away (Android, in the background),
///    [`detach_surface`](Host::detach_surface), and attach the new one
///    when it comes back.
pub trait Host {
    /// The window title.
    fn title(&self) -> &str;
    /// The window size, resizability and vsync the game asked for.
    fn window_config(&self) -> &WindowConfig;
    /// Gives the renderer a window surface: the first time this makes the
    /// GPU context; later (after [`detach_surface`](Host::detach_surface))
    /// it renders to the new surface with every GPU resource kept.
    ///
    /// # Errors
    ///
    /// When there's no usable GPU or the surface can't be made.
    fn attach_surface(&mut self, target: SurfaceTarget<'static>, size: (u32, u32)) -> Result<()>;
    /// Lets go of the window surface; frames keep running but aren't drawn.
    fn detach_surface(&mut self);
    /// The window's size changed (physical pixels).
    fn resize(&mut self, width: u32, height: u32);
    /// Feeds one input event.
    fn input(&mut self, event: &InputEvent);
    /// Runs one game frame: time, physics, the update, scene systems.
    fn frame(&mut self) -> FrameStep;
    /// Draws the last frame, if there's a surface.
    ///
    /// # Errors
    ///
    /// When presenting fails for good (lost device).
    fn render(&mut self) -> Result<()>;
    /// Whether the game wants the cursor hidden and captured.
    fn cursor_locked(&self) -> bool;
}

/// What a windowed run keeps from one frame to the next.
pub(crate) struct RunState {
    pub(crate) input: InputState,
    pub(crate) gamepads: automata_input::Gamepads,
    pub(crate) renderer: Option<Renderer>,
    pub(crate) draw: DrawList,
    last_frame_start: Option<Instant>,
}

impl RunState {
    pub(crate) fn new(config: &EngineConfig) -> Self {
        let mut draw = DrawList::new();
        draw.set_clear_color(config.clear_color);
        Self {
            input: InputState::new(),
            gamepads: automata_input::Gamepads::new(),
            renderer: None,
            draw,
            last_frame_start: None,
        }
    }
}

/// The frame cycle, borrowed from [`crate::Application`] for one run (or,
/// in a stepped run, for one step).
pub(crate) struct Runtime<'a, F: FnMut(&mut AppContext<'_>)> {
    pub(crate) config: &'a EngineConfig,
    pub(crate) window_config: &'a WindowConfig,
    pub(crate) time: &'a mut Time,
    pub(crate) world: &'a mut WorldState,
    pub(crate) quit_requested: &'a mut bool,
    pub(crate) state: &'a mut RunState,
    update: F,
}

impl<'a, F: FnMut(&mut AppContext<'_>)> Runtime<'a, F> {
    pub(crate) fn new(
        config: &'a EngineConfig,
        window_config: &'a WindowConfig,
        time: &'a mut Time,
        world: &'a mut WorldState,
        quit_requested: &'a mut bool,
        state: &'a mut RunState,
        update: F,
    ) -> Self {
        Self {
            config,
            window_config,
            time,
            world,
            quit_requested,
            state,
            update,
        }
    }

    /// The quality preset for a new renderer.
    fn graphics_quality(&self) -> GraphicsQuality {
        self.world.graphics_quality()
    }

    /// Runs one frame: advance time, age events, update, propagate scene
    /// state and record the draw list.
    pub(crate) fn run_frame(&mut self) -> FrameStep {
        let now = Instant::now();
        let delta = self
            .state
            .last_frame_start
            .map_or(Duration::ZERO, |start| now.saturating_duration_since(start));
        self.state.last_frame_start = Some(now);
        self.time.advance(delta);
        self.world.events_mut().update();
        debug!(frame = self.time.frame_count(), "frame begin");

        self.state.gamepads.update();
        for event in self.state.gamepads.drain_events() {
            self.world.events_mut().send(event);
        }
        if let Some(timestep) = self.config.physics_timestep {
            self.world.step_physics(delta.as_secs_f32(), timestep);
        }
        self.world.pump_audio(delta.as_secs_f64());

        let renderer_ref = self.state.renderer.as_ref();
        (self.update)(&mut AppContext::new(
            self.time,
            &self.state.input,
            &self.state.gamepads,
            &mut self.state.draw,
            renderer_ref,
            self.world,
            self.quit_requested,
        ));
        self.state.input.end_frame();
        if let Some(quality) = self.world.take_graphics_change() {
            if let Some(renderer) = &mut self.state.renderer {
                info!(?quality, "graphics quality changed");
                renderer.set_quality(quality);
            }
        }
        self.run_scene_systems(delta);

        if *self.quit_requested {
            return FrameStep::Quit;
        }
        FrameStep::Next(
            self.config
                .target_fps
                .map(|fps| now + Duration::from_secs_f64(1.0 / f64::from(fps))),
        )
    }

    /// Built-in scene systems (transform propagation) and the
    /// scene-to-renderer bridge, after the game update. Plain-data
    /// `MeshVisual` components become GPU-backed `MeshRenderer`s here, the
    /// first frame a renderer is available.
    fn run_scene_systems(&mut self, delta: Duration) {
        self.world.scene_mut().update();
        if let Some(renderer) = &self.state.renderer {
            automata_scene::materialize_visuals(self.world.scene_mut().world_mut(), renderer);
            automata_scene::load_particle_textures(self.world.scene_mut().world_mut(), renderer);
        }
        automata_scene::animate(self.world.scene_mut().world_mut(), delta.as_secs_f32());
        automata_scene::simulate_particles(self.world.scene_mut().world_mut(), delta.as_secs_f32());
        automata_scene::record_draw_list(self.world.scene().world(), &mut self.state.draw);
        automata_physics::record_grass_pushers(self.world.scene().world(), &mut self.state.draw);
        // Waves run on the physics clock, so floating bodies ride exactly
        // the waves that are drawn.
        self.state
            .draw
            .set_time(self.world.physics3d_ref().water_time());
        // Ripples are simulated around what the camera sees.
        let physics = self.world.physics3d();
        if let Some(camera) = self.state.draw.camera_3d() {
            physics.set_ripple_view(camera.position, camera.target);
        }
        if let Some(ripples) = physics.ripples() {
            self.state.draw.set_ripples(ripples);
        }
        // Sound is muffled while the camera is under water.
        if let Some(camera) = self.state.draw.camera_3d() {
            self.world.listen_from(camera.position);
        }
    }
}

impl<F: FnMut(&mut AppContext<'_>)> Host for Runtime<'_, F> {
    fn title(&self) -> &str {
        &self.config.title
    }

    fn window_config(&self) -> &WindowConfig {
        self.window_config
    }

    fn attach_surface(&mut self, target: SurfaceTarget<'static>, size: (u32, u32)) -> Result<()> {
        if let Some(renderer) = &mut self.state.renderer {
            renderer.resume(target, size)?;
            // Back from the background: the time away isn't a frame.
            self.state.last_frame_start = None;
        } else {
            let renderer = pollster::block_on(Renderer::new(
                target,
                size,
                self.window_config.vsync,
                self.graphics_quality(),
            ))?;
            info!(
                width = size.0,
                height = size.1,
                "window and GPU context ready"
            );
            self.state.renderer = Some(renderer);
        }
        Ok(())
    }

    fn detach_surface(&mut self) {
        if let Some(renderer) = &mut self.state.renderer {
            renderer.suspend();
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if let Some(renderer) = &mut self.state.renderer {
            renderer.resize(width, height);
        }
    }

    fn input(&mut self, event: &InputEvent) {
        self.state.input.handle_event(event);
    }

    fn frame(&mut self) -> FrameStep {
        self.run_frame()
    }

    fn render(&mut self) -> Result<()> {
        if let Some(renderer) = &mut self.state.renderer {
            renderer.render(&self.state.draw)?;
            self.state.draw.clear();
        }
        Ok(())
    }

    fn cursor_locked(&self) -> bool {
        self.world.cursor_locked()
    }
}
