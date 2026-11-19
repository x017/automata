//! Engine configuration.
//!
//! [`EngineConfig`] is a plain data struct with public fields: construct it
//! with [`EngineConfig::new`] or [`EngineConfig::default`] and adjust fields
//! directly. File-based configuration arrives with the asset system in a
//! later phase.

use crate::window::WindowMode;

/// Log filter applied when neither the configuration nor the `RUST_LOG`
/// environment variable provides one.
pub const DEFAULT_LOG_FILTER: &str = "info";

/// Configuration an [`Application`](crate::Application) is created from.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Human-readable application title.
    pub title: String,

    /// Application version string. Defaults to the engine crate version.
    pub version: String,

    /// `tracing` filter directive, e.g. `"info"` or `"automata_core=debug,wgpu=warn"`.
    ///
    /// Overridden by the `RUST_LOG` environment variable when that is set.
    pub log_filter: String,

    /// Target frame rate for the main loop.
    ///
    /// `None` disables frame pacing and lets the loop run flat out, which is
    /// useful for tests and headless tools.
    pub target_fps: Option<u32>,

    /// Whether the application opens a window or runs headless.
    ///
    /// Headless mode runs the same loop without a window, GPU surface, or
    /// input events — the mode tests, CI, and headless tooling should use.
    pub window_mode: WindowMode,

    /// Fixed physics timestep, in seconds (default 1/60).
    ///
    /// The physics world steps at this rate regardless of frame rate: the
    /// loop accumulates real time and runs as many fixed steps as fit,
    /// keeping simulation deterministic. Set to `None` to disable the
    /// physics step entirely (e.g. turn-based games).
    pub physics_timestep: Option<f32>,
}

impl EngineConfig {
    /// Creates a configuration for an application titled `title`, keeping the
    /// defaults for every other field.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            title: "AUTOMATA".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            log_filter: DEFAULT_LOG_FILTER.to_owned(),
            target_fps: Some(60),
            window_mode: WindowMode::default(),
            physics_timestep: Some(1.0 / 60.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_targets_60_fps() {
        let config = EngineConfig::default();
        assert_eq!(config.title, "AUTOMATA");
        assert_eq!(config.log_filter, DEFAULT_LOG_FILTER);
        assert_eq!(config.target_fps, Some(60));
    }

    #[test]
    fn new_sets_title_and_keeps_defaults() {
        let config = EngineConfig::new("My Game");
        assert_eq!(config.title, "My Game");
        assert_eq!(config.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(config.target_fps, Some(60));
        assert!(matches!(config.window_mode, WindowMode::Windowed(_)));
    }
}
