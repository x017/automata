//! Window presentation: the winit event loop that hosts the editor.
//!
//! Oberon renders its whole UI — panels plus the live engine viewport — into
//! an in-memory RGBA pixmap each frame (see [`crate::ui`]). This module owns
//! the OS window, turns OS input into the UI's per-frame [`Pointer`] and
//! keyboard calls, applies the [`Intent`]s the UI returns to the document
//! (with undo), runs play mode, and pushes the pixmap to the screen through
//! the engine's wgpu surface ([`Renderer::blit_rgba_to_surface`]). The entire
//! path is safe Rust.
//!
//! A real window needs a display server (X11/Wayland). In a headless
//! environment `run` reports the failure and returns; the editor's logic is
//! exercised headless via the `screenshot` example and the unit tests.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use automata_renderer::Renderer;

use crate::bridge::{self, BridgeMsg, ScriptBridge};
use crate::canvas::Canvas;
use crate::code;
use crate::doc::{CreateKind, Document};
use crate::fields;
use crate::lsp::{self, LspAction, LspClient};
use crate::play::{PlayInput, PlaySession};
use crate::project::{self, Prefs};
use crate::render::ViewportRenderer;
use crate::ui::{CursorHint, DialogKind, EditKey, Intent, LogLevel, PlayState, Pointer, Ui};
use crate::view::{GizmoMode, ViewCamera};

/// The editor window's initial logical size.
pub const WINDOW_SIZE: (u32, u32) = (1600, 950);

/// Two presses closer than this (and a few pixels apart) are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Runs the editor window. Blocks until the window is closed.
///
/// `scene`, when given, is the RON scene file to open (and save back to).
/// Returns `Ok(())` on a clean exit, or an error string describing why the
/// window could not be presented (no display server, no GPU, …).
pub fn run(scene: Option<&std::path::Path>) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;

    let mut app = EditorApp::new(scene)?;
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop run: {e}"))?;

    if let Some(err) = app.fatal_error {
        return Err(err);
    }
    Ok(())
}

/// Mouse and keyboard state accumulated between frames.
#[derive(Debug, Default)]
struct Input {
    /// Cursor position in physical pixels.
    cursor: (f32, f32),
    left: bool,
    right: bool,
    middle: bool,
    /// A left press happened since the last frame.
    pressed: bool,
    /// A right press happened since the last frame.
    right_pressed: bool,
    /// The middle button was pressed since the last frame.
    middle_pressed: bool,
    /// That press was a double click.
    double_click: bool,
    last_press: Option<(Instant, (f32, f32))>,
    scroll: f32,
    modifiers: ModifiersState,
    /// Keys currently held (drives the player in play mode).
    held: HashSet<KeyCode>,
}

impl Input {
    /// The command modifier: Ctrl, or ⌘ on macOS. On Linux and Windows the
    /// Super / Windows key belongs to the window manager (Super+D, Super+1,
    /// …), so it must never trigger editor shortcuts.
    fn ctrl(&self) -> bool {
        self.modifiers.control_key() || (cfg!(target_os = "macos") && self.modifiers.super_key())
    }

    /// Forgets every held key, button and modifier (focus changes).
    fn release_all(&mut self) {
        self.left = false;
        self.right = false;
        self.middle = false;
        self.modifiers = ModifiersState::empty();
        self.held.clear();
    }

    fn held_any(&self, keys: &[KeyCode]) -> bool {
        keys.iter().any(|k| self.held.contains(k))
    }

    /// The player controls from the held keys (WASD / arrows, Space, Shift).
    fn play_input(&self, camera_yaw: f32) -> PlayInput {
        let axis = |pos: &[KeyCode], neg: &[KeyCode]| {
            f32::from(u8::from(self.held_any(pos))) - f32::from(u8::from(self.held_any(neg)))
        };
        PlayInput {
            forward: axis(
                &[KeyCode::KeyW, KeyCode::ArrowUp],
                &[KeyCode::KeyS, KeyCode::ArrowDown],
            ),
            right: axis(
                &[KeyCode::KeyD, KeyCode::ArrowRight],
                &[KeyCode::KeyA, KeyCode::ArrowLeft],
            ),
            jump: self.held.contains(&KeyCode::Space),
            sprint: self.held_any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            camera_yaw,
        }
    }
}

/// Keys the player uses in play mode (never treated as editor shortcuts
/// while simulating).
fn is_game_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::KeyW
            | KeyCode::KeyA
            | KeyCode::KeyS
            | KeyCode::KeyD
            | KeyCode::ArrowUp
            | KeyCode::ArrowDown
            | KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::Space
            | KeyCode::ShiftLeft
            | KeyCode::ShiftRight
    )
}

/// The running editor: document, UI state, and the window/GPU once resumed.
struct EditorApp {
    doc: Document,
    ui: Ui,
    canvas: Option<Canvas>,
    viewport: ViewportRenderer,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    input: Input,
    /// The running simulation while in play mode.
    play: Option<PlaySession>,
    /// The Python script runner while playing a scene with scripts.
    bridge: Option<ScriptBridge>,
    /// The language server for the script editor.
    lsp: Option<LspClient>,
    /// Whether starting a language server failed (don't retry).
    lsp_failed: bool,
    /// Whether the "scripts are slow" warning was shown this session.
    bridge_slow_warned: bool,
    /// The editor camera to restore when play mode ends.
    edit_camera: Option<ViewCamera>,
    last_frame: Instant,
    last_title: String,
    last_cursor: CursorHint,
    fatal_error: Option<String>,
    /// Persistent preferences (recent files, layout, tool paths).
    /// When the scene was last autosaved (or loaded/saved).
    last_autosave: Instant,
    /// The running game export or template download.
    build: Option<crate::build::BuildJob>,
    /// The app should exit at the next opportunity.
    quit: bool,
}

