//! Platform drivers: who owns the window and the OS event loop.
//!
//! On desktop the engine's own winit driver runs windowed games. Other
//! platforms install theirs before the game starts: the Android build's
//! SDL3 driver (in `android/engine`, built in CI) calls
//! [`install_driver`] when the Python module loads. A driver gets the
//! engine as a [`Host`] and runs until the game quits or the window closes.

use std::sync::OnceLock;

use crate::error::Result;
pub use crate::runtime::{FrameStep, Host};

/// A platform driver: runs the window and event loop for one game run.
pub type Driver = fn(&mut dyn Host) -> Result<()>;

static DRIVER: OnceLock<Driver> = OnceLock::new();

/// Makes `driver` run windowed games instead of the built-in one. Only
/// the first call counts; returns whether this one did.
pub fn install_driver(driver: Driver) -> bool {
    DRIVER.set(driver).is_ok()
}

/// The installed driver, if any.
pub(crate) fn installed_driver() -> Option<Driver> {
    DRIVER.get().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quits_at_once(host: &mut dyn Host) -> Result<()> {
        assert_eq!(host.title(), "Driven");
        assert!(!host.cursor_locked());
        host.resize(0, 0); // no renderer yet: ignored
        host.input(&automata_input::InputEvent::CursorMoved { x: 4.0, y: 2.0 });
        while host.frame() != FrameStep::Quit {}
        host.render()
    }

    #[test]
    fn an_installed_driver_runs_windowed_games() {
        assert!(install_driver(quits_at_once));
        assert!(!install_driver(quits_at_once), "only the first counts");
        let mut app = crate::Application::new(crate::EngineConfig {
            target_fps: None,
            ..crate::EngineConfig::new("Driven")
        });
        let mut frames = 0;
        let mut cursor = None;
        app.run(|ctx| {
            frames += 1;
            cursor = ctx.input().cursor_position();
            if frames == 3 {
                ctx.request_quit();
            }
        })
        .unwrap();
        assert_eq!(frames, 3);
        assert_eq!(cursor, Some((4.0, 2.0)));
    }
}
