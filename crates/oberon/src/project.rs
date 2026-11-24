//! Project-level editor services: preferences (recent scenes, panel
//! layout, tool paths), autosave, and new-file templates.
//!
//! Preferences live in `$XDG_CONFIG_HOME/oberon/prefs.ron` (or
//! `~/.config/oberon/prefs.ron`). Autosaves go to `.oberon/autosave.ron`
//! inside the project so a crash never loses more than a minute of work.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ui::PanelSizes;

/// The maximum number of recent scenes remembered.
pub const RECENT_LIMIT: usize = 12;

/// Persistent editor preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Recently opened scenes, most recent first.
    pub recent: Vec<PathBuf>,
    /// Folders recently used in the file browser, most recent first.
    pub recent_dirs: Vec<PathBuf>,
    /// Hierarchy width.
    pub hierarchy_w: f32,
    /// Inspector width.
    pub inspector_w: f32,
    /// Bottom dock height.
    pub bottom_h: f32,
    /// Files panel height (bottom of the left sidebar).
    pub files_h: f32,
    /// Python interpreter for play-mode scripts (default: `python3`).
    pub python: Option<String>,
    /// Language server command line for the code editor (default: auto).
    pub language_server: Option<String>,
    /// Autosave interval in seconds (0 disables).
    pub autosave_secs: u64,
    /// Interface zoom on top of the display's scale factor (1.0 = 100%).
    pub ui_scale: f32,
    /// Show the shortcut hints in the status bar.
    pub status_hints: bool,
    /// Script editor settings.
    pub editor: crate::code::EditorSettings,
    /// Renderer quality preset (shadow map resolution).
    pub graphics_quality: automata_renderer::GraphicsQuality,
    /// Show the selected camera's view as an inset in the Scene view.
    pub camera_preview: bool,
    /// Wait for the display when presenting (frame rate = refresh rate).
    pub vsync: bool,
    /// Frame rate cap; 0 = none (V-Sync, when on, still paces frames).
    pub fps_limit: u32,
    /// Gizmo snapping (grid, angle and scale steps).
    pub snap: crate::view::Snap,
}

impl Default for Prefs {
    fn default() -> Self {
        let panels = PanelSizes::default();
        Self {
            recent: Vec::new(),
            recent_dirs: Vec::new(),
            hierarchy_w: panels.hierarchy_w,
            inspector_w: panels.inspector_w,
            bottom_h: panels.bottom_h,
            files_h: panels.files_h,
            python: None,
            language_server: None,
            autosave_secs: 60,
            ui_scale: 1.0,
            status_hints: true,
            editor: crate::code::EditorSettings::default(),
            graphics_quality: automata_renderer::GraphicsQuality::default(),
            camera_preview: true,
            vsync: true,
            fps_limit: 0,
            snap: crate::view::Snap::default(),
        }
    }
}

impl Prefs {
    /// The preferences file location.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("oberon").join("prefs.ron"))
    }

    /// Loads preferences (defaults when missing or unreadable).
    #[must_use]
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Saves preferences, creating the directory. Errors are returned for
    /// logging; preferences are never critical.
    ///
    /// # Errors
    ///
    /// Returns an error string when the file cannot be written.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no home directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Moves `path` to the front of the recent list.
    pub fn touch_recent(&mut self, path: &Path) {
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|p| p != &abs);
        self.recent.insert(0, abs);
        self.recent.truncate(RECENT_LIMIT);
    }

    /// The recent scenes that still exist.
    #[must_use]
    pub fn existing_recent(&self) -> Vec<PathBuf> {
        self.recent
            .iter()
            .filter(|p| p.is_file())
            .cloned()
            .collect()
    }

    /// Applies the saved panel sizes.
    pub fn apply_panels(&self, panels: &mut PanelSizes) {
        panels.hierarchy_w = self.hierarchy_w;
        panels.inspector_w = self.inspector_w;
        panels.bottom_h = self.bottom_h;
        panels.files_h = self.files_h;
    }

    /// Records the current panel sizes.
    pub fn capture_panels(&mut self, panels: &PanelSizes) {
        self.hierarchy_w = panels.hierarchy_w;
        self.inspector_w = panels.inspector_w;
        self.bottom_h = panels.bottom_h;
        self.files_h = panels.files_h;
    }
}

