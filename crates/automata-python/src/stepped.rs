//! Stepped runs: platforms whose own loop calls the game.
//!
//! In a browser the page can't block in a game loop: the browser calls
//! back once per animation frame. The web build installs a *step driver*
//! (see [`install_step_driver`]); `Game.run` then starts the game and
//! returns, and the page calls [`step_running_game`] (`automata._core.
//! step_running_game()`) from each `requestAnimationFrame`. A step driver
//! does one frame's platform work on the engine's [`Host`]: hand over the
//! canvas on the first frame, feed the input events queued since the last
//! one, resize, then [`Host::frame`] and [`Host::render`].

use std::cell::RefCell;
use std::sync::Mutex;

use automata_core::platform::{FrameStep, Host};
use pyo3::prelude::*;

use crate::game::PyGame;

/// One frame of platform work in a stepped run; returns what
/// [`Host::frame`] said (quit, or keep going).
pub type StepDriver = fn(&mut dyn Host) -> FrameStep;

static DRIVER: Mutex<Option<StepDriver>> = Mutex::new(None);

thread_local! {
    /// The game in a stepped run, if any: what the page's frames step.
    static RUNNING: RefCell<Option<Py<PyGame>>> = const { RefCell::new(None) };
}

/// Makes `Game.run` start stepped runs driven by `driver` (or, with
/// `None`, run its own loop again). The web build calls it when
/// `automata._core` loads.
pub fn install_step_driver(driver: Option<StepDriver>) {
    *DRIVER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = driver;
}

/// The installed step driver.
pub(crate) fn driver() -> Option<StepDriver> {
    *DRIVER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Records the game a stepped run started.
pub(crate) fn set_running(game: Option<Py<PyGame>>) {
    RUNNING.with(|slot| *slot.borrow_mut() = game);
}

/// Forgets `game` as the running one (its stepped run ended).
pub(crate) fn clear_running(game: &Bound<'_, PyGame>) {
    RUNNING.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|running| running.is(game)) {
            *slot = None;
        }
    });
}

/// Runs one frame of the game in a stepped run. Returns whether a game is
/// still running afterwards — `False` once it has quit (and when none
/// was).
///
/// # Errors
///
/// An exception from the game's update ends the run and is raised here,
/// as is a failure to step (`automata.Error`).
#[pyfunction]
pub fn step_running_game(py: Python<'_>) -> PyResult<bool> {
    let Some(game) = RUNNING.with(|slot| slot.borrow().as_ref().map(|g| g.clone_ref(py))) else {
        return Ok(false);
    };
    PyGame::step(game.bind(py), py)
}

/// The step driver `_core._use_stepped_runs(True)` installs for tests: no
/// window, just the frame and a (surface-less) render.
fn test_step_driver(host: &mut dyn Host) -> FrameStep {
    let step = host.frame();
    if let Err(err) = host.render() {
        tracing::error!("render failed: {err}");
    }
    step
}

/// Testing hook: with `True`, windowed games run stepped (with a
/// windowless driver) as they do on the web; `False` goes back to normal
/// runs.
#[pyfunction]
#[pyo3(name = "_use_stepped_runs")]
pub fn use_stepped_runs(enabled: bool) {
    install_step_driver(enabled.then_some(test_step_driver as StepDriver));
}