impl EditorApp {
    fn new(scene: Option<&std::path::Path>) -> Result<Self, String> {
        let doc = match scene {
            Some(path) => Document::load(path)?,
            None => Document::starter(),
        };
        let mut ui = Ui::new();
        let mut prefs = Prefs::load();
        prefs.apply_panels(&mut ui.panels);
        if let Some(path) = &doc.path {
            prefs.touch_recent(path);
        }
        ui.recent = prefs.existing_recent();
        ui.prefs = prefs;
        ui.project_root = doc.base_dir();
        ui.log(
            LogLevel::Info,
            "Oberon ready. Click an object to select it; drag the gizmo to move it.",
        );
        ui.log(
            LogLevel::Info,
            "Edit values in the Inspector: drag a number to scrub it, or click to type.",
        );
        match &doc.path {
            Some(path) => ui.log(LogLevel::Info, format!("Opened {}", path.display())),
            None => ui.log(LogLevel::Info, "New scene (unsaved). Ctrl+S saves it."),
        }
        Ok(Self {
            doc,
            ui,
            canvas: None,
            viewport: ViewportRenderer::new(),
            window: None,
            renderer: None,
            input: Input::default(),
            play: None,
            bridge: None,
            lsp: None,
            lsp_failed: false,
            bridge_slow_warned: false,
            edit_camera: None,
            last_frame: Instant::now(),
            last_title: String::new(),
            last_cursor: CursorHint::Default,
            fatal_error: None,
            last_autosave: Instant::now(),
            build: None,
            quit: false,
        })
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    // ----- intents -------------------------------------------------------

    /// Applies a UI intent to the document (or play state).
    fn apply(&mut self, intent: Intent) {
        // Actions that discard the scene ask about unsaved changes first.
        let scripts_dirty = matches!(intent, Intent::Quit) && self.ui.code.dirty_count() > 0;
        if matches!(intent, Intent::New | Intent::Open(_) | Intent::Quit)
            && (self.doc.dirty() || scripts_dirty)
        {
            let scene = self.doc.dirty();
            self.ui.ask_unsaved(intent, scene);
            return;
        }
        let intent = match intent {
            Intent::Force(inner) => *inner,
            other => other,
        };
        if matches!(intent, Intent::New | Intent::Open(_)) && self.play.is_some() {
            self.stop();
        }
        if intent.edits_document()
            && self.ui.play != PlayState::Editing
            && !matches!(intent, Intent::New | Intent::Open(_))
        {
            return; // the scene is read-only while simulating
        }
        let doc = &mut self.doc;
        match intent {
            Intent::Select(id) => {
                doc.select(id);
                if let Some(play) = &mut self.play {
                    play.doc.select(id);
                }
            }
            Intent::Create(kind) => {
                let id = if kind == CreateKind::Camera {
                    doc.create_camera_at(self.ui.camera.to_camera3d())
                } else {
                    doc.create(kind)
                };
                let name = doc.index_of(id).map(|i| doc.entities[i].name().to_string());
                self.ui.log(
                    LogLevel::Info,
                    format!("Created {}", name.unwrap_or_default()),
                );
            }
            Intent::Duplicate(id) => {
                if let Some(new_id) = doc.duplicate_entity(id) {
                    let name = doc
                        .index_of(new_id)
                        .map(|i| doc.entities[i].name().to_string());
                    self.ui.log(
                        LogLevel::Info,
                        format!("Duplicated as {}", name.unwrap_or_default()),
                    );
                }
            }
            Intent::Delete(id) => {
                let name = doc.index_of(id).map(|i| doc.entities[i].name().to_string());
                if doc.delete_entity(id) {
                    self.ui.log(
                        LogLevel::Info,
                        format!("Deleted {}", name.unwrap_or_default()),
                    );
                }
            }
            Intent::AddComponent(id, kind) => {
                if doc.add_component(id, kind) {
                    self.ui
                        .log(LogLevel::Info, format!("Added {}", kind.label()));
                }
            }
            Intent::RemoveComponent(id, kind) => {
                if doc.remove_component(id, kind) {
                    self.ui
                        .log(LogLevel::Info, format!("Removed {}", kind.label()));
                }
            }
            Intent::SetField {
                id,
                field,
                value,
                gesture,
            } => {
                doc.modify(id, gesture, |d| {
                    fields::set(d, field, value);
                });
            }
            Intent::SetTransform { id, transform } => {
                doc.modify(id, true, |d| d.transform = Some(transform));
            }
            Intent::Rename { id, name } => {
                doc.rename_entity(id, &name);
            }
            Intent::Toggle { id, flag } => {
                doc.modify(id, false, |d| {
                    fields::toggle(d, flag);
                });
            }
            Intent::Cycle { id, choice } => {
                doc.modify(id, false, |d| {
                    fields::cycle(d, choice);
                });
            }
            Intent::ResetTransform(id) => {
                doc.reset_transform(id);
            }
            Intent::EndGesture => doc.end_gesture(),
            Intent::NudgeSelected(delta) => {
                if let Some(i) = doc.selected_index() {
                    let id = doc.entities[i].id;
                    doc.nudge_entity(id, delta);
                }
            }
            Intent::Undo => {
                if !doc.undo() {
                    self.ui.log(LogLevel::Warn, "Nothing to undo");
                }
            }
            Intent::Redo => {
                if !doc.redo() {
                    self.ui.log(LogLevel::Warn, "Nothing to redo");
                }
            }
            Intent::Save => self.save(),
            Intent::New => {
                self.doc = Document::starter();
                self.viewport.invalidate();
                self.ui.log(LogLevel::Info, "New scene");
            }
            Intent::Play => {
                if self.ui.prefs.editor.save_before_play && self.ui.code.dirty_count() > 0 {
                    self.ui.code.save_all();
                }
                self.play();
            }
            Intent::RestartLanguageServer => {
                if let Some(mut client) = self.lsp.take() {
                    client.shutdown();
                }
                self.lsp_failed = false;
                self.ui.code.server = None;
                self.ui
                    .log(LogLevel::Info, "Restarting the language server…");
            }
            Intent::Pause => match self.ui.play {
                PlayState::Playing => self.set_paused(true),
                PlayState::Paused => self.set_paused(false),
                PlayState::Editing => {}
            },
            Intent::Stop => self.stop(),
            Intent::Build(target) => self.start_build(target),
            Intent::DownloadTemplates(target) => self.download_templates(target),
            Intent::OpenBuildFolder => {
                let cfg = crate::build::config_for(&self.ui.project_root, self.doc.path.as_deref());
                let dir = self.ui.project_root.join(&cfg.output);
                let dir = if dir.is_dir() {
                    dir
                } else {
                    self.ui.project_root.clone()
                };
                if let Err(e) = crate::build::reveal(&dir) {
                    self.ui.log(LogLevel::Error, e);
                }
            }
            other => self.apply_more(other),
        }
    }

    /// Intents for files, prefabs, attributes, scripts, and clipboard.
    #[allow(clippy::too_many_lines)]
    fn apply_more(&mut self, intent: Intent) {
        let doc = &mut self.doc;
        match intent {
            Intent::SetColor {
                id,
                target,
                rgba,
                gesture,
            } => {
                doc.modify(id, gesture, |d| {
                    fields::set_color(d, target, rgba);
                });
            }
            Intent::SetChoice { id, choice, label } => {
                doc.modify(id, false, |d| {
                    fields::set_choice(d, choice, label);
                });
            }
            Intent::ResetComponent(id, kind) => {
                if doc.reset_component(id, kind) {
                    self.ui
                        .log(LogLevel::Info, format!("Reset {}", kind.label()));
                }
            }
            Intent::PasteComponent { id, kind, source } => {
                if doc.paste_component(id, kind, &source) {
                    self.ui
                        .log(LogLevel::Info, format!("Pasted {} values", kind.label()));
                }
            }
            Intent::PasteEntities(entities) => {
                let ids = doc.paste_entities(&entities, [0.5, 0.0, 0.5]);
                self.ui
                    .log(LogLevel::Info, format!("Pasted {} entity(s)", ids.len()));
            }
            Intent::SetAttribute {
                id,
                key,
                value,
                gesture,
            } => {
                doc.modify(id, gesture, |d| {
                    d.attributes.insert(key, value);
                });
            }
            Intent::RemoveAttribute { id, key } => {
                doc.modify(id, false, |d| {
                    d.attributes.remove(&key);
                });
            }
            Intent::RenameAttribute { id, from, to } => {
                let mut clash = false;
                doc.modify(id, false, |d| {
                    if d.attributes.contains_key(&to) {
                        clash = true;
                    } else if let Some(v) = d.attributes.remove(&from) {
                        d.attributes.insert(to.clone(), v);
                    }
                });
                if clash {
                    self.ui.log(
                        LogLevel::Warn,
                        format!("An attribute named {to:?} already exists"),
                    );
                }
            }
            Intent::Open(path) => self.open_scene(&path),
            Intent::SaveAs(path) => {
                self.doc.path = Some(path.clone());
                self.save();
                self.ui.files_changed(Some(&path));
            }
            Intent::SavePrefab { id, path } => {
                self.save_prefab(id, &path);
                self.ui.files_changed(Some(&path));
            }
            Intent::InstantiatePrefab(path) => self.instantiate_prefab(&path),
            Intent::OpenScript(path) => self.open_script(&path, None),
            Intent::OpenScriptAt { path, line } => self.open_script(&path, Some(line)),
            Intent::NewScript(path) => {
                if self.ensure_script(&path) {
                    self.ui.files_changed(Some(&path));
                    self.open_script(&path, None);
                }
            }
            Intent::AttachScript { id, path } => {
                if self.ensure_script(&path) {
                    self.ui.files_changed(None);
                    let rel = self.doc.relative(&path);
                    self.doc.modify(id, false, |d| {
                        d.script = Some(automata_core::Script { path: rel.clone() });
                    });
                    self.ui.log(LogLevel::Info, format!("Attached {rel}"));
                    self.open_script(&path, None);
                }
            }
            Intent::DetachScript(id) => {
                if self.doc.modify(id, false, |d| d.script = None) {
                    self.ui.log(LogLevel::Info, "Script detached");
                }
            }
            Intent::NewFolder(path) => match std::fs::create_dir_all(&path) {
                Ok(()) => {
                    self.ui.files_changed(Some(&path));
                    self.ui
                        .log(LogLevel::Info, format!("Created folder {}", path.display()));
                }
                Err(e) => self
                    .ui
                    .log(LogLevel::Error, format!("New folder failed: {e}")),
            },
            Intent::Quit => self.quit = true,
            Intent::SetEnvironment(id, env) => {
                doc.modify(id, false, |d| d.environment = Some(env));
            }
            Intent::SpawnTest(scene) => {
                let n = doc.spawn_test(scene);
                self.ui.log(
                    LogLevel::Info,
                    format!("Added {n} test objects ({})", scene.label()),
                );
            }
            Intent::ClearTests => {
                let n = doc.clear_tests();
                self.ui
                    .log(LogLevel::Info, format!("Removed {n} test objects"));
            }
            Intent::AlignCameraToView(id) => {
                let view = self.ui.camera.to_camera3d();
                doc.modify(id, false, |d| {
                    if let Some(cam) = &mut d.camera_3d {
                        cam.position = view.position;
                        cam.target = view.target;
                    }
                    if let Some(t) = &mut d.transform {
                        t.position = view.position.to_array();
                    }
                });
            }
            _ => {}
        }
    }

    fn open_scene(&mut self, path: &std::path::Path) {
        match Document::load(path) {
            Ok(doc) => {
                self.doc = doc;
                self.viewport.invalidate();
                self.ui.prefs.touch_recent(path);
                self.ui.recent = self.ui.prefs.existing_recent();
                self.ui.project_root = self.doc.base_dir();
                self.last_autosave = Instant::now();
                self.save_prefs();
                self.ui
                    .log(LogLevel::Info, format!("Opened {}", path.display()));
            }
            Err(e) => self.ui.log(
                LogLevel::Error,
                format!("Open {} failed: {e}", path.display()),
            ),
        }
    }

    fn save_prefab(&mut self, id: u64, path: &std::path::Path) {
        let Some(prefab) = self.doc.prefab_of(id) else {
            return;
        };
        let result = prefab.to_ron().map_err(|e| e.to_string()).and_then(|text| {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, text).map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => self
                .ui
                .log(LogLevel::Info, format!("Saved prefab {}", path.display())),
            Err(e) => self
                .ui
                .log(LogLevel::Error, format!("Save prefab failed: {e}")),
        }
    }

    fn instantiate_prefab(&mut self, path: &std::path::Path) {
        let loaded = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| automata_core::SceneFile::from_ron(&t).map_err(|e| e.to_string()));
        match loaded {
            Ok(prefab) => {
                // Drop it where the scene camera is looking, on the ground.
                let t = self.ui.camera.target;
                let ground = prefab
                    .entities
                    .first()
                    .and_then(|e| e.transform)
                    .map_or(0.0, |tr| tr.position[1]);
                let ids = self.doc.instantiate(&prefab, [t[0], ground, t[2]]);
                self.ui.log(
                    LogLevel::Info,
                    format!("Instantiated {} ({} entity(s))", path.display(), ids.len()),
                );
            }
            Err(e) => self.ui.log(
                LogLevel::Error,
                format!("Instantiate {} failed: {e}", path.display()),
            ),
        }
    }

