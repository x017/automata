//! The Settings dialog (Edit ▸ Settings…, Ctrl+,): interface, script
//! editor, Python tooling and file handling. Changes apply immediately and
//! are saved to the preferences file.

use super::{ButtonStyle, CursorHint, EditTarget, Intent, Rect, TextEdit, Ui};
use crate::canvas::Canvas;
use crate::theme::{color, layout};

/// A page of the Settings dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    /// Interface scale and chrome.
    #[default]
    Appearance,
    /// The script editor.
    Editor,
    /// Python interpreter and language server.
    Python,
    /// Autosave and recent files.
    Files,
    /// Game export (this project's `build.ron`).
    Build,
}

impl SettingsPage {
    const ALL: [Self; 5] = [
        Self::Appearance,
        Self::Editor,
        Self::Python,
        Self::Files,
        Self::Build,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Script Editor",
            Self::Python => "Python",
            Self::Files => "Files",
            Self::Build => "Build",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Self::Appearance => "How Oberon looks on your display.",
            Self::Editor => "Typing, display and saving in the built-in script editor.",
            Self::Python => "The interpreter that runs Play mode scripts and the language server behind completions.",
            Self::Files => "Autosave and recently opened scenes.",
            Self::Build => "Export this project as a game for Windows, Linux, macOS, Android and the web. Saved to build.ron in the project folder.",
        }
    }
}

/// A free-text setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingText {
    /// The Python interpreter for Play mode scripts.
    Python,
    /// The language-server command line.
    LanguageServer,
    /// Build: the game's name.
    BuildName,
    /// Build: the version.
    BuildVersion,
    /// Build: the start scene.
    BuildScene,
    /// Build: the output folder.
    BuildOutput,
    /// Build: the Android app ID.
    BuildPackage,
    /// Build: the Android version code.
    BuildVersionCode,
    /// Build: the Android release key file.
    BuildKey,
    /// Build: the Android processor types.
    BuildAbis,
    /// Build: the macOS bundle ID.
    BuildBundleId,
    /// Build: the macOS app icon.
    BuildIcon,
    /// Build: the Steam app ID.
    BuildSteamAppId,
    /// Build: the Steamworks SDK folder.
    BuildSteamSdk,
}

const ROW: f32 = 52.0;

