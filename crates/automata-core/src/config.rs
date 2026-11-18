//! Engine configuration.
//!
//! [`EngineConfig`] is a plain data struct with public fields: construct it
//! with [`EngineConfig::new`] or [`EngineConfig::default`] and adjust fields
//! directly. File-based configuration arrives with the asset system in a
//! later phase.

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
    /// useful for tests and headless tools. Phase 2 replaces this with frame
    /// timing driven by the window event loop.
    pub target_fps: Option<u32>,
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
    }
}