    /// Creates `path` from the script template if it does not exist.
    fn ensure_script(&mut self, path: &std::path::Path) -> bool {
        if path.exists() {
            return true;
        }
        let result = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, project::script_template(path)));
        match result {
            Ok(()) => {
                self.ui
                    .log(LogLevel::Info, format!("Created {}", path.display()));
                true
            }
            Err(e) => {
                self.ui.log(
                    LogLevel::Error,
                    format!("Create {} failed: {e}", path.display()),
                );
                false
            }
        }
    }

    /// Opens a script in the code editor.
    fn open_script(&mut self, path: &std::path::Path, line: Option<u32>) {
        self.ui.open_code(path, line);
    }

    /// Changes the interface zoom by `delta` (0 resets to 100%).
    fn zoom(&mut self, delta: f32) {
        let p = &mut self.ui.prefs;
        p.ui_scale = if delta == 0.0 {
            1.0
        } else {
            ((p.ui_scale + delta) * 10.0).round().clamp(6.0, 25.0) / 10.0
        };
        let pct = (p.ui_scale * 100.0).round();
        self.ui
            .log(LogLevel::Info, format!("Interface scale {pct}%"));
        self.save_prefs();
    }

    fn save_prefs(&mut self) {
        self.ui.prefs.capture_panels(&self.ui.panels);
        if let Err(e) = self.ui.prefs.save() {
            tracing::warn!(error = %e, "could not save preferences");
        }
    }

    /// Writes the autosave file when the scene has unsaved changes.
    fn autosave(&mut self) {
        let secs = self.ui.prefs.autosave_secs;
        if secs == 0 || self.last_autosave.elapsed() < Duration::from_secs(secs) {
            return;
        }
        self.last_autosave = Instant::now();
        if !self.doc.dirty() || self.play.is_some() {
            return;
        }
        let Ok(text) = self.doc.to_ron() else {
            return;
        };
        match project::write_autosave(&self.ui.project_root, &text) {
            Ok(path) => tracing::info!(path = %path.display(), "autosaved"),
            Err(e) => self.ui.log(LogLevel::Warn, format!("Autosave failed: {e}")),
        }
    }

    fn save(&mut self) {
        let path = self.doc.path.clone().unwrap_or_else(default_save_path);
        match self.doc.save(Some(path.as_path())) {
            Ok(path) => {
                self.ui
                    .log(LogLevel::Info, format!("Saved {}", path.display()));
                self.ui.prefs.touch_recent(&path);
                self.ui.recent = self.ui.prefs.existing_recent();
                self.ui.project_root = self.doc.base_dir();
                self.last_autosave = Instant::now();
                self.save_prefs();
            }
            Err(err) => {
                tracing::error!(error = %err, "save failed");
                self.ui.log(LogLevel::Error, format!("Save failed: {err}"));
            }
        }
    }

    // ----- export ----------------------------------------------------------

    /// Exports the game for `target` (or the platforms in Build Settings).
    fn start_build(&mut self, target: Option<crate::build::Target>) {
        use crate::build::{BuildJob, Target};
        if let Some(job) = &self.build {
            self.ui
                .log(LogLevel::Warn, format!("Busy: {}", job.label()));
            return;
        }
        // Exports read the project from disk: save what's open first.
        if self.doc.path.is_none() {
            self.ui.log(
                LogLevel::Error,
                "Save the scene into a project folder before building (Ctrl+S).",
            );
            return;
        }
        if self.doc.dirty() {
            self.save();
        }
        if self.ui.code.dirty_count() > 0 {
            self.ui.code.save_all();
        }
        let root = self.ui.project_root.clone();
        let cfg = crate::build::config_for(&root, self.doc.path.as_deref());
        let targets = match target {
            Some(t) => vec![t],
            None if cfg.targets.is_empty() => Target::host().map_or_else(Vec::new, |t| vec![t]),
            None => cfg.targets.clone(),
        };
        if targets.is_empty() {
            self.ui
                .log(LogLevel::Error, "No platforms ticked in Build Settings.");
            return;
        }
        let job = BuildJob::export(root, cfg, targets);
        self.ui.log(LogLevel::Info, job.label());
        self.ui.build_status = Some(job.label());
        self.build = Some(job);
    }

    /// Downloads export templates.
    fn download_templates(&mut self, target: Option<crate::build::Target>) {
        use crate::build::{BuildJob, Target};
        if let Some(job) = &self.build {
            self.ui
                .log(LogLevel::Warn, format!("Busy: {}", job.label()));
            return;
        }
        let targets = target.map_or_else(|| Target::ALL.to_vec(), |t| vec![t]);
        let job = BuildJob::download(targets);
        self.ui.build_status = Some(job.label());
        self.build = Some(job);
    }

    /// Moves the running job's progress into the console.
    fn poll_build(&mut self) {
        use crate::build::{BuildEvent, JobKind, Level};
        let Some(job) = &mut self.build else {
            return;
        };
        let (events, done) = job.poll();
        for e in events {
            match e {
                BuildEvent::Log(level, text) => {
                    let level = match level {
                        Level::Info => LogLevel::Info,
                        Level::Warning => LogLevel::Warn,
                        Level::Error => LogLevel::Error,
                    };
                    self.ui.log(level, text);
                }
                BuildEvent::Finished(target, Ok(path)) => {
                    let what = if job.kind == JobKind::Export {
                        "Built"
                    } else {
                        "Installed the template:"
                    };
                    self.ui.log(
                        LogLevel::Info,
                        format!("{} {what} {}", target.label(), path.display()),
                    );
                }
                BuildEvent::Finished(target, Err(e)) => {
                    self.ui
                        .log(LogLevel::Error, format!("{} failed: {e}", target.label()));
                }
            }
        }
        if done {
            let secs = job.started.elapsed().as_secs_f32();
            let (ok, failed) = (job.succeeded.len(), job.failed.len());
            let summary = match job.kind {
                JobKind::Export if failed == 0 => {
                    format!("Build finished in {secs:.1} s → {}", job.output.display())
                }
                JobKind::Export => format!("Build finished with errors ({ok} ok, {failed} failed)"),
                JobKind::Download if failed == 0 => "Export templates installed".to_string(),
                JobKind::Download => {
                    format!("Template download failed ({failed} of {})", ok + failed)
                }
            };
            let level = if failed == 0 {
                LogLevel::Info
            } else {
                LogLevel::Error
            };
            self.ui.log(level, summary.clone());
            self.ui.build_last = Some((failed == 0, summary));
            self.ui.build_status = None;
            self.build = None;
            if failed > 0 {
                self.ui.show_console();
            }
        }
    }

    fn play(&mut self) {
        match self.ui.play {
            PlayState::Editing => {
                self.doc.end_gesture();
                let session = PlaySession::start(&self.doc);
                let dynamic = session.dynamic_count();
                let characters = session.character_count();
                if let Some(pos) = session.player_position() {
                    // Third-person follow camera behind the player.
                    self.edit_camera = Some(self.ui.camera);
                    self.ui.camera.target = (pos + glam::Vec3::Y * 0.6).to_array();
                    self.ui.camera.distance = 9.0;
                    self.ui.camera.pitch = 0.4;
                }
                let scripts = session.script_count();
                self.play = Some(session);
                self.ui.play = PlayState::Playing;
                self.input.held.clear();
                if scripts > 0 {
                    self.start_bridge(scripts);
                }
                if characters > 0 {
                    self.ui.log(
                        LogLevel::Info,
                        "Playing. WASD / arrows move, Space jumps, Shift sprints, \
                         drag or right-drag turns the camera. Ctrl+P or Stop to exit.",
                    );
                } else {
                    self.ui
                        .log(LogLevel::Info, "Playing. Ctrl+P or Stop to exit.");
                }
                if dynamic == 0 && characters == 0 {
                    self.ui.log(
                        LogLevel::Warn,
                        "Nothing moves in this scene: add a Player (+ Create → Player) or a \
                         Physics Cube, or give objects a Rigidbody + Collider.",
                    );
                } else if characters == 0 {
                    self.ui.log(
                        LogLevel::Warn,
                        "No player to control: + Create → Player (or add a Character \
                         Controller component) to walk around.",
                    );
                }
            }
            PlayState::Paused => self.set_paused(false),
            PlayState::Playing => {}
        }
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some(play) = &mut self.play {
            play.paused = paused;
            self.ui.play = if paused {
                PlayState::Paused
            } else {
                PlayState::Playing
            };
        }
    }

    /// Starts the Python script runner for the session's scripts.
    fn start_bridge(&mut self, scripts: usize) {
        let python = self
            .ui
            .prefs
            .python
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| bridge::default_python().to_string());
        let runtime = self.ui.project_root.join(".oberon");
        let cwd = self.doc.base_dir();
        self.bridge_slow_warned = false;
        match ScriptBridge::start(&python, &runtime, &cwd) {
            Ok(b) => {
                self.bridge = Some(b);
                self.ui.log(
                    LogLevel::Info,
                    format!(
                        "Running {scripts} script{} with {python}…",
                        if scripts == 1 { "" } else { "s" }
                    ),
                );
            }
            Err(err) => self.ui.log(
                LogLevel::Error,
                format!(
                    "Could not start Python ({python}): {err}. Install Python 3 or set \
                     `python` in ~/.config/oberon/prefs.ron. Scripts are disabled."
                ),
            ),
        }
    }

    /// Shows one runner message in the Console (and applies frame writes).
    fn handle_bridge_msg(&mut self, msg: BridgeMsg) {
        match msg {
            BridgeMsg::Ready { python } => {
                self.ui.log(
                    LogLevel::Info,
                    format!("Scripts running (Python {python})."),
                );
            }
            BridgeMsg::Log { level, msg } => {
                let level = match level.as_str() {
                    "warn" => LogLevel::Warn,
                    "error" => LogLevel::Error,
                    _ => LogLevel::Info,
                };
                self.ui.log(level, msg);
            }
            BridgeMsg::Error {
                msg,
                file,
                line,
                traceback,
            } => {
                self.ui.log(LogLevel::Error, msg);
                for l in traceback.lines() {
                    self.ui.log(LogLevel::Error, l.to_string());
                }
                if line > 0 {
                    self.ui.log(
                        LogLevel::Error,
                        format!(
                            "  File \"{}\", line {line}  (click to open; fix and save to reload)",
                            file.display()
                        ),
                    );
                }
            }
            BridgeMsg::Reloaded(_) | BridgeMsg::Bye => {}
            BridgeMsg::Frame(writes) => {
                if let Some(play) = &mut self.play {
                    play.apply_script_writes(&writes);
                }
            }
            BridgeMsg::Exited(status) => {
                self.ui.log(
                    LogLevel::Error,
                    format!("The script runner exited ({status}). Scripts stopped."),
                );
            }
        }
    }

    /// Exchanges one frame with the script runner: applies its replies,
    /// then sends the current state (when the previous frame is answered).
    fn tick_scripts(&mut self, dt: f32) {
        let Some(bridge) = &mut self.bridge else {
            return;
        };
        let msgs = bridge.poll();
        let slow = bridge
            .waiting_for()
            .is_some_and(|w| w > Duration::from_secs(2));
        let gone = bridge.exited();
        for msg in msgs {
            self.handle_bridge_msg(msg);
        }
        if gone {
            self.bridge = None;
            return;
        }
        if slow && !self.bridge_slow_warned {
            self.bridge_slow_warned = true;
            self.ui.log(
                LogLevel::Warn,
                "A script has not finished its frame for 2 s (an endless loop?). Stop to \
                 restart the scripts.",
            );
        }
        let (Some(play), Some(bridge)) = (&self.play, &mut self.bridge) else {
            return;
        };
        if play.paused || bridge.busy() {
            return;
        }
        let doc = &self.doc;
        let states = play.script_states(|rel| doc.resolve(rel));
        let mut keys: Vec<&str> = self
            .input
            .held
            .iter()
            .filter_map(|&k| bridge::key_name(k))
            .collect();
        keys.sort_unstable();
        bridge.send_frame(dt, play.elapsed, &keys, &states);
    }

    fn stop(&mut self) {
        if let Some(bridge) = self.bridge.take() {
            for msg in bridge.stop() {
                self.handle_bridge_msg(msg);
            }
        }
        if let Some(play) = self.play.take() {
            self.ui.log(
                LogLevel::Info,
                format!("Stopped after {:.1}s. Scene restored.", play.elapsed),
            );
        }
        self.ui.play = PlayState::Editing;
        if let Some(cam) = self.edit_camera.take() {
            self.ui.camera = cam;
        }
        self.input.held.clear();
    }

    // ----- keyboard ------------------------------------------------------

    fn keyboard(&mut self, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        if event.state == ElementState::Released {
            self.input.held.remove(&code);
        } else if !cfg!(target_os = "macos") && self.input.modifiers.super_key() {
            // Super+key is a window-manager shortcut (tile, switch
            // workspace, launch): ignore it entirely.
            return;
        } else {
            self.input.held.insert(code);
        }
        // While simulating, movement keys drive the player, not the editor;
        // with scripts running, every key scripts can read does (Ctrl
        // shortcuts like Ctrl+P still reach the editor).
        let script_key = self.bridge.is_some()
            && bridge::key_name(code).is_some()
            && !matches!(code, KeyCode::Escape | KeyCode::F1 | KeyCode::Delete);
        if self.play.is_some()
            && (is_game_key(code) || script_key)
            && !self.input.ctrl()
            && !self.ui.is_editing_text()
            && !self.ui.code_has_focus()
        {
            return;
        }
        if event.state == ElementState::Released {
            // Releasing an arrow key ends the nudge gesture (one undo step
            // per key hold).
            if is_nudge_key(code) {
                self.doc.end_gesture();
            }
            return;
        }
        // Escape closes the topmost popup / picker / dialog first.
        if code == KeyCode::Escape && !self.ui.is_editing_text() && self.ui.close_overlay() {
            return;
        }
        // The command palette owns the keyboard while open.
        if self.ui.palette_open() {
            use crate::ui::palette::PaletteKey;
            let key = match code {
                KeyCode::ArrowUp => Some(PaletteKey::Up),
                KeyCode::ArrowDown => Some(PaletteKey::Down),
                KeyCode::Enter | KeyCode::NumpadEnter => Some(PaletteKey::Enter),
                KeyCode::Backspace => Some(PaletteKey::Backspace),
                KeyCode::Escape => Some(PaletteKey::Escape),
                _ => None,
            };
            match key {
                Some(k) => {
                    let doc = self.play.as_ref().map_or(&self.doc, |p| &p.doc);
                    self.ui.palette_key(k, doc);
                }
                None if !self.input.ctrl() => {
                    if let Some(text) = &event.text {
                        self.ui.palette_text(text);
                    }
                }
                None => {}
            }
            return;
        }
        // A modal dialog without a focused field only listens for Enter.
        if self.ui.dialog_open() && !self.ui.is_editing_text() {
            if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
                self.ui.close_dialog();
            }
            return;
        }

        // A focused text field takes the keyboard.
        if self.ui.is_editing_text() {
            let key = match code {
                KeyCode::Backspace => Some(EditKey::Backspace),
                KeyCode::Delete => Some(EditKey::Delete),
                KeyCode::Enter | KeyCode::NumpadEnter => Some(EditKey::Enter),
                KeyCode::Tab => Some(EditKey::Tab),
                KeyCode::Escape => Some(EditKey::Escape),
                _ => None,
            };
            match key {
                Some(key) => {
                    if let Some(intent) = self.ui.edit_key(key) {
                        self.apply(intent);
                    }
                }
                None if !self.input.ctrl() => {
                    if let Some(text) = &event.text {
                        self.ui.type_text(text);
                    }
                }
                None => {}
            }
            return;
        }

        let ctrl = self.input.ctrl();
        let shift = self.input.modifiers.shift_key();

        // The script editor takes the keyboard when focused; keys it doesn't
        // use (Ctrl+P, Ctrl+Q, F1, …) fall through to the global shortcuts.
        let alt = self.input.modifiers.alt_key();
        let mods = code::Mods { ctrl, shift, alt };
        // The second key of a Ctrl+K chord arrives as a key, not as text.
        let chord = self.ui.code.chord_pending();
        let editor_key =
            code_key(code, mods).or_else(|| chord.then(|| code_char(code).map(code::Key::Char))?);
        if self.ui.code_has_focus() {
            if let Some(key) = editor_key {
                if self.ui.code_key(key, mods) {
                    return;
                }
            } else if !ctrl && !alt {
                if let Some(text) = &event.text {
                    self.ui.code_text(text);
                }
                if code != KeyCode::F1 {
                    return;
                }
            }
            if !ctrl && code != KeyCode::F1 {
                return;
            }
        }

        // Tab navigation works from the Scene tab and unfocused editors too.
        if ctrl || alt || chord {
            if let Some(key) = editor_key {
                if self.ui.code_tab_key(key, mods) {
                    return;
                }
            }
        }

        let selected = self.doc.selected_index().map(|i| self.doc.entities[i].id);

        // Nudging repeats while held.
        if !ctrl {
            if let Some(delta) = self.nudge_delta(code) {
                self.apply(Intent::NudgeSelected(delta));
                return;
            }
        }
        if event.repeat {
            return;
        }
        let intent = match (code, ctrl) {
            (KeyCode::KeyP, true) if shift => {
                let doc = self.play.as_ref().map_or(&self.doc, |p| &p.doc);
                self.ui.open_palette(doc);
                None
            }
            (KeyCode::KeyB, true) if shift => Some(Intent::Build(None)),
            (KeyCode::KeyE, true) if !shift => {
                self.ui.open_quick_open();
                None
            }
            (KeyCode::Comma, true) => {
                self.ui.open_settings();
                None
            }
            (KeyCode::Equal | KeyCode::NumpadAdd, true) => {
                self.zoom(0.1);
                None
            }
            (KeyCode::Minus | KeyCode::NumpadSubtract, true) => {
                self.zoom(-0.1);
                None
            }
            (KeyCode::Digit0 | KeyCode::Numpad0, true) => {
                self.zoom(0.0);
                None
            }
            (KeyCode::KeyS, true) if shift => {
                self.ui.open_dialog_kind(DialogKind::SaveSceneAs, &self.doc);
                None
            }
            (KeyCode::KeyO, true) => {
                self.ui.open_dialog_kind(DialogKind::OpenScene, &self.doc);
                None
            }
            (KeyCode::KeyQ, true) => Some(Intent::Quit),
            (KeyCode::KeyC, true) => {
                self.ui.copy_selected(&self.doc);
                None
            }
            (KeyCode::KeyV, true) => self.ui.paste_intent(),
            (KeyCode::F1, false) => {
                self.ui.open_dialog_kind(DialogKind::Shortcuts, &self.doc);
                None
            }
            (KeyCode::KeyZ, true) if shift => Some(Intent::Redo),
            (KeyCode::KeyZ, true) => Some(Intent::Undo),
            (KeyCode::KeyY, true) => Some(Intent::Redo),
            (KeyCode::KeyS, true) => Some(Intent::Save),
            (KeyCode::KeyN, true) => Some(Intent::New),
            (KeyCode::KeyD, true) => selected.map(Intent::Duplicate),
            (KeyCode::KeyP, true) => Some(if self.ui.play == PlayState::Editing {
                Intent::Play
            } else {
                Intent::Stop
            }),
            (KeyCode::Delete | KeyCode::Backspace, false) => selected.map(Intent::Delete),
            (KeyCode::Escape, false) => Some(Intent::Select(None)),
            (KeyCode::KeyF, false) => {
                self.focus_selected();
                None
            }
            (KeyCode::F2, false) => {
                if self.ui.play == PlayState::Editing {
                    self.ui.begin_rename(&self.doc);
                }
                None
            }
            (KeyCode::KeyW, false) => {
                self.ui.gizmo = GizmoMode::Move;
                None
            }
            (KeyCode::KeyE, false) => {
                self.ui.gizmo = GizmoMode::Rotate;
                None
            }
            (KeyCode::KeyR, false) => {
                self.ui.gizmo = GizmoMode::Scale;
                None
            }
            _ => None,
        };
        if let Some(intent) = intent {
            self.apply(intent);
        }
    }

    fn focus_selected(&mut self) {
        match &self.play {
            Some(play) => self.ui.focus_selected(&play.doc),
            None => self.ui.focus_selected(&self.doc),
        }
    }

    /// Arrow keys move the selection on the ground plane relative to the
    /// camera; PageUp/PageDown move it along world Y. Shift = fine steps.
    fn nudge_delta(&self, code: KeyCode) -> Option<[f32; 3]> {
        let step = if self.input.modifiers.shift_key() {
            0.05
        } else {
            0.25
        };
        let (sy, cy) = self.ui.camera.yaw.sin_cos();
        Some(match code {
            KeyCode::ArrowLeft => [-cy * step, 0.0, sy * step],
            KeyCode::ArrowRight => [cy * step, 0.0, -sy * step],
            KeyCode::ArrowUp => [-sy * step, 0.0, -cy * step],
            KeyCode::ArrowDown => [sy * step, 0.0, cy * step],
            KeyCode::PageUp => [0.0, step, 0.0],
            KeyCode::PageDown => [0.0, -step, 0.0],
            _ => return None,
        })
    }

    // ----- script editor ---------------------------------------------------

    /// Handles requests from the script editor. Saved scripts reach a running
    /// game through the bridge's hot reload; language features are served by
    /// the language server once one is running.
    fn handle_code_requests(&mut self) {
        let requests = self.ui.take_code_requests();
        // The language server starts with the first open script.
        if self.lsp.is_none() && !self.lsp_failed && !self.ui.code.tabs.is_empty() {
            let root = self.ui.project_root.clone();
            let dirs = lsp::tool_dirs(&root, self.ui.prefs.python.as_deref());
            match LspClient::start(self.ui.prefs.language_server.as_deref(), &root, &dirs) {
                Ok(client) => {
                    self.ui.code.server = Some(client.name.clone());
                    self.lsp = Some(client);
                }
                Err(e) => {
                    self.lsp_failed = true;
                    self.ui.log(
                        LogLevel::Info,
                        format!("Code intelligence: {e}. Using word completion."),
                    );
                }
            }
        }
        let Some(client) = &mut self.lsp else {
            for ev in requests {
                if let code::CodeEvent::Complete(..) = ev {
                    let items = self.ui.code.local_completions();
                    self.ui.code.show_completions(items);
                }
            }
            return;
        };
        client.sync(&self.ui.code);
        for ev in &requests {
            client.request(&self.ui.code, ev);
        }
        let actions = client.poll(&mut self.ui.code);
        let dead = client.is_dead();
        for action in actions {
            match action {
                LspAction::Log(text, error) => {
                    let level = if error {
                        LogLevel::Error
                    } else {
                        LogLevel::Info
                    };
                    self.ui.log(level, text);
                }
                LspAction::Goto(path, pos) => {
                    self.ui.open_code(&path, u32::try_from(pos.line + 1).ok());
                    if let Some(tab) = self.ui.code.current_mut() {
                        tab.buf.move_to(pos, false);
                        tab.reveal = true;
                    }
                }
            }
        }
        if dead {
            self.lsp = None;
            self.lsp_failed = true;
            self.ui.code.server = None;
        }
    }

    // ----- frame ---------------------------------------------------------

    /// Steps play mode, renders one frame, applies the UI's intents, and
    /// presents.
    fn render_frame(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let (w, h) = (size.width, size.height);
        if w == 0 || h == 0 || self.renderer.is_none() {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.tick_scripts(dt);
        self.poll_build();
        let ms = dt * 1000.0;
        let prev = self.ui.stats.frame_ms;
        self.ui.stats.frame_ms = if prev <= 0.0 || ms > 250.0 {
            ms.min(250.0)
        } else {
            prev + (ms - prev) * 0.08
        };
        if let Some(r) = &mut self.renderer {
            if r.quality() != self.ui.prefs.graphics_quality {
                r.set_quality(self.ui.prefs.graphics_quality);
            }
        }
        if let Some(play) = &mut self.play {
            // With a scene camera, movement is relative to it and scripts
            // drive the view; otherwise the editor camera follows the player.
            let scene_cam = play.scene_camera();
            let yaw = scene_cam.map_or(self.ui.camera.yaw, |c| ViewCamera::from_camera3d(&c).yaw);
            play.input = self.input.play_input(yaw);
            play.update(dt);
            // The camera follows the player (orbit/zoom stay under the
            // user's control).
            if let Some(pos) = play.player_position().filter(|_| scene_cam.is_none()) {
                let goal = pos + glam::Vec3::Y * 0.6;
                let current = glam::Vec3::from(self.ui.camera.target);
                let k = 1.0 - (-dt * 10.0).exp();
                self.ui.camera.target = current.lerp(goal, k).to_array();
            }
        }

        let scale = window.scale_factor() as f32 * self.ui.prefs.ui_scale.clamp(0.5, 3.0);
        let pointer = self.pointer(scale);

        let intents = {
            let Self {
                doc,
                ui,
                canvas,
                viewport,
                renderer,
                play,
                ..
            } = self;
            let canvas = canvas.get_or_insert_with(|| Canvas::new(w, h));
            if canvas.size() != (w, h) {
                canvas.resize(w, h);
            }
            canvas.set_scale(scale);

            // The scene view's size in physical pixels: the GPU renders at
            // full resolution and the UI composites it 1:1.
            let (lw, lh) = canvas.logical_size();
            let view = ui.layout(lw, lh).viewport;
            let s = canvas.scale();
            let vw = (view.w * s).round().max(1.0) as u32;
            let vh = (view.h * s).round().max(1.0) as u32;

            let cam = ui.camera;
            match play {
                // A script tab covers the scene view: skip the 3D render.
                Some(session) if !ui.scene_visible() => {
                    ui.frame(canvas, &session.doc, pointer, None)
                }
                None if !ui.scene_visible() => ui.frame(canvas, doc, pointer, None),
                Some(session) => {
                    let (frame, fw, fh) =
                        viewport.render_play(renderer.as_mut(), session, cam, vw, vh);
                    ui.frame(canvas, &session.doc, pointer, Some((frame, fw, fh)))
                }
                None => {
                    // The selected camera's preview first (same world).
                    let preview = ui.camera_preview_request(doc, lw, lh, s);
                    match (preview, renderer.as_mut()) {
                        (Some((camera, pw, ph)), Some(r)) => {
                            viewport.sync(r, doc);
                            let img = viewport.render_camera(Some(r), camera, pw, ph);
                            ui.set_camera_preview(img);
                        }
                        _ => ui.set_camera_preview(None),
                    }
                    let (frame, fw, fh) = viewport.render(renderer.as_mut(), doc, cam, vw, vh);
                    ui.frame(canvas, doc, pointer, Some((frame, fw, fh)))
                }
            }
        };

        self.ui.stats.meshes = self.viewport.mesh_count();
        // Per-frame transient input is consumed.
        self.input.pressed = false;
        self.input.right_pressed = false;
        self.input.middle_pressed = false;
        self.input.double_click = false;
        self.input.scroll = 0.0;

        // Intents take effect next frame (this one is already drawn).
        for intent in intents {
            self.apply(intent);
        }
        self.handle_code_requests();
        if self.ui.take_prefs_dirty() {
            self.save_prefs();
        }

        if let (Some(renderer), Some(canvas)) = (&mut self.renderer, &self.canvas) {
            renderer.blit_rgba_to_surface(canvas.pixels(), canvas.size());
        }
        self.update_window_chrome(&window);
        self.autosave();
    }

    /// This frame's pointer in logical pixels.
    fn pointer(&self, scale: f32) -> Pointer {
        let i = &self.input;
        let s = scale.max(0.1);
        Pointer {
            x: i.cursor.0 / s,
            y: i.cursor.1 / s,
            down: i.left,
            pressed: i.pressed,
            double_click: i.double_click,
            right_down: i.right,
            right_pressed: i.right_pressed,
            middle_pressed: i.middle_pressed,
            middle_down: i.middle,
            scroll: i.scroll,
            shift: i.modifiers.shift_key(),
            ctrl: i.ctrl(),
            alt: i.modifiers.alt_key(),
        }
    }

    /// Keeps the window title and mouse cursor in sync with the editor.
    fn update_window_chrome(&mut self, window: &Window) {
        let name = self.doc.path.as_ref().map_or_else(
            || "Untitled".to_string(),
            |p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            },
        );
        let mode = match self.ui.play {
            PlayState::Editing => "",
            PlayState::Playing => " [Playing]",
            PlayState::Paused => " [Paused]",
        };
        let dirty = if self.doc.dirty() { "*" } else { "" };
        let title = format!("{name}{dirty}{mode} — Oberon");
        if title != self.last_title {
            window.set_title(&title);
            self.last_title = title;
        }
        let cursor = self.ui.cursor();
        if cursor != self.last_cursor {
            window.set_cursor(match cursor {
                CursorHint::Default => CursorIcon::Default,
                CursorHint::Pointer => CursorIcon::Pointer,
                CursorHint::Text => CursorIcon::Text,
                CursorHint::ResizeH => CursorIcon::EwResize,
                CursorHint::ResizeV => CursorIcon::NsResize,
                CursorHint::Grab => CursorIcon::Grab,
                CursorHint::Grabbing => CursorIcon::Grabbing,
                CursorHint::Move => CursorIcon::Move,
            });
            self.last_cursor = cursor;
        }
    }
}

