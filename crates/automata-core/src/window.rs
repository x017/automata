//! Window configuration.
//!
//! Window creation, event handling, and presentation are implemented by the
//! winit driver (private to this crate); this module holds the public
//! configuration types.

/// How an [`Application`](crate::Application) presents itself.
#[derive(Debug, Clone, Default)]
pub enum WindowMode {
    /// No window and no GPU surface: the loop runs headless.
    ///
    /// Used by tests, CI, headless tooling, and dedicated servers. Input
    /// state stays empty and no rendering happens.
    Headless,

    /// Create an OS window and render to it (the default).
    #[default]
    Windowed(WindowConfig),
}

/// Configuration of the application window in [`WindowMode::Windowed`].
#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// Initial inner size in logical pixels.
    pub size: (u32, u32),

    /// Whether the user may resize the window.
    pub resizable: bool,

    /// Synchronize presentation with the display's vertical blank (vsync).
    pub vsync: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            size: (1280, 720),
            resizable: true,
            vsync: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_windowed() {
        assert!(matches!(WindowMode::default(), WindowMode::Windowed(_)));
    }

    #[test]
    fn default_window_is_720p_resizable_vsync() {
        let config = WindowConfig::default();
        assert_eq!(config.size, (1280, 720));
        assert!(config.resizable);
        assert!(config.vsync);
    }
}