/// Where the autosave for a project lives.
#[must_use]
pub fn autosave_path(project: &Path) -> PathBuf {
    project.join(".oberon").join("autosave.ron")
}

/// Writes `ron` to the project's autosave file.
///
/// # Errors
///
/// Returns the I/O error message on failure.
pub fn write_autosave(project: &Path, ron: &str) -> Result<PathBuf, String> {
    let path = autosave_path(project);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, ron).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The starter contents of a new behaviour script named after `path`.
#[must_use]
pub fn script_template(path: &Path) -> String {
    let name = path.file_stem().map_or_else(
        || "script".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    format!(
        r#""""{name}: an AUTOMATA behaviour script.

Attach it to an entity (Inspector → Add Component → Script…). Oberon calls
`start` once and `update` every frame for that entity; `oberon run` and
exported games do the same. Type `ctx.` or `entity.` for everything you can
use, with docs.

    entity.transform.position        (x, y, z) — read / assign
    entity.character_velocity        set to move a Character Controller
    entity.attributes["health"]      custom values from the Inspector
    ctx.input.axis("a", "d")         -1, 0 or 1 from a pair of keys
    ctx.time.delta                   seconds since the last frame
    ctx.physics.grounded(entity)     standing on something?
    ctx.scene["Name"]                another entity by name
    ctx.camera.follow(entity)        the scene camera (Play renders through it)
    print(...)                       shows up in Oberon's Console
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # types for your editor; nothing is imported when it runs
    from automata.script_api import Context, Entity

SPEED = 5.0
JUMP = 7.0
GRAVITY = 20.0


def start(entity: Entity, ctx: Context) -> None:
    entity.state["vy"] = 0.0
    print(f"{{entity.name}} ready")


def update(entity: Entity, ctx: Context) -> None:
    x = ctx.input.axis("a", "d")
    z = ctx.input.axis("w", "s")
    vy = entity.state["vy"]
    if ctx.physics.grounded(entity):
        vy = JUMP if ctx.input.pressed("space") else -2.0
    else:
        vy -= GRAVITY * ctx.time.delta
    entity.state["vy"] = vy
    entity.character_velocity = (x * SPEED, vy, z * SPEED)
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_recent_is_most_recent_first() {
        let mut prefs = Prefs::default();
        prefs.touch_recent(Path::new("/tmp/a.ron"));
        prefs.touch_recent(Path::new("/tmp/b.ron"));
        prefs.touch_recent(Path::new("/tmp/a.ron"));
        assert_eq!(prefs.recent[0], PathBuf::from("/tmp/a.ron"));
        assert_eq!(prefs.recent.len(), 2);
        let text = ron::to_string(&prefs).unwrap();
        let back: Prefs = ron::from_str(&text).unwrap();
        assert_eq!(back, prefs);
        // Older files without newer fields still load.
        let partial: Prefs = ron::from_str("(recent: [])").unwrap();
        assert_eq!(partial.autosave_secs, 60);
    }

    #[test]
    fn script_template_is_valid_looking_python() {
        let t = script_template(Path::new("scripts/player.py"));
        assert!(t.starts_with("\"\"\"player:"));
        assert!(t.contains("def update(entity: Entity, ctx: Context) -> None:"));
        assert!(t.contains("from automata.script_api import Context, Entity"));
        assert!(t.contains("entity.state[\"vy\"] = 0.0"));
        assert!(
            t.contains("{entity.name} ready"),
            "format! braces unescaped"
        );
    }
}
