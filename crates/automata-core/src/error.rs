//! Error types for the engine core.
//!
//! Every subsystem crate defines its own error enum built on `thiserror` and
//! re-exports a [`Result`] alias. Errors crossing the Python boundary are
//! converted into Python exceptions by the binding layer (Phase 5).

use thiserror::Error;

/// Errors produced by the engine core.
#[derive(Debug, Error)]
pub enum Error {
    /// An operation was attempted while the application was in a state that
    /// does not permit it.
    #[error("invalid application state: expected {expected}, found {found}")]
    InvalidState {
        /// The state the operation requires.
        expected: String,
        /// The state the application was actually in.
        found: String,
    },

    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The OS event loop failed to start or run.
    #[cfg(not(any(target_os = "android", target_os = "emscripten")))]
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),

    /// Window creation failed (for example, no display is available).
    #[cfg(not(any(target_os = "android", target_os = "emscripten")))]
    #[error("window error: {0}")]
    Window(#[from] winit::error::OsError),

    /// A platform driver (SDL on Android) failed: no window, no display.
    #[error("platform error: {0}")]
    Platform(String),

    /// GPU initialization or presentation failed.
    #[error("renderer error: {0}")]
    Renderer(#[from] automata_renderer::Error),
}

/// Convenience alias for results returned by the engine core.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_state_displays_both_states() {
        let error = Error::InvalidState {
            expected: "Created".to_owned(),
            found: "Shutdown".to_owned(),
        };
        assert_eq!(
            error.to_string(),
            "invalid application state: expected Created, found Shutdown"
        );
    }

    #[test]
    fn io_errors_convert_into_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let error = Error::from(io_error);
        assert!(matches!(error, Error::Io(_)));
    }
}
