//! The `Game` class: engine lifecycle and the main loop driven by a Python
//! update callback.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use automata_core::{AppContext, Application, EngineConfig, WindowConfig, WindowMode};
use pyo3::prelude::*;

use crate::context::{FrameContext, FrameTime, PyInput, PyResources, ResourceStore};
use crate::error::{core_error, AutomataError};
use crate::events::{EventOutbox, PyEvents};
use crate::scene::{apply_writes, load_staged, FrameWrites, PyScene, SceneView, WriteQueue};

/// The application: owns the engine, the loaded scene, and the main loop.
///
/// ```python
/// game = at.Game(title="Demo", resolution=(1280, 720))
/// game.load_scene(scene)
/// game.run(update)   # update(ctx) runs once per frame
/// ```
///
/// A game runs once: after `run` returns, the engine is shut down and a
/// second `run` raises `automata.Error`.
#[pyclass(name = "Game", unsendable, module = "automata._core", subclass)]
pub struct PyGame {
    app: Application,
    resources: ResourceStore,
    /// Plays the scene's `AudioSource` components.
    sources: automata_core::AudioSourcePlayer,
    /// Images drawn by `ctx.screen`.
    hud: crate::screen::HudTextures,
}

#[pymethods]
impl PyGame {
    /// Creates a game. With `headless=True` no window or GPU is created —
    /// the loop is sleep-paced; useful for tests, CI, and tooling.
    ///
    /// `clear_color=(r, g, b, a)` sets the color the window is cleared to
    /// each frame (components 0.0..=1.0); the default is a very dark blue.
    /// `graphics` is a quality preset: `"low"` (no shadows), `"medium"`,
    /// `"high"` (default), `"ultra"`, or `"beautiful"`. `fixed_delta` (headless only)
    /// advances every frame by exactly that many seconds instead of the
    /// measured time: deterministic tests, and benchmarks where a slow frame
    /// mustn't trigger catch-up physics steps.
    #[new]
    #[pyo3(signature = (title="AUTOMATA", resolution=(1280, 720), headless=false, resizable=true, vsync=true, log_filter=None, target_fps=None, clear_color=None, graphics=None, fixed_delta=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        title: &str,
        resolution: (u32, u32),
        headless: bool,
        resizable: bool,
        vsync: bool,
        log_filter: Option<&str>,
        target_fps: Option<u32>,
        clear_color: Option<(f64, f64, f64, f64)>,
        graphics: Option<&str>,
        fixed_delta: Option<f32>,
    ) -> PyResult<Self> {
        if target_fps == Some(0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "target_fps must be positive; omit it to disable frame pacing",
            ));
        }
        if fixed_delta.is_some_and(|d| !(d.is_finite() && d > 0.0)) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "fixed_delta must be a positive number of seconds",
            ));
        }
        let mut config = EngineConfig {
            window_mode: if headless {
                WindowMode::Headless
            } else {
                WindowMode::Windowed(WindowConfig {
                    size: resolution,
                    resizable,
                    vsync,
                })
            },
            target_fps,
            fixed_delta,
            ..EngineConfig::new(title)
        };
        if let Some(filter) = log_filter {
            filter.clone_into(&mut config.log_filter);
        }
        if let Some((r, g, b, a)) = clear_color {
            for component in [r, g, b, a] {
                if !(0.0..=1.0).contains(&component) {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "clear_color components must be in 0.0..=1.0",
                    ));
                }
            }
            config.clear_color = automata_renderer::Color { r, g, b, a };
        }
        if let Some(quality) = graphics {
            config.graphics_quality = parse_quality(quality)?;
        }
        Ok(Self {
            app: Application::new(config),
            resources: Rc::new(RefCell::new(HashMap::new())),
            sources: automata_core::AudioSourcePlayer::new(),
            hud: crate::screen::HudTextures::default(),
        })
    }

    /// The engine's lifecycle state: `"created"`, `"running"`, or
    /// `"shutdown"`.
    #[getter]
    fn state(&self) -> &'static str {
        match self.app.state() {
            automata_core::AppState::Created => "created",
            automata_core::AppState::Running => "running",
            automata_core::AppState::Shutdown => "shutdown",
        }
    }

    /// Loads a staged scene into the engine, replacing the current one.
    ///
    /// Call it before `run`. While the game is running, the Python
    /// `Game.load_scene` switches scenes through `ctx.scene.load` instead
    /// (applied when the frame's callback returns).
    fn load_scene(&mut self, scene: &PyScene) {
        load_staged(self.app.world_mut().scene_mut(), scene.staged_entities());
    }

    /// Runs the main loop, calling `update(ctx)` once per frame.
    ///
    /// The loop ends when `ctx.quit()` is called, the window is closed, or
    /// `max_frames` is reached. Returns the total number of frames run.
    /// Engine failures raise `automata.Error`; exceptions raised by
    /// `update` stop the loop and propagate unchanged.
    #[pyo3(signature = (update, max_frames=None))]
    fn run(
        &mut self,
        py: Python<'_>,
        update: &Bound<'_, PyAny>,
        max_frames: Option<u64>,
    ) -> PyResult<u64> {
        if self.app.state() == automata_core::AppState::Shutdown {
            return Err(AutomataError::new_err(
                "this game has already shut down; create a new Game to run again",
            ));
        }

        let quit: crate::context::QuitFlag = Rc::new(Cell::new(false));
        let writes: WriteQueue = Rc::new(RefCell::new(FrameWrites::default()));
        let physics_queue: crate::physics::PhysicsQueue =
            Rc::new(RefCell::new(crate::physics::PhysicsFrame::default()));
        let outbox: EventOutbox = Rc::new(RefCell::new(Vec::new()));
        let resources = Rc::clone(&self.resources);
        let mut update_error: Option<PyErr> = None;
        crate::components::default_asset_root(py);
        let screen: crate::screen::ScreenQueue = Rc::default();
        let audio_commands: crate::audio::AudioCommands = Rc::default();
        let Self {
            app, sources, hud, ..
        } = self;
        let mut extras = FrameExtras {
            sources,
            hud,
            screen: &screen,
            audio_commands: &audio_commands,
        };

        let run_result = app.run(|ctx| {
            run_python_frame(
                ctx,
                update,
                &quit,
                &writes,
                &physics_queue,
                &outbox,
                &resources,
                &mut extras,
                &mut update_error,
            );
            let hit_limit = max_frames.is_some_and(|limit| ctx.time().frame_count() >= limit);
            if quit.get() || update_error.is_some() || hit_limit {
                ctx.request_quit();
            }
        });

        run_result.map_err(|err| core_error(&err))?;
        if let Some(err) = update_error {
            return Err(err);
        }
        Ok(self.app.time().frame_count())
    }

    /// Requests shutdown from outside the update callback (for example from
    /// a signal handler thread that marshals back onto this one).
    fn quit(&mut self) {
        self.app.request_quit();
    }

    /// Serializes the live world — the scene as it currently stands, not as
    /// it was staged — to RON text.
    ///
    /// For Oberon this is the save path: physics has run, transforms have
    /// moved, and the file captures the world as it is now.
    fn scene_to_ron(&self) -> PyResult<String> {
        let file =
            automata_core::scene_file::SceneFile::from_world(self.app.world().scene().world());
        file.to_ron()
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))
    }

    /// Saves the live world to `path` as a RON scene file.
    fn save_scene(&self, path: &str) -> PyResult<()> {
        let text = self.scene_to_ron()?;
        std::fs::write(path, text)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(err.to_string()))
    }
}

