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
}

#[pymethods]
impl PyGame {
    /// Creates a game. With `headless=True` no window or GPU is created —
    /// the loop is sleep-paced; useful for tests, CI, and tooling.
    #[new]
    #[pyo3(signature = (title="AUTOMATA", resolution=(1280, 720), headless=false, resizable=true, vsync=true, log_filter=None, target_fps=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        title: &str,
        resolution: (u32, u32),
        headless: bool,
        resizable: bool,
        vsync: bool,
        log_filter: Option<&str>,
        target_fps: Option<u32>,
    ) -> PyResult<Self> {
        if target_fps == Some(0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "target_fps must be positive; omit it to disable frame pacing",
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
            ..EngineConfig::new(title)
        };
        if let Some(filter) = log_filter {
            filter.clone_into(&mut config.log_filter);
        }
        Ok(Self {
            app: Application::new(config),
            resources: Rc::new(RefCell::new(HashMap::new())),
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
    /// May be called before `run`, or from inside the update callback to
    /// switch scenes (applied immediately).
    fn load_scene(&mut self, scene: &PyScene) {
        load_staged(self.app.world_mut().scene_mut(), scene.entities());
    }

    /// Runs the main loop, calling `update(ctx)` once per frame.
    ///
    /// The loop ends when `ctx.quit()` is called, the window is closed, or
    /// `max_frames` is reached. Returns the total number of frames run.
    /// Engine failures raise `automata.Error`; exceptions raised by
    /// `update` stop the loop and propagate unchanged.
    #[pyo3(signature = (update, max_frames=None))]
    fn run(&mut self, update: &Bound<'_, PyAny>, max_frames: Option<u64>) -> PyResult<u64> {
        if self.app.state() == automata_core::AppState::Shutdown {
            return Err(AutomataError::new_err(
                "this game has already shut down; create a new Game to run again",
            ));
        }

        let quit: crate::context::QuitFlag = Rc::new(Cell::new(false));
        let writes: WriteQueue = Rc::new(RefCell::new(FrameWrites::default()));
        let outbox: EventOutbox = Rc::new(RefCell::new(Vec::new()));
        let resources = Rc::clone(&self.resources);
        let mut update_error: Option<PyErr> = None;

        let run_result = self.app.run(|ctx| {
            run_python_frame(
                ctx,
                update,
                &quit,
                &writes,
                &outbox,
                &resources,
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
}

/// Runs one frame's Python update and applies the buffered writes.
#[allow(clippy::too_many_arguments)]
fn run_python_frame(
    ctx: &mut AppContext<'_>,
    update: &Bound<'_, PyAny>,
    quit: &crate::context::QuitFlag,
    writes: &WriteQueue,
    outbox: &EventOutbox,
    resources: &ResourceStore,
    update_error: &mut Option<PyErr>,
) {
    let frame = FrameContext::new(
        FrameTime {
            delta: ctx.time().delta().as_secs_f64(),
            elapsed: ctx.time().elapsed_seconds(),
            frame: ctx.time().frame_count(),
        },
        PyInput::snapshot(ctx.input()),
        SceneView::snapshot(ctx.scene(), Rc::clone(writes)),
        PyEvents::snapshot(ctx.events(), Rc::clone(outbox)),
        PyResources::new(Rc::clone(resources)),
        Rc::clone(quit),
    );

    let call_result = update.call1((frame,));

    // Writes recorded by the callback are applied even when it raised, so
    // error-recovery paths see a consistent world.
    apply_writes(ctx.scene(), writes);
    PyEvents::flush(ctx.events(), outbox);

    if let Err(err) = call_result {
        tracing::error!(error = %err, "python update callback raised");
        *update_error = Some(err);
    }
}

impl std::fmt::Debug for PyGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Game")
            .field("state", &self.app.state())
            .finish_non_exhaustive()
    }
}