fn is_nudge_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::ArrowUp
            | KeyCode::ArrowDown
            | KeyCode::PageUp
            | KeyCode::PageDown
    )
}

/// `scene.ron` in the working directory, or `scene-2.ron`, … if taken.
fn default_save_path() -> PathBuf {
    let first = PathBuf::from("scene.ron");
    if !first.exists() {
        return first;
    }
    (2..10_000)
        .map(|i| PathBuf::from(format!("scene-{i}.ron")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

impl ApplicationHandler for EditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Oberon — AUTOMATA Scene Editor")
            .with_inner_size(LogicalSize::new(
                f64::from(WINDOW_SIZE.0),
                f64::from(WINDOW_SIZE.1),
            ))
            .with_min_inner_size(LogicalSize::new(800.0, 500.0))
            .with_resizable(true);

        let result = (|| -> Result<(Arc<Window>, Renderer), String> {
            let window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .map_err(|e| format!("create window: {e}"))?,
            );
            let size = window.inner_size();
            let renderer = pollster::block_on(Renderer::new(
                window.clone(),
                (size.width.max(1), size.height.max(1)),
                true, // vsync
                self.ui.prefs.graphics_quality,
            ))
            .map_err(|e| format!("create renderer: {e}"))?;
            Ok((window, renderer))
        })();

        match result {
            Ok((window, renderer)) => {
                let size = window.inner_size();
                let mut canvas = Canvas::new(size.width.max(1), size.height.max(1));
                canvas.set_scale(window.scale_factor() as f32);
                self.canvas = Some(canvas);
                self.window = Some(window.clone());
                self.renderer = Some(renderer);
                self.last_frame = Instant::now();
                window.request_redraw();
            }
            Err(err) => {
                self.fatal_error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.apply(Intent::Quit);
                if self.quit {
                    self.save_prefs();
                    event_loop.exit();
                    return;
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {}
            WindowEvent::Focused(focused) => {
                // Buttons and keys released outside the window never report;
                // don't leave a drag or a modifier stuck. On regaining focus
                // start clean too: the shortcut that switched windows (Alt+Tab,
                // Super+…) must not leak in. Real modifier state follows in
                // a ModifiersChanged event.
                self.input.release_all();
                self.ui.code_commit_switch();
                if !focused {
                    self.doc.end_gesture();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.input.modifiers = modifiers.state();
                // Releasing Ctrl settles the Ctrl+Tab switcher, like VS Code.
                if !self.input.ctrl() {
                    self.ui.code_commit_switch();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        self.input.left = down;
                        if down {
                            let now = Instant::now();
                            let at = self.input.cursor;
                            let double = self.input.last_press.is_some_and(|(t, p)| {
                                now.duration_since(t) < DOUBLE_CLICK
                                    && (p.0 - at.0).abs() < 6.0
                                    && (p.1 - at.1).abs() < 6.0
                            });
                            self.input.pressed = true;
                            self.input.double_click = double;
                            // A third click starts a new sequence.
                            self.input.last_press = if double { None } else { Some((now, at)) };
                        }
                    }
                    MouseButton::Right => {
                        self.input.right = down;
                        if down {
                            self.input.right_pressed = true;
                        }
                    }
                    MouseButton::Middle => {
                        self.input.middle = down;
                        if down {
                            self.input.middle_pressed = true;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // winit fakes presses for keys already held when the window
                // gains focus (X11, Windows) — e.g. the Tab of Alt+Tab. They
                // were meant for the window manager, not Oberon.
                if is_synthetic {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        self.input.held.remove(&code);
                    }
                    return;
                }
                self.keyboard(&event);
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
                return;
            }
            _ => return,
        }
        self.request_redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            self.save_prefs();
            event_loop.exit();
            return;
        }
        // Redraw at a steady ~60 FPS so play mode, hover and drags stay live
        // without spinning the CPU between frames.
        let deadline = self.last_frame + Duration::from_millis(16);
        if Instant::now() >= deadline {
            self.request_redraw();
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }
    }
}

/// Maps a key to the script editor's key model (letters, digits and
/// punctuation only when Ctrl / Alt is held; plain typing arrives as text).
fn code_key(code: KeyCode, m: code::Mods) -> Option<code::Key> {
    use code::Key;
    let key = match code {
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Escape => Key::Escape,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F12 => Key::F12,
        _ if m.ctrl || m.alt => Key::Char(code_char(code)?),
        _ => return None,
    };
    Some(key)
}

/// The unshifted character of a physical key (US layout).
fn code_char(code: KeyCode) -> Option<char> {
    let letters = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyB, 'b'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyD, 'd'),
        (KeyCode::KeyE, 'e'),
        (KeyCode::KeyF, 'f'),
        (KeyCode::KeyG, 'g'),
        (KeyCode::KeyH, 'h'),
        (KeyCode::KeyI, 'i'),
        (KeyCode::KeyJ, 'j'),
        (KeyCode::KeyK, 'k'),
        (KeyCode::KeyL, 'l'),
        (KeyCode::KeyM, 'm'),
        (KeyCode::KeyN, 'n'),
        (KeyCode::KeyO, 'o'),
        (KeyCode::KeyP, 'p'),
        (KeyCode::KeyQ, 'q'),
        (KeyCode::KeyR, 'r'),
        (KeyCode::KeyS, 's'),
        (KeyCode::KeyT, 't'),
        (KeyCode::KeyU, 'u'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyW, 'w'),
        (KeyCode::KeyX, 'x'),
        (KeyCode::KeyY, 'y'),
        (KeyCode::KeyZ, 'z'),
        (KeyCode::Digit0, '0'),
        (KeyCode::Digit1, '1'),
        (KeyCode::Digit2, '2'),
        (KeyCode::Digit3, '3'),
        (KeyCode::Digit4, '4'),
        (KeyCode::Digit5, '5'),
        (KeyCode::Digit6, '6'),
        (KeyCode::Digit7, '7'),
        (KeyCode::Digit8, '8'),
        (KeyCode::Digit9, '9'),
        (KeyCode::Slash, '/'),
        (KeyCode::NumpadDivide, '/'),
        (KeyCode::BracketLeft, '['),
        (KeyCode::BracketRight, ']'),
        (KeyCode::Space, ' '),
        (KeyCode::Comma, ','),
        (KeyCode::Period, '.'),
        (KeyCode::Minus, '-'),
        (KeyCode::Equal, '='),
        (KeyCode::Backquote, '`'),
        (KeyCode::Backslash, '\\'),
        (KeyCode::Semicolon, ';'),
        (KeyCode::Quote, '\''),
    ];
    letters.iter().find(|(k, _)| *k == code).map(|(_, c)| *c)
}