/// A Script Editor toggle: label, hint and the setting it flips.
type ToggleRow = (
    &'static str,
    &'static str,
    fn(&mut crate::code::EditorSettings) -> &mut bool,
);
const AUTOSAVE: [(u64, &str); 5] = [
    (0, "Off"),
    (30, "30 s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
];

impl Ui {
    /// Opens the Settings dialog on `page`.
    pub fn open_settings_page(&mut self, page: SettingsPage) {
        self.settings_page = page;
        self.open_settings();
    }

    /// Opens Settings on the last-viewed page.
    pub fn open_settings(&mut self) {
        self.menu = None;
        self.picker = None;
        self.palette = None;
        self.dialog = Some(super::dialogs::Dialog::Settings(self.settings_page));
    }

    /// Applies a committed settings text field.
    pub(super) fn commit_setting(&mut self, which: SettingText, text: &str) -> Option<Intent> {
        let value = Some(text.trim().to_string()).filter(|t| !t.is_empty());
        let slot = match which {
            SettingText::Python => &mut self.prefs.python,
            SettingText::LanguageServer => &mut self.prefs.language_server,
            SettingText::BuildName
            | SettingText::BuildVersion
            | SettingText::BuildScene
            | SettingText::BuildOutput
            | SettingText::BuildPackage
            | SettingText::BuildVersionCode
            | SettingText::BuildKey
            | SettingText::BuildAbis
            | SettingText::BuildBundleId
            | SettingText::BuildIcon
            | SettingText::BuildSteamAppId
            | SettingText::BuildSteamSdk => {
                self.commit_build_text(which, value);
                return None;
            }
        };
        if *slot == value {
            return None;
        }
        *slot = value;
        self.prefs_dirty = true;
        Some(Intent::RestartLanguageServer)
    }

    /// Draws the Settings dialog body inside `r`.
    pub(super) fn draw_settings(&mut self, canvas: &mut Canvas, r: Rect, page: SettingsPage) {
        // Header.
        canvas.text_in(
            crate::canvas::Face::Medium,
            r.x + 24.0,
            r.y + 17.0,
            16.0,
            "Settings",
            color::text(),
        );
        let saved = "Changes save automatically";
        let sw = canvas.measure_text(layout::SMALL_SIZE, saved);
        canvas.text(
            r.x + r.w - sw - 56.0,
            r.y + 22.0,
            layout::SMALL_SIZE,
            saved,
            color::text_faint(),
        );
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 42.0, r.y + 14.0, 26.0, 26.0),
            "",
            ButtonStyle::Ghost,
        ) {
            self.dialog = None;
            return;
        }
        let (cx, cy) = (r.x + r.w - 29.0, r.y + 27.0);
        canvas.line(
            cx - 4.0,
            cy - 4.0,
            cx + 4.0,
            cy + 4.0,
            color::text_dim(),
            1.4,
        );
        canvas.line(
            cx + 4.0,
            cy - 4.0,
            cx - 4.0,
            cy + 4.0,
            color::text_dim(),
            1.4,
        );
        let top = r.y + 54.0;
        canvas.line(r.x, top, r.x + r.w, top, color::border(), 1.0);

        // Navigation.
        let nav = Rect::new(r.x, top, 190.0, r.h - (top - r.y));
        canvas.line(
            nav.x + nav.w,
            nav.y,
            nav.x + nav.w,
            nav.y + nav.h - 10.0,
            color::border(),
            1.0,
        );
        let mut y = nav.y + 12.0;
        for p in SettingsPage::ALL {
            let row = Rect::new(nav.x + 10.0, y, nav.w - 20.0, 32.0);
            let active = p == page;
            let hot = self.hot(row);
            if active {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, layout::RADIUS, color::select());
            } else if hot {
                canvas.fill_rounded(row.x, row.y, row.w, row.h, layout::RADIUS, color::hover());
            }
            let fg = if active {
                color::text()
            } else {
                color::text_dim()
            };
            canvas.text(row.x + 14.0, row.y + 9.0, layout::FONT_SIZE, p.title(), fg);
            if hot {
                self.cursor = CursorHint::Pointer;
                if self.p.pressed {
                    self.settings_page = p;
                    self.dialog = Some(super::dialogs::Dialog::Settings(p));
                    self.edit = None;
                }
            }
            y += 36.0;
        }

        // Page.
        let body = Rect::new(nav.x + nav.w + 24.0, top + 18.0, r.w - nav.w - 48.0, 0.0);
        canvas.text(body.x, body.y, 15.0, page.title(), color::text());
        canvas.text_clipped(
            body.x,
            body.y + 24.0,
            layout::SMALL_SIZE,
            page.blurb(),
            color::text_dim(),
            body.w,
        );
        let mut y = body.y + 54.0;
        match page {
            SettingsPage::Appearance => self.page_appearance(canvas, body, &mut y),
            SettingsPage::Editor => self.page_editor(canvas, body, &mut y),
            SettingsPage::Python => self.page_python(canvas, body, &mut y),
            SettingsPage::Files => self.page_files(canvas, body, &mut y),
            SettingsPage::Build => self.page_build(canvas, body, &mut y),
        }

        // Footer.
        let fy = r.y + r.h - 46.0;
        canvas.line(
            nav.x + nav.w,
            fy - 10.0,
            r.x + r.w,
            fy - 10.0,
            color::border(),
            1.0,
        );
        if self.button(
            canvas,
            Rect::new(body.x, fy, 150.0, 28.0),
            "Reset this page",
            ButtonStyle::Normal,
        ) {
            self.reset_page(page);
        }
        if self.button(
            canvas,
            Rect::new(r.x + r.w - 110.0, fy, 92.0, 28.0),
            "Done",
            ButtonStyle::Accent,
        ) {
            self.dialog = None;
            self.edit = None;
        }
    }

    fn reset_page(&mut self, page: SettingsPage) {
        let d = crate::project::Prefs::default();
        match page {
            SettingsPage::Appearance => {
                self.prefs.ui_scale = d.ui_scale;
                self.prefs.status_hints = d.status_hints;
            }
            SettingsPage::Editor => self.prefs.editor = d.editor,
            SettingsPage::Python => {
                if self.prefs.python.is_some() || self.prefs.language_server.is_some() {
                    self.out.push(Intent::RestartLanguageServer);
                }
                self.prefs.python = None;
                self.prefs.language_server = None;
            }
            SettingsPage::Files => self.prefs.autosave_secs = d.autosave_secs,
            SettingsPage::Build => {
                let cfg = self.build_settings();
                let fresh = crate::build::BuildConfig {
                    name: cfg.name.clone(),
                    ..crate::build::BuildConfig::default()
                };
                *cfg = fresh;
                self.save_build_settings();
                self.edit = None;
                return;
            }
        }
        self.edit = None;
        self.prefs_dirty = true;
    }

    fn page_appearance(&mut self, canvas: &mut Canvas, body: Rect, y: &mut f32) {
        let pct = (self.prefs.ui_scale * 100.0).round();
        if let Some(d) = self.setting_stepper(
            canvas,
            body,
            *y,
            "Interface scale",
            "Zoom on top of your display's scaling. Ctrl+= / Ctrl+- / Ctrl+0.",
            &format!("{pct}%"),
        ) {
            self.prefs.ui_scale = ((pct + d * 10.0) / 100.0).clamp(0.6, 2.5);
            self.prefs_dirty = true;
        }
        *y += ROW;
        let on = self.prefs.status_hints;
        if self.setting_toggle(
            canvas,
            body,
            *y,
            "Shortcut hints",
            "Show keyboard hints in the status bar.",
            on,
        ) {
            self.prefs.status_hints = !on;
            self.prefs_dirty = true;
        }
        *y += ROW;
        self.setting_info(
            canvas,
            body,
            *y,
            "Theme",
            "AUTOMATA Black — neutral blacks, paper-white accents.",
        );
    }

    fn page_editor(&mut self, canvas: &mut Canvas, body: Rect, y: &mut f32) {
        let size = self.prefs.editor.font_size;
        if let Some(d) = self.setting_stepper(
            canvas,
            body,
            *y,
            "Font size",
            "Code font size (Fira Code).",
            &format!("{size:.0} px"),
        ) {
            self.prefs.editor.font_size = (size + d).clamp(9.0, 28.0);
            self.prefs_dirty = true;
        }
        *y += ROW;
        let pct = (self.editor_scale() * 100.0).round();
        if let Some(d) = self.setting_stepper(
            canvas,
            body,
            *y,
            "Editor scale",
            "Zooms the script editor only, independent of the interface scale. Ctrl+wheel.",
            &format!("{pct}%"),
        ) {
            self.prefs.editor.scale = ((pct + d * 10.0) / 100.0).clamp(0.5, 3.0);
            self.prefs_dirty = true;
        }
        *y += ROW;
        let rows: [ToggleRow; 7] = [
            (
                "Highlight current line",
                "A subtle band behind the caret's line.",
                |e| &mut e.highlight_line,
            ),
            (
                "Indentation guides",
                "Vertical lines at each indentation level.",
                |e| &mut e.indent_guides,
            ),
            (
                "Show whitespace",
                "Dots for spaces, arrows for tabs.",
                |e| &mut e.show_whitespace,
            ),
            (
                "Auto-close brackets and quotes",
                "Typing ( [ { \" ' inserts the closer.",
                |e| &mut e.auto_close,
            ),
            (
                "Suggest while typing",
                "Completions after a dot or two letters (Ctrl+Space always works).",
                |e| &mut e.auto_complete,
            ),
            (
                "Trim trailing whitespace",
                "Strip spaces at line ends when saving.",
                |e| &mut e.trim_on_save,
            ),
            (
                "Save scripts before Play",
                "Modified scripts are saved when you press Play.",
                |e| &mut e.save_before_play,
            ),
        ];
        for (label, hint, get) in rows {
            let on = *get(&mut self.prefs.editor);
            if self.setting_toggle(canvas, body, *y, label, hint, on) {
                *get(&mut self.prefs.editor) = !on;
                self.prefs_dirty = true;
            }
            *y += ROW - 4.0;
        }
    }

    fn page_python(&mut self, canvas: &mut Canvas, body: Rect, y: &mut f32) {
        let python = self.prefs.python.clone().unwrap_or_default();
        self.setting_text(
            canvas,
            body,
            *y,
            "Interpreter",
            "Runs Play mode scripts. Empty = python3 on PATH.",
            SettingText::Python,
            &python,
            crate::bridge::default_python(),
        );
        *y += ROW + 8.0;
        let ls = self.prefs.language_server.clone().unwrap_or_default();
        self.setting_text(
            canvas,
            body,
            *y,
            "Language server",
            "A command line, \"off\", or empty to auto-detect.",
            SettingText::LanguageServer,
            &ls,
            "auto  (basedpyright · pyright · pylsp · jedi)",
        );
        *y += ROW + 8.0;
        let (status, c) = match &self.code.server {
            Some(name) => (format!("Running: {name}"), color::success()),
            None if self.code.tabs.is_empty() => (
                "Starts when you open a script".to_string(),
                color::text_dim(),
            ),
            None => (
                "Not running — word completion only".to_string(),
                color::warn(),
            ),
        };
        canvas.text(body.x, *y + 4.0, layout::FONT_SIZE, "Status", color::text());
        canvas.fill_circle(body.x + 200.0, *y + 11.0, 4.0, c);
        canvas.text(body.x + 212.0, *y + 4.0, layout::FONT_SIZE, &status, c);
        if self.button(
            canvas,
            Rect::new(body.x + body.w - 120.0, *y - 2.0, 120.0, 26.0),
            "Restart server",
            ButtonStyle::Normal,
        ) {
            self.out.push(Intent::RestartLanguageServer);
        }
        *y += ROW;
        self.setting_info(
            canvas,
            body,
            *y,
            "Script API",
            "ctx. and entity. completions, key names and entity names are built in — no import needed.",
        );
    }

    fn page_files(&mut self, canvas: &mut Canvas, body: Rect, y: &mut f32) {
        canvas.text(body.x, *y, layout::FONT_SIZE, "Autosave", color::text());
        canvas.text(
            body.x,
            *y + 18.0,
            layout::SMALL_SIZE,
            "Writes .oberon/autosave.ron while the scene has unsaved changes.",
            color::text_dim(),
        );
        *y += 40.0;
        let mut x = body.x;
        for (secs, label) in AUTOSAVE {
            let w = canvas.measure_text(layout::FONT_SIZE, label) + 26.0;
            let on = self.prefs.autosave_secs == secs;
            let style = if on {
                ButtonStyle::Active
            } else {
                ButtonStyle::Normal
            };
            if self.button(canvas, Rect::new(x, *y, w, 26.0), label, style) {
                self.prefs.autosave_secs = secs;
                self.prefs_dirty = true;
            }
            x += w + 6.0;
        }
        *y += ROW;
        let n = self.prefs.recent.len();
        canvas.text(
            body.x,
            *y,
            layout::FONT_SIZE,
            "Recent scenes",
            color::text(),
        );
        let hint = format!("{n} remembered (File ▸ Open Recent).");
        canvas.text(
            body.x,
            *y + 18.0,
            layout::SMALL_SIZE,
            &hint,
            color::text_dim(),
        );
        let style = if n == 0 {
            ButtonStyle::Disabled
        } else {
            ButtonStyle::Normal
        };
        if self.button(
            canvas,
            Rect::new(body.x + body.w - 120.0, *y + 2.0, 120.0, 26.0),
            "Clear list",
            style,
        ) {
            self.prefs.recent.clear();
            self.recent.clear();
            self.prefs_dirty = true;
        }
        *y += ROW;
        let path = crate::project::Prefs::path()
            .map_or_else(|| "(unavailable)".to_string(), |p| p.display().to_string());
        self.setting_info(canvas, body, *y, "Preferences file", &path);
    }

    // ----- build ---------------------------------------------------------

    /// The Build page's settings, loaded from the project on first use.
    pub(super) fn build_settings(&mut self) -> &mut crate::build::BuildConfig {
        let root = self.project_root.clone();
        if self.build_cfg.as_ref().is_none_or(|(r, _)| *r != root) {
            let cfg = crate::build::config_for(&root, None);
            self.build_cfg = Some((root, cfg));
        }
        &mut self.build_cfg.as_mut().expect("just set").1
    }

    /// Loads Build Settings for the project, starting in `scene` when the
    /// project has no `build.ron` yet.
    pub(super) fn load_build_settings(&mut self, scene: Option<&std::path::Path>) {
        let root = self.project_root.clone();
        let cfg = crate::build::config_for(&root, scene);
        self.build_cfg = Some((root, cfg));
    }

    fn save_build_settings(&mut self) {
        let root = self.project_root.clone();
        let result = self.build_settings().save(&root);
        if let Err(e) = result {
            self.log(super::LogLevel::Error, format!("Build settings: {e}"));
        }
    }

    fn commit_build_text(&mut self, which: SettingText, value: Option<String>) {
        // The Android fields only take valid values.
        let problem = match (which, value.as_deref()) {
            (SettingText::BuildPackage, Some(v)) => crate::build::check_android_package(v).err(),
            (SettingText::BuildVersionCode, Some(v)) => match v.parse::<u32>() {
                Ok(n) if n >= 1 => None,
                _ => Some(format!(
                    "the Android version code must be a whole number from 1 up, not `{v}`"
                )),
            },
            (SettingText::BuildAbis, Some(v)) => crate::build::parse_android_abis(v).err(),
            (SettingText::BuildBundleId, Some(v)) => crate::build::check_bundle_id(v).err(),
            (SettingText::BuildSteamAppId, Some(v)) => match v.parse::<u32>() {
                Ok(n) if n >= 1 => None,
                _ => Some(format!(
                    "the Steam app ID is the number on the game's Steamworks page, not `{v}`"
                )),
            },
            (SettingText::BuildKey, Some(v)) => {
                let root = self.project_root.clone();
                crate::build::release_key_path(&root, v)
                    .map(|p| crate::build::load_release_key(&p))
                    .and_then(Result::err)
            }
            _ => None,
        };
        if let Some(problem) = problem {
            self.log(super::LogLevel::Error, format!("Build settings: {problem}"));
            return;
        }
        let cfg = self.build_settings();
        let defaults = crate::build::BuildConfig::default();
        match which {
            SettingText::BuildName => cfg.name = value.unwrap_or(defaults.name),
            SettingText::BuildVersion => cfg.version = value.unwrap_or(defaults.version),
            SettingText::BuildScene => {
                cfg.start_scene = value.map(|v| v.replace('\\', "/"));
                if cfg.start_scene.is_some() {
                    cfg.main_script = None;
                }
            }
            SettingText::BuildOutput => cfg.output = value.unwrap_or(defaults.output),
            SettingText::BuildPackage => cfg.android.package = value.unwrap_or_default(),
            SettingText::BuildVersionCode => {
                cfg.android.version_code = value
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(defaults.android.version_code);
            }
            SettingText::BuildKey => cfg.android.key = value.unwrap_or_default(),
            SettingText::BuildAbis => {
                cfg.android.abis = value
                    .and_then(|v| crate::build::parse_android_abis(&v).ok())
                    .unwrap_or_default();
            }
            SettingText::BuildBundleId => cfg.macos.bundle_id = value.unwrap_or_default(),
            SettingText::BuildIcon => {
                cfg.macos.icon = value.map(|v| v.replace('\\', "/")).unwrap_or_default();
            }
            SettingText::BuildSteamAppId => {
                cfg.steam.app_id = value.and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            SettingText::BuildSteamSdk => cfg.steam.sdk = value.unwrap_or_default(),
            SettingText::Python | SettingText::LanguageServer => return,
        }
        self.save_build_settings();
    }

    /// Makes a release key for the project in AUTOMATA's settings folder
    /// and uses it.
    fn new_release_key(&mut self) {
        let cfg = self.build_settings().clone();
        let path = crate::build::suggested_release_key_path(&cfg.android_package());
        let result = if path.exists() {
            crate::build::load_release_key(&path).map(|_| "Using the existing")
        } else {
            crate::build::create_release_key(&path, cfg.name.trim()).map(|_| "Made a")
        };
        match result {
            Ok(what) => {
                self.log(
                    super::LogLevel::Info,
                    format!(
                        "{what} release key {}. Back it up and keep it private: every update of the game must be signed with it.",
                        path.display()
                    ),
                );
                self.build_settings().android.key = path.display().to_string();
                self.save_build_settings();
            }
            Err(e) => self.log(super::LogLevel::Error, format!("Build settings: {e}")),
        }
    }

    fn page_build(&mut self, canvas: &mut Canvas, body: Rect, y: &mut f32) {
        use crate::build::{Target, TemplateState};
        let cfg = self.build_settings().clone();
        let half = (body.w - 16.0) / 2.0;
        let right = body.x + half + 16.0;
        let package_hint = cfg.android_package();
        let bundle_hint = cfg.macos_bundle_id();
        let pairs = [
            (
                "Game name",
                SettingText::BuildName,
                cfg.name.clone(),
                "My Game",
                "Version",
                SettingText::BuildVersion,
                cfg.version.clone(),
                "0.1.0",
            ),
            (
                "Start scene",
                SettingText::BuildScene,
                cfg.start_scene.clone().unwrap_or_default(),
                cfg.main_script
                    .as_deref()
                    .map_or("The open scene", |_| "None — runs the main script"),
                "Output folder",
                SettingText::BuildOutput,
                cfg.output.clone(),
                "build",
            ),
            (
                "Android app ID",
                SettingText::BuildPackage,
                cfg.android.package.clone(),
                package_hint.as_str(),
                "Android version code",
                SettingText::BuildVersionCode,
                cfg.android.version_code.to_string(),
                "1",
            ),
            (
                "Android release key",
                SettingText::BuildKey,
                cfg.android.key.clone(),
                "Debug key — for testing only",
                "Android processors",
                SettingText::BuildAbis,
                cfg.android.abis.join(", "),
                "arm64-v8a, x86_64",
            ),
            (
                "macOS bundle ID",
                SettingText::BuildBundleId,
                cfg.macos.bundle_id.clone(),
                bundle_hint.as_str(),
                "macOS icon",
                SettingText::BuildIcon,
                cfg.macos.icon.clone(),
                "A square PNG, e.g. icon.png",
            ),
            (
                "Steam app ID",
                SettingText::BuildSteamAppId,
                if cfg.steam.app_id == 0 {
                    String::new()
                } else {
                    cfg.steam.app_id.to_string()
                },
                "None — no Steam",
                "Steamworks SDK",
                SettingText::BuildSteamSdk,
                cfg.steam.sdk.clone(),
                "The SDK folder (from Steamworks)",
            ),
        ];
        for (l1, w1, v1, p1, l2, w2, v2, p2) in pairs {
            canvas.text(body.x, *y, layout::FONT_SIZE, l1, color::text());
            canvas.text(right, *y, layout::FONT_SIZE, l2, color::text());
            for (x, which, value, placeholder) in [(body.x, w1, &v1, p1), (right, w2, &v2, p2)] {
                let browse = match which {
                    SettingText::BuildScene => Some(super::DialogKind::BuildScene),
                    SettingText::BuildOutput => Some(super::DialogKind::BuildOutput),
                    SettingText::BuildKey => Some(super::DialogKind::BuildKey),
                    SettingText::BuildSteamSdk => Some(super::DialogKind::BuildSteamSdk),
                    _ => None,
                };
                // The key field also has a New button, when no key is set.
                let new_key = which == SettingText::BuildKey && value.is_empty();
                let buttons = match (browse.is_some(), new_key) {
                    (_, true) => 36.0 + 56.0,
                    (true, false) => 36.0,
                    (false, false) => 0.0,
                };
                let field_w = half - buttons;
                self.setting_field(
                    canvas,
                    Rect::new(x, *y + 20.0, field_w, 26.0),
                    which,
                    value,
                    placeholder,
                );
                if let Some(kind) = browse {
                    let bx = x + half - 32.0 - if new_key { 56.0 } else { 0.0 };
                    self.browse_button(canvas, Rect::new(bx, *y + 20.0, 32.0, 26.0), kind);
                }
                if new_key {
                    let b = Rect::new(x + half - 52.0, *y + 20.0, 52.0, 26.0);
                    if self.button(canvas, b, "New", ButtonStyle::Normal) {
                        self.new_release_key();
                    }
                }
            }
            *y += 54.0;
        }

        // Platforms, right to left.
        let platforms: Vec<(Target, bool, String, f32)> = [
            Target::Web,
            Target::Android,
            Target::MacOS,
            Target::Windows,
            Target::Linux,
        ]
        .into_iter()
        .map(|t| {
            let on = cfg.targets.contains(&t);
            let label = if on {
                format!("✓ {}", t.label())
            } else {
                t.label().to_string()
            };
            let w = canvas.measure_text(layout::FONT_SIZE, &label) + 26.0;
            (t, on, label, w)
        })
        .collect();
        let buttons: f32 = platforms.iter().map(|p| p.3 + 6.0).sum();
        Self::setting_label(
            canvas,
            body,
            *y,
            "Platforms",
            "Build (Ctrl+Shift+B) exports every ticked platform.",
            buttons,
        );
        let mut x = body.x + body.w;
        for (t, on, label, w) in platforms {
            x -= w;
            let style = if on {
                ButtonStyle::Active
            } else {
                ButtonStyle::Normal
            };
            if self.button(canvas, Rect::new(x, *y + 2.0, w, 26.0), &label, style) {
                let targets = &mut self.build_settings().targets;
                if on {
                    targets.retain(|x| *x != t);
                } else {
                    targets.push(t);
                    targets.sort_by_key(|t| t.name());
                }
                self.save_build_settings();
            }
            x -= 6.0;
        }
        *y += ROW;
        if self.setting_toggle(
            canvas,
            body,
            *y,
            "Archive",
            "Also write a .zip (Windows, macOS, the web: itch.io-ready) or .tar.gz (Linux), ready to upload. Android builds are an .apk.",
            cfg.archive,
        ) {
            self.build_settings().archive = !cfg.archive;
            self.save_build_settings();
        }
        *y += ROW;
        let touch = cfg.touch_controls.is_some();
        if self.setting_toggle(
            canvas,
            body,
            *y,
            "Touch controls",
            "An on-screen stick (WASD) and Jump button (Space) on phones. Change their keys in build.ron.",
            touch,
        ) {
            self.build_settings().touch_controls =
                (!touch).then(crate::build::starter_touch_controls);
            self.save_build_settings();
        }
        *y += ROW;

        // Templates.
        canvas.text(
            body.x,
            *y,
            layout::FONT_SIZE,
            "Export templates",
            color::text(),
        );
        let dir = crate::build::templates_dir_label();
        canvas.text_clipped(
            body.x,
            *y + 18.0,
            layout::SMALL_SIZE,
            &dir,
            color::text_dim(),
            body.w,
        );
        *y += 42.0;
        let idle = self.build_status.is_none();
        // Two columns: a dot, the platform, its state and a button.
        let cell = (body.w - 16.0) / 2.0;
        for (i, t) in Target::ALL.into_iter().enumerate() {
            let x = body.x + if i % 2 == 0 { 0.0 } else { cell + 16.0 };
            let ty = *y + (i / 2) as f32 * 30.0;
            let (state, tip, c, action) = match crate::build::template_state(t) {
                TemplateState::Ready => (
                    "Ready",
                    "Installed: the game ships its own Python.",
                    color::success(),
                    "Update",
                ),
                TemplateState::Development => (
                    "Development",
                    "A development template: the game uses the Python on PATH. Download the release one to ship.",
                    color::warn(),
                    "Download",
                ),
                TemplateState::Missing => (
                    "Not installed",
                    "Download it to build for this platform.",
                    color::text_faint(),
                    "Download",
                ),
            };
            canvas.fill_circle(x + 4.0, ty + 8.0, 3.5, c);
            canvas.text(
                x + 16.0,
                ty + 1.0,
                layout::FONT_SIZE,
                t.label(),
                color::text(),
            );
            let state_x = x + 90.0;
            let button_w = 86.0;
            canvas.text_clipped(
                state_x,
                ty + 1.0,
                layout::FONT_SIZE,
                state,
                color::text_dim(),
                cell - 90.0 - button_w - 8.0,
            );
            self.tip(Rect::new(x, ty - 4.0, cell - button_w - 8.0, 24.0), tip);
            let style = if idle {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Disabled
            };
            if self.button(
                canvas,
                Rect::new(x + cell - button_w, ty - 4.0, button_w, 24.0),
                action,
                style,
            ) && idle
            {
                self.out.push(Intent::DownloadTemplates(Some(t)));
            }
        }
        *y += Target::ALL.len().div_ceil(2) as f32 * 30.0 + 4.0;

        // Actions.
        let style = if idle {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Disabled
        };
        if self.button(
            canvas,
            Rect::new(body.x, *y, 120.0, 28.0),
            "Build now",
            style,
        ) && idle
        {
            self.out.push(Intent::Build(None));
        }
        if self.button(
            canvas,
            Rect::new(body.x + 128.0, *y, 150.0, 28.0),
            "Open build folder",
            ButtonStyle::Normal,
        ) {
            self.out.push(Intent::OpenBuildFolder);
        }
        let (status, c) = match (&self.build_status, &self.build_last) {
            (Some(s), _) => (s.clone(), color::text()),
            (None, Some((true, s))) => (s.clone(), color::success()),
            (None, Some((false, s))) => (s.clone(), color::danger()),
            (None, None) => (String::new(), color::text_dim()),
        };
        canvas.text_clipped(
            body.x + 292.0,
            *y + 7.0,
            layout::SMALL_SIZE,
            &status,
            c,
            body.w - 292.0,
        );
    }

    // ----- rows ------------------------------------------------------------

    fn setting_label(canvas: &mut Canvas, body: Rect, y: f32, label: &str, hint: &str, right: f32) {
        canvas.text(body.x, y, layout::FONT_SIZE, label, color::text());
        canvas.text_clipped(
            body.x,
            y + 18.0,
            layout::SMALL_SIZE,
            hint,
            color::text_dim(),
            body.w - right - 16.0,
        );
    }

    fn setting_info(&mut self, canvas: &mut Canvas, body: Rect, y: f32, label: &str, text: &str) {
        canvas.text(body.x, y, layout::FONT_SIZE, label, color::text());
        canvas.text_clipped(
            body.x,
            y + 18.0,
            layout::SMALL_SIZE,
            text,
            color::text_dim(),
            body.w,
        );
    }

    /// A labelled on/off switch; returns true when clicked.
    fn setting_toggle(
        &mut self,
        canvas: &mut Canvas,
        body: Rect,
        y: f32,
        label: &str,
        hint: &str,
        on: bool,
    ) -> bool {
        Self::setting_label(canvas, body, y, label, hint, 50.0);
        let sw = Rect::new(body.x + body.w - 40.0, y + 4.0, 40.0, 22.0);
        let row = Rect::new(body.x, y - 4.0, body.w, 44.0);
        let hot = self.hot(row);
        let track = if on {
            color::accent()
        } else if hot {
            color::hover()
        } else {
            color::raised()
        };
        canvas.fill_rounded(sw.x, sw.y, sw.w, sw.h, sw.h / 2.0, track);
        if !on {
            canvas.stroke_rounded(sw.x, sw.y, sw.w, sw.h, sw.h / 2.0, color::border(), 1.0);
        }
        let knob_x = if on { sw.x + sw.w - 11.0 } else { sw.x + 11.0 };
        let knob = if on {
            color::on_accent()
        } else {
            color::text_dim()
        };
        canvas.fill_circle(knob_x, sw.y + sw.h / 2.0, 7.5, knob);
        if hot {
            self.cursor = CursorHint::Pointer;
        }
        hot && self.p.pressed
    }

    /// A labelled − value + stepper; returns −1 / +1 when clicked.
    fn setting_stepper(
        &mut self,
        canvas: &mut Canvas,
        body: Rect,
        y: f32,
        label: &str,
        hint: &str,
        value: &str,
    ) -> Option<f32> {
        Self::setting_label(canvas, body, y, label, hint, 140.0);
        let x = body.x + body.w - 132.0;
        let minus = self.button(
            canvas,
            Rect::new(x, y + 2.0, 30.0, 26.0),
            "−",
            ButtonStyle::Normal,
        );
        canvas.fill_rounded(
            x + 34.0,
            y + 2.0,
            64.0,
            26.0,
            layout::RADIUS,
            color::field(),
        );
        let vw = canvas.measure_text(layout::FONT_SIZE, value);
        canvas.text(
            x + 66.0 - vw / 2.0,
            y + 7.0,
            layout::FONT_SIZE,
            value,
            color::text(),
        );
        let plus = self.button(
            canvas,
            Rect::new(x + 102.0, y + 2.0, 30.0, 26.0),
            "+",
            ButtonStyle::Normal,
        );
        if minus {
            Some(-1.0)
        } else if plus {
            Some(1.0)
        } else {
            None
        }
    }

    /// A labelled text field (click to edit, Enter to apply).
    #[allow(clippy::too_many_arguments)]
    fn setting_text(
        &mut self,
        canvas: &mut Canvas,
        body: Rect,
        y: f32,
        label: &str,
        hint: &str,
        which: SettingText,
        value: &str,
        placeholder: &str,
    ) {
        canvas.text(body.x, y, layout::FONT_SIZE, label, color::text());
        canvas.text_clipped(
            body.x,
            y + 18.0,
            layout::SMALL_SIZE,
            hint,
            color::text_dim(),
            body.w,
        );
        if which == SettingText::Python {
            let field = Rect::new(body.x, y + 36.0, body.w - 36.0, 26.0);
            self.setting_field(canvas, field, which, value, placeholder);
            let b = Rect::new(body.x + body.w - 32.0, y + 36.0, 32.0, 26.0);
            self.browse_button(canvas, b, super::DialogKind::PythonInterpreter);
            return;
        }
        let field = Rect::new(body.x, y + 36.0, body.w, 26.0);
        self.setting_field(canvas, field, which, value, placeholder);
    }

    /// A "…" button next to a path field: opens the file browser (which
    /// comes back to Settings).
    fn browse_button(&mut self, canvas: &mut Canvas, r: Rect, kind: super::DialogKind) {
        self.tip(r, "Browse…");
        if self.button(canvas, r, "…", ButtonStyle::Normal) {
            self.edit = None;
            self.open_file_dialog(kind, None);
        }
    }

    /// A text field; click to edit, Enter (or clicking away) applies.
    fn setting_field(
        &mut self,
        canvas: &mut Canvas,
        field: Rect,
        which: SettingText,
        value: &str,
        placeholder: &str,
    ) {
        let editing = self
            .edit
            .as_ref()
            .is_some_and(|e| e.target == EditTarget::Setting(which));
        canvas.fill_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            layout::RADIUS,
            color::field(),
        );
        let border = if editing {
            color::accent()
        } else {
            color::border()
        };
        canvas.stroke_rounded(
            field.x,
            field.y,
            field.w,
            field.h,
            layout::RADIUS,
            border,
            1.0,
        );
        if editing {
            self.draw_edit_text(canvas, field, field.x + 8.0);
        } else {
            let (text, c) = if value.is_empty() {
                (placeholder, color::text_faint())
            } else {
                (value, color::text())
            };
            canvas.text_clipped(
                field.x + 8.0,
                field.y + 6.0,
                layout::FONT_SIZE,
                text,
                c,
                field.w - 16.0,
            );
            if self.hot(field) {
                self.cursor = CursorHint::Text;
                if self.p.pressed {
                    if let Some(intent) = self.commit_edit() {
                        self.out.push(intent);
                    }
                    self.edit = Some(TextEdit {
                        id: 0,
                        target: EditTarget::Setting(which),
                        buffer: value.to_string(),
                        rect: field,
                        fresh: false,
                        attr: None,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Pointer;
    use super::*;
    use crate::doc::Document;

    fn click(ui: &mut Ui, c: &mut Canvas, doc: &Document, x: f32, y: f32) {
        let press = Pointer {
            x,
            y,
            down: true,
            pressed: true,
            ..Pointer::default()
        };
        ui.frame(c, doc, press, None);
        ui.frame(
            c,
            doc,
            Pointer {
                x,
                y,
                ..Pointer::default()
            },
            None,
        );
    }

    #[test]
    fn the_build_page_fits_above_the_footer() {
        let mut c = Canvas::new(1600, 950);
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.open_settings_page(SettingsPage::Build);
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let r = ui.dialog_rect(1600.0, 950.0).unwrap();
        let body = Rect::new(
            r.x + 190.0 + 24.0,
            r.y + 54.0 + 18.0,
            r.w - 190.0 - 48.0,
            0.0,
        );
        let mut y = body.y + 54.0;
        ui.page_build(&mut c, body, &mut y);
        // `y` is the top of the Build now row; its buttons are 28 tall and
        // the footer's divider is 10 above the footer.
        let bottom = y + 28.0;
        let divider = r.y + r.h - 46.0 - 10.0;
        assert!(
            bottom + 8.0 <= divider,
            "the Build page ends at {bottom}, too close to the footer at {divider}"
        );
    }

    #[test]
    fn toggles_change_and_persist_preferences() {
        let mut c = Canvas::new(1600, 950);
        let doc = Document::starter();
        let mut ui = Ui::new();
        ui.settings_page = SettingsPage::Editor;
        ui.open_settings();
        ui.frame(&mut c, &doc, Pointer::default(), None);
        let r = ui.dialog_rect(1600.0, 950.0).unwrap();
        // First toggle row ("Highlight current line") below the font size
        // and editor scale steppers.
        let body_x = r.x + 190.0 + 24.0;
        let body_w = r.w - 190.0 - 48.0;
        let y = r.y + 54.0 + 18.0 + 54.0 + 2.0 * ROW;
        assert!(ui.prefs.editor.highlight_line);
        assert!(!ui.take_prefs_dirty());
        click(&mut ui, &mut c, &doc, body_x + body_w - 20.0, y + 15.0);
        assert!(!ui.prefs.editor.highlight_line);
        assert!(ui.take_prefs_dirty(), "a changed setting is saved");
        // The code editor picks it up on the next frame.
        ui.frame(&mut c, &doc, Pointer::default(), None);
        assert!(!ui.code.settings.highlight_line);
        // Reset restores the defaults.
        click(&mut ui, &mut c, &doc, body_x + 40.0, r.y + r.h - 32.0);
        assert!(ui.prefs.editor.highlight_line);
        assert!(ui.take_prefs_dirty());
        // Switch pages from the navigation.
        click(&mut ui, &mut c, &doc, r.x + 60.0, r.y + 54.0 + 12.0 + 16.0);
        assert_eq!(ui.settings_page, SettingsPage::Appearance);
    }
}