/// Per-run state beyond the basic queues: audio sources and the screen.
struct FrameExtras<'a> {
    sources: &'a mut automata_core::AudioSourcePlayer,
    hud: &'a mut crate::screen::HudTextures,
    screen: &'a crate::screen::ScreenQueue,
    audio_commands: &'a crate::audio::AudioCommands,
}

/// Runs one frame's Python update and applies the buffered writes.
#[allow(clippy::too_many_arguments)]
fn run_python_frame(
    ctx: &mut AppContext<'_>,
    update: &Bound<'_, PyAny>,
    quit: &crate::context::QuitFlag,
    writes: &WriteQueue,
    physics_queue: &crate::physics::PhysicsQueue,
    outbox: &EventOutbox,
    resources: &ResourceStore,
    extras: &mut FrameExtras<'_>,
    update_error: &mut Option<PyErr>,
) {
    let (physics, physics2d) = {
        let world = ctx.world_state();
        let scene_world = world.scene().world();
        (
            crate::physics::PyPhysics::snapshot(
                world.physics3d_ref(),
                scene_world,
                Rc::clone(physics_queue),
            ),
            crate::physics::PyPhysics2D::snapshot(
                world.physics2d_ref(),
                scene_world,
                Rc::clone(physics_queue),
            ),
        )
    };
    let gamepad_events = crate::gamepads::PyGamepadEvent::convert_all(
        ctx.events().read::<automata_input::GamepadEvent>().cloned(),
    );
    let playing: std::collections::HashSet<u64> = ctx
        .scene()
        .world()
        .query::<(automata_scene::Entity, &automata_core::AudioSource)>()
        .iter()
        .map(|(e, _)| e)
        .filter(|e| extras.sources.is_playing(*e))
        .map(|e| e.to_bits().get())
        .collect();
    let audio =
        crate::audio::PyAudio::for_frame(ctx.audio(), Rc::clone(extras.audio_commands), playing);
    let root = automata_assets::root();
    // Headless runs have no surface; report the default window size.
    let size = ctx
        .renderer()
        .map_or((1280, 720), automata_renderer::Renderer::size);
    let screen = crate::screen::PyScreen::new(
        size,
        ctx.world_state().cursor_locked(),
        ctx.world_state().graphics_quality(),
        root.clone(),
        Rc::clone(extras.screen),
    );
    let frame = FrameContext::new(
        FrameTime {
            delta: ctx.time().delta().as_secs_f64(),
            elapsed: ctx.time().elapsed_seconds(),
            frame: ctx.time().frame_count(),
        },
        PyInput::snapshot(ctx.input(), ctx.gamepads(), gamepad_events),
        SceneView::snapshot(ctx.scene(), Rc::clone(writes)),
        PyEvents::snapshot(ctx.events(), Rc::clone(outbox)),
        PyResources::new(Rc::clone(resources)),
        Rc::clone(quit),
        physics,
        physics2d,
        audio,
        screen,
    );

    let call_result = update.call1((frame,));

    // Writes recorded by the callback are applied even when it raised, so
    // error-recovery paths see a consistent world.
    apply_writes(ctx.scene(), writes);
    {
        let (w3, w2, _) = ctx.world_state_mut().physics_parts();
        crate::physics::apply_physics_ops(w3, w2, physics_queue);
    }
    PyEvents::flush(ctx.events(), outbox);

    // Audio sources: play-on-start, script requests, distance fading.
    let commands = std::mem::take(&mut *extras.audio_commands.borrow_mut());
    {
        let audio = ctx.audio();
        let mut audio = audio.borrow_mut();
        for message in
            extras
                .sources
                .update(ctx.scene().world(), &mut audio, root.as_deref(), commands)
        {
            tracing::warn!("{message}");
            eprintln!("automata: {message}");
        }
    }
    // The screen: cursor lock and this frame's HUD.
    let frame = std::mem::take(&mut *extras.screen.borrow_mut());
    if let Some(locked) = frame.lock {
        ctx.world_state_mut().set_cursor_locked(locked);
    }
    if let Some(quality) = frame.graphics {
        ctx.world_state_mut().set_graphics_quality(quality);
    }
    if !frame.items.is_empty() {
        let sprites = ctx
            .renderer()
            .map(|r| extras.hud.sprites(r, &frame.items))
            .unwrap_or_default();
        for sprite in sprites {
            ctx.draw().sprite(sprite);
        }
    }

    if let Err(err) = call_result {
        tracing::error!(error = %err, "python update callback raised");
        *update_error = Some(err);
    }
}

/// Parses a graphics quality name (`"low"` … `"beautiful"`).
pub(crate) fn parse_quality(name: &str) -> PyResult<automata_renderer::GraphicsQuality> {
    automata_renderer::GraphicsQuality::ALL
        .into_iter()
        .find(|q| quality_name(*q) == name)
        .ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "unknown graphics quality {name:?}; expected \"low\", \
                 \"medium\", \"high\", \"ultra\", or \"beautiful\""
            ))
        })
}

/// The Python name of a graphics quality (`"high"`).
pub(crate) fn quality_name(quality: automata_renderer::GraphicsQuality) -> &'static str {
    use automata_renderer::GraphicsQuality as Q;
    match quality {
        Q::Low => "low",
        Q::Medium => "medium",
        Q::High => "high",
        Q::Ultra => "ultra",
        Q::Beautiful => "beautiful",
    }
}

impl std::fmt::Debug for PyGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Game")
            .field("state", &self.app.state())
            .finish_non_exhaustive()
    }
}
