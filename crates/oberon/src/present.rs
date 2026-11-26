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

use automata_core::TransformData;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorGrabMode, CursorIcon, Window, WindowId};

use automata_audio::AudioEngine;
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
    /// Mouse buttons scripts see as held (pressed over the game view, or
    /// anywhere while the cursor is captured).
    game_buttons: HashSet<&'static str>,
    /// Mouse movement not yet sent to scripts (physical pixels).
    mouse_delta: (f32, f32),
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
        self.game_buttons.clear();
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

/// The gamepad backend, opened lazily.
enum Pads {
    NotOpened,
    /// It couldn't start (no backend on this system): don't retry.
    Unavailable,
    Open(Box<automata_input::Gamepads>),
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
    /// The scripts' time in their last answered frame (for the Profiler).
    last_script_ms: f32,
    /// Gamepads for scripts (`"pad_*"` keys), opened when Play first sends
    /// a frame.
    gamepads: Pads,
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
    /// When model files were last checked for changes (hot reload).
    last_model_check: Instant,
    /// The sound output (opened on first use: Play or a clip preview).
    audio: Option<AudioEngine>,
    /// A script asked for the mouse to be captured (mouse-look).
    lock_wanted: bool,
    /// The cursor is captured right now (Esc releases it).
    lock_active: bool,
    /// The clip preview playing from the Files panel / Inspector.
    preview_sound: Option<(u64, PathBuf)>,
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
            last_script_ms: 0.0,
            gamepads: Pads::NotOpened,
            edit_camera: None,
            last_frame: Instant::now(),
            last_title: String::new(),
            last_cursor: CursorHint::Default,
            fatal_error: None,
            last_autosave: Instant::now(),
            build: None,
            quit: false,
            last_model_check: Instant::now(),
            audio: None,
            lock_wanted: false,
            lock_active: false,
            preview_sound: None,
        })
    }

    /// The sound output, opened on first use.
    fn audio(&mut self) -> &mut AudioEngine {
        self.audio.get_or_insert_with(AudioEngine::new)
    }

    /// Captures (hides and locks) or releases the cursor for mouse-look.
    fn set_cursor_lock(&mut self, locked: bool) {
        if self.lock_active == locked {
            return;
        }
        self.lock_active = locked;
        self.input.mouse_delta = (0.0, 0.0);
        let Some(window) = &self.window else {
            return;
        };
        if locked {
            if window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
                .is_err()
            {
                self.ui.log(
                    LogLevel::Warn,
                    "This system can't capture the mouse; mouse-look still works while the \
                     cursor stays in the window.",
                );
            }
            window.set_cursor_visible(false);
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
    }

    /// The UI's logical-pixel scale (window DPI × UI scale preference).
    fn ui_scale(&self) -> f32 {
        let dpi = self
            .window
            .as_ref()
            .map_or(1.0, |w| w.scale_factor() as f32);
        dpi * self.ui.prefs.ui_scale.clamp(0.5, 3.0)
    }

    /// Whether the pointer is over the game view.
    fn pointer_in_view(&self) -> bool {
        let s = self.ui_scale().max(0.1);
        let r = self.ui.viewport_rect();
        let (x, y) = (self.input.cursor.0 / s, self.input.cursor.1 / s);
        r.w > 0.0 && x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
    }

    /// Mouse buttons for scripts, and click-to-capture. Returns whether the
    /// click belongs to the game alone (the editor UI must not see it).
    fn game_mouse_button(&mut self, button: MouseButton, down: bool) -> bool {
        let name = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
            _ => return self.lock_active,
        };
        if !down {
            self.input.game_buttons.remove(name);
            return self.lock_active;
        }
        let playing = self.play.as_ref().is_some_and(|p| !p.paused) && self.bridge.is_some();
        if self.lock_active {
            self.input.game_buttons.insert(name);
            return true;
        }
        if playing && self.pointer_in_view() && self.ui.scene_visible() {
            self.input.game_buttons.insert(name);
            if self.lock_wanted && button == MouseButton::Left {
                // Click the view to capture the mouse again after Esc.
                self.set_cursor_lock(true);
                return true;
            }
        }
        false
    }

    /// Plays a sound file as a preview (stops the previous preview). Playing
    /// the same file again stops it instead.
    pub(crate) fn preview_clip(&mut self, path: &std::path::Path) {
        if let Some((id, current)) = self.preview_sound.take() {
            self.audio().stop(id, 0.05);
            if current == path {
                return;
            }
        }
        let text = path.to_string_lossy().into_owned();
        match self.audio().play(automata_audio::PlayOptions::once(&text)) {
            Ok(id) => {
                let seconds = self.audio().duration(&text).unwrap_or(0.0);
                self.ui.log(
                    LogLevel::Info,
                    format!(
                        "Playing {} ({seconds:.1}s). Click again to stop.",
                        path.file_name()
                            .map_or_else(|| text.clone(), |n| n.to_string_lossy().into_owned())
                    ),
                );
                self.preview_sound = Some((id, path.to_path_buf()));
            }
            Err(e) => self
                .ui
                .log(LogLevel::Error, format!("Can't play {text}: {e}")),
        }
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
        if self.fan_out(&intent) {
            return;
        }
        self.apply_edit(intent);
    }

    /// Applies an edit aimed at one of several selected entities to every
    /// selected entity, as one undo step. Returns false (doing nothing) for
    /// intents that aren't such an edit.
    fn fan_out(&mut self, intent: &Intent) -> bool {
        let ids = self.doc.selection();
        let Some(primary) = intent.fan_target() else {
            return false;
        };
        if ids.len() < 2 || !ids.contains(&primary) {
            return false;
        }
        let data = |doc: &Document, id: u64| doc.index_of(id).map(|i| doc.entities[i].data.clone());
        let Some(lead) = data(&self.doc, primary) else {
            return false;
        };
        let list: Vec<Intent> = match intent {
            Intent::Duplicate(_) => {
                let copies = self.doc.duplicate_many(&ids);
                self.ui.log(
                    LogLevel::Info,
                    format!("Duplicated {} entities", copies.len()),
                );
                return true;
            }
            // Every selected entity takes the lead's new value.
            Intent::Toggle { flag, .. } => {
                let Some(on) = fields::get_flag(&lead, *flag) else {
                    return false;
                };
                ids.iter()
                    .filter(|&&id| {
                        data(&self.doc, id).and_then(|d| fields::get_flag(&d, *flag)) == Some(on)
                    })
                    .map(|&id| Intent::Toggle { id, flag: *flag })
                    .collect()
            }
            Intent::Cycle { choice, .. } => {
                let mut next = lead.clone();
                fields::cycle(&mut next, *choice);
                let Some(label) = fields::choice_label(&next, *choice) else {
                    return false;
                };
                ids.iter()
                    .map(|&id| Intent::SetChoice {
                        id,
                        choice: *choice,
                        label,
                    })
                    .collect()
            }
            Intent::SetTransform { transform, .. } => {
                let Some(before) = lead.transform else {
                    return false;
                };
                ids.iter()
                    .filter_map(|&id| {
                        let t = data(&self.doc, id)?.transform?;
                        let transform = if id == primary {
                            *transform
                        } else {
                            group_transform(&before, transform, &t)
                        };
                        Some(Intent::SetTransform { id, transform })
                    })
                    .collect()
            }
            other => {
                let list: Vec<Intent> = ids.iter().filter_map(|&id| other.retarget(id)).collect();
                if list.is_empty() {
                    return false;
                }
                list
            }
        };
        let deleting = matches!(intent, Intent::Delete(_));
        let count = list.len();
        self.doc.begin_batch();
        for i in list {
            self.apply_edit(i);
        }
        self.doc.end_batch();
        if deleting {
            self.ui
                .log(LogLevel::Info, format!("Deleted {count} entities"));
        }
        true
    }

    /// Applies one edit (or editor action) to the document.
    #[allow(clippy::too_many_lines)]
    fn apply_edit(&mut self, intent: Intent) {
        let doc = &mut self.doc;
        match intent {
            Intent::Brush {
                id,
                tool,
                point,
                normal,
                radius,
                strength,
                anchor,
            } => {
                doc.brush(id, tool, point, normal, radius, strength, anchor);
            }
            Intent::Select(id) => {
                doc.select(id);
                if let Some(play) = &mut self.play {
                    play.doc.select(id);
                }
            }
            Intent::SelectMore { id, range } => {
                if range {
                    doc.select_range(id);
                } else {
                    doc.toggle_selected(id);
                }
                let ids = doc.selection();
                if let Some(play) = &mut self.play {
                    play.doc.select_many(&ids);
                }
            }
            Intent::SelectMany(ids) => {
                doc.select_many(&ids);
                if let Some(play) = &mut self.play {
                    play.doc.select_many(&ids);
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
                if name.contains(':') {
                    self.ui.log(
                        LogLevel::Warn,
                        format!(
                            "Entity names can't contain ':' ({name}): it separates the parts \
                             of a script key like Player:animation:clip"
                        ),
                    );
                } else {
                    doc.rename_entity(id, &name);
                }
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
                // One coalesced undo step moves the whole selection.
                for id in doc.selection() {
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
            Intent::SetAnimationClip { id, clip } => {
                doc.modify(id, false, |d| {
                    if let Some(a) = &mut d.animation {
                        a.clip.clone_from(&clip);
                    }
                });
                if let Some(p) = self.ui.anim_preview.as_mut().filter(|p| p.id == id) {
                    p.time = 0.0;
                }
            }
            Intent::SetAttach { id, to, bone } => {
                doc.modify(id, false, |d| {
                    let Some(a) = &mut d.attach else {
                        return;
                    };
                    if let Some(to) = to {
                        // Attaching for the first time puts it on the bone.
                        if a.to.is_empty() {
                            if let Some(t) = &mut d.transform {
                                t.position = [0.0; 3];
                            }
                        }
                        a.to = to;
                    }
                    if let Some(bone) = bone {
                        a.bone = bone;
                    }
                });
            }
            Intent::SetNavLink { id, to } => {
                doc.modify(id, false, |d| {
                    if let Some(l) = &mut d.nav_link {
                        l.to.clone_from(&to);
                    }
                });
            }
            Intent::AutoPaintTerrain(id) => {
                doc.auto_paint_terrain(id, 1.5, 38.0);
            }
            Intent::ImportHeightmap { id, path } => {
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| doc.import_heightmap(id, &bytes))
                {
                    Ok(()) => self.ui.log(
                        LogLevel::Info,
                        format!("Terrain heights from {}", path.display()),
                    ),
                    Err(e) => self.ui.log(
                        LogLevel::Error,
                        format!("Couldn't import {}: {e}", path.display()),
                    ),
                }
            }
            Intent::BakeNav(id) => match PlaySession::bake_nav_to_file(doc, id) {
                Ok((path, mesh)) => {
                    self.ui.log(
                        LogLevel::Info,
                        format!(
                            "Baked the navigation mesh: {:.0} m² walkable in {} areas, {} jump links \
                             ({})",
                            mesh.area(),
                            mesh.regions,
                            mesh.links.len(),
                            path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into()),
                        ),
                    );
                    self.ui.nav_missing = None;
                    self.ui.nav_overlay = Some((path, Arc::new(mesh)));
                    self.ui.show_nav = true;
                }
                Err(message) => self.ui.log(LogLevel::Error, message),
            },
            Intent::SetJointTarget { id, target } => {
                doc.modify(id, false, |d| {
                    if let Some(j) = &mut d.joint {
                        j.connected.clone_from(&target);
                    }
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
            Intent::DuplicateAlong {
                id,
                count,
                step,
                turn_y,
            } => {
                let ids = doc.duplicate_along(id, count, step, turn_y);
                self.ui.log(
                    LogLevel::Info,
                    format!("Placed {} copies along a line", ids.len()),
                );
            }
            Intent::ApplyPrefab(id) => match doc.apply_to_prefab(id) {
                Ok((path, 0)) => self.ui.log(
                    LogLevel::Info,
                    format!("Nothing to apply: it matches {}", path.display()),
                ),
                Ok((path, n)) => {
                    self.ui.log(
                        LogLevel::Info,
                        format!(
                            "Applied {n} override(s) to {}; its instances follow",
                            path.display()
                        ),
                    );
                    self.ui.files_changed(Some(&path));
                }
                Err(e) => self.ui.log(LogLevel::Error, format!("Apply failed: {e}")),
            },
            Intent::RevertPrefab { id, paths } => {
                if doc.revert_overrides(id, paths.as_deref()) {
                    self.ui.log(LogLevel::Info, "Reverted to the prefab");
                }
            }
            Intent::UnlinkPrefab(id) => {
                if doc.unlink_prefab(id) {
                    self.ui.log(
                        LogLevel::Info,
                        "Unlinked from its prefab (it keeps its values)",
                    );
                }
            }
            Intent::SelectInstances(id) => {
                let path = doc
                    .index_of(id)
                    .and_then(|i| doc.entities[i].data.prefab.as_ref())
                    .map(|l| l.path.clone());
                if let Some(path) = path {
                    let mut ids = doc.instances_of(&path);
                    // Keep the clicked one primary.
                    ids.retain(|&x| x != id);
                    ids.push(id);
                    doc.select_many(&ids);
                    self.ui.log(
                        LogLevel::Info,
                        format!("Selected {} instance(s) of {path}", ids.len()),
                    );
                }
            }
            Intent::CreateVariant(path) => match doc.create_variant(&path) {
                Ok(variant) => {
                    self.ui.log(
                        LogLevel::Info,
                        format!("Created variant {}", variant.display()),
                    );
                    self.ui.files_changed(Some(&variant));
                }
                Err(e) => self.ui.log(LogLevel::Error, format!("Variant failed: {e}")),
            },
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
            Intent::AddModel { path, at } => self.add_model(&path, at),
            Intent::SetModel { id, path } => {
                if let Some(rel) = self.import_model(&path) {
                    if self.doc.set_model(id, &rel) {
                        // A collider made for the old mesh would be wrong.
                        self.doc.fit_collider_to_new_mesh(id);
                        self.ui.log(LogLevel::Info, format!("Mesh set to {rel}"));
                    }
                }
            }
            Intent::SetMaterial { id, path } => self.set_material(id, path.as_deref()),
            Intent::SetAnimGraph { id, path } => self.set_anim_graph(id, path),
            Intent::NewMaterial(path) => {
                if let Some(path) = self.ensure_material(path) {
                    self.ui.files_changed(Some(&path));
                    self.open_script(&path, None);
                }
            }
            Intent::AddSound { path, at } => self.add_sound(&path, at),
            Intent::SetAudioClip { id, path } => {
                if let Some(rel) = self.import_sound(&path) {
                    if self.doc.set_audio_clip(id, &rel) {
                        self.ui.log(LogLevel::Info, format!("Clip set to {rel}"));
                    }
                }
            }
            Intent::PreviewSound(path) => self.preview_clip(&path),
            Intent::ImportFont(path) => self.import_font(&path),
            Intent::SetImport { model, settings } => self.set_import(&model, &settings),
            Intent::FitCollider(id) => {
                if self.doc.fit_collider(id) {
                    self.ui.log(LogLevel::Info, "Collider fitted to the mesh");
                }
            }
            Intent::OpenScript(path) => self.open_script(&path, None),
            Intent::OpenScriptAt { path, line } => self.open_script(&path, Some(line)),
            Intent::NewScript(path) => {
                if self.ensure_script(&path) {
                    self.ui.files_changed(Some(&path));
                    self.open_script(&path, None);
                }
            }
            Intent::NewUiScreen(path) => {
                if let Some(path) = self.ensure_ui_screen(path) {
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
            Intent::MoveFile { from, to } => self.move_file(&from, &to),
            Intent::DuplicateFile(path) => match crate::files::duplicate(&path) {
                Ok(copy) => {
                    self.ui.files_changed(Some(&copy));
                    self.ui.log(
                        LogLevel::Info,
                        format!("Created {}", crate::files::display_name(&copy)),
                    );
                }
                Err(e) => self.ui.log(LogLevel::Error, e),
            },
            Intent::DeleteFile(path) => match crate::files::delete(&path) {
                Ok(()) => {
                    self.ui.files_changed(None);
                    let how = if crate::files::deletes_to_trash() {
                        "Moved to the Trash"
                    } else {
                        "Deleted"
                    };
                    self.ui.log(
                        LogLevel::Info,
                        format!("{how}: {}", crate::files::display_name(&path)),
                    );
                }
                Err(e) => self.ui.log(LogLevel::Error, e),
            },
            Intent::RevealFile(path) => {
                if let Err(e) = crate::files::reveal(&path) {
                    self.ui.log(LogLevel::Error, e);
                }
            }
            Intent::Quit => self.quit = true,
            Intent::SetEnvironment(id, env) => {
                doc.modify(id, false, |d| d.environment = Some(*env));
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
            Ok(()) => {
                // The entity becomes its first instance.
                let rel = self.doc.relative(path);
                self.doc.link_prefab(id, &rel);
                self.ui
                    .log(LogLevel::Info, format!("Saved prefab {}", path.display()));
            }
            Err(e) => self
                .ui
                .log(LogLevel::Error, format!("Save prefab failed: {e}")),
        }
    }

    /// Saves a model's import settings and reloads it at once.
    fn set_import(
        &mut self,
        model: &std::path::Path,
        settings: &automata_assets::import::ImportSettings,
    ) {
        match automata_assets::import::write_settings(model, settings) {
            Ok(()) => {
                automata_assets::refresh();
                self.viewport.invalidate();
                self.ui.files_changed(None);
            }
            Err(e) => self.ui.log(
                LogLevel::Error,
                format!("Couldn't save import settings for {}: {e}", model.display()),
            ),
        }
    }

    /// Keeps model paths resolving against the scene's folder, and reloads
    /// models, materials and textures whose files changed on disk (checked
    /// twice a second).
    fn watch_models(&mut self) {
        self.sync_material_previews();
        self.sync_anim_editor();
        let root = self.doc.base_dir();
        if automata_assets::root().as_deref() != Some(root.as_path()) {
            automata_assets::set_root(Some(&root));
            self.viewport.invalidate();
        }
        if self.last_model_check.elapsed() >= Duration::from_millis(500) {
            self.last_model_check = Instant::now();
            if automata_assets::refresh() {
                self.viewport.invalidate();
                self.ui
                    .log(LogLevel::Info, "Asset files changed on disk; reloaded");
            }
            if self.play.is_none() {
                let n = self.doc.poll_prefabs();
                if n > 0 {
                    self.ui.log(
                        LogLevel::Info,
                        format!("A prefab changed on disk; {n} instance(s) follow"),
                    );
                }
            }
        }
        for note in std::mem::take(&mut self.doc.notes) {
            self.ui.log(LogLevel::Warn, note);
        }
    }

    /// A file dropped on the window from the system file manager: models
    /// are imported and added where they were dropped.
    fn dropped_file(&mut self, path: &std::path::Path) {
        if crate::fonts::is_font_path(path) {
            self.import_font(path);
            return;
        }
        let sound = automata_core::audio_source::is_audio_path(path);
        if !sound && !automata_assets::is_model_path(path) {
            self.ui.log(
                LogLevel::Warn,
                format!(
                    "Can't import {}: drop .glb, .gltf or .obj models, .ogg, .wav, .mp3 \
                     or .flac sounds, or .ttf or .otf fonts",
                    path.display()
                ),
            );
            return;
        }
        if self.ui.play != PlayState::Editing {
            self.ui.log(LogLevel::Warn, "Stop play mode to add files");
            return;
        }
        let scale = self
            .window
            .as_ref()
            .map_or(1.0, |w| w.scale_factor() as f32)
            * self.ui.prefs.ui_scale.clamp(0.5, 3.0);
        let (x, y) = (self.input.cursor.0 / scale, self.input.cursor.1 / scale);
        let at = self.ui.viewport_rect().contains(x, y).then_some((x, y));
        if sound {
            self.add_sound(path, at);
        } else {
            self.add_model(path, at);
        }
    }

    /// Renames or moves a file or folder, then points the scene, the open
    /// script tabs and the scene's own path at the new place.
    fn move_file(&mut self, from: &std::path::Path, to: &std::path::Path) {
        if let Err(e) = crate::files::rename(from, to) {
            self.ui.log(LogLevel::Error, e);
            return;
        }
        self.ui.code.retarget(from, to);
        // Moving the scene (or a folder holding it) carries its relative
        // references along; anything else is re-pointed.
        let scene_moved = self.doc.path.clone().and_then(|scene| {
            scene
                .strip_prefix(from)
                .ok()
                .map(std::path::Path::to_path_buf)
        });
        let refs = match scene_moved {
            Some(rest) => {
                self.doc.path = Some(if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                });
                0
            }
            None => self.doc.retarget_paths(from, to),
        };
        self.ui.files_changed(Some(to));
        let what = if refs > 0 {
            format!(" ({refs} object(s) in the scene updated; save to keep)")
        } else {
            String::new()
        };
        self.ui.log(
            LogLevel::Info,
            format!(
                "Renamed {} → {}{what}",
                crate::files::display_name(from),
                crate::files::display_name(to)
            ),
        );
    }

    /// Brings a sound into the project (copying it into `sounds/` when it
    /// lives outside). Returns its scene-relative path, or `None` after
    /// logging why not.
    fn import_sound(&mut self, path: &std::path::Path) -> Option<String> {
        let root = self.doc.base_dir();
        let dest = match crate::sounds::import_sound(path, &root) {
            Ok(dest) => dest,
            Err(e) => {
                self.ui.log(LogLevel::Error, format!("Import failed: {e}"));
                self.ui.show_console();
                return None;
            }
        };
        if let Err(e) = self.ui.clips.get(&dest).clone() {
            self.ui
                .log(LogLevel::Warn, format!("{}: {e}", dest.display()));
        }
        let rel = self.doc.relative(&dest);
        if dest != path {
            self.ui.files_changed(Some(&dest));
            self.ui.log(LogLevel::Info, format!("Imported {rel}"));
        }
        Some(rel)
    }

    /// Brings a font into the project (copying it into `fonts/` when it
    /// lives outside), shows it in the Assets tab, and copies the path a
    /// script names it by.
    fn import_font(&mut self, path: &std::path::Path) {
        let root = self.doc.base_dir();
        let dest = match crate::fonts::import_font(path, &root) {
            Ok(dest) => dest,
            Err(e) => {
                self.ui.log(LogLevel::Error, format!("Import failed: {e}"));
                self.ui.show_console();
                return;
            }
        };
        let rel = crate::fonts::script_path(&dest, &root);
        self.ui.code.clipboard.set(&rel);
        if dest == path {
            self.ui.log(
                LogLevel::Info,
                format!("Copied {rel} (use it as font=\"{rel}\")"),
            );
        } else {
            self.ui.files_changed(Some(&dest));
            self.ui.show_fonts();
            self.ui.log(
                LogLevel::Info,
                format!(
                    "Imported {rel}. Scripts use it with font=\"{rel}\" (copied), \
                     e.g. ctx.hud.text(\"Hi\", 20, 20, font=\"{rel}\")"
                ),
            );
        }
    }

    /// Adds an entity whose Audio Source plays the sound at `path`, at the
    /// drop point `at` (or where the scene camera looks).
    fn add_sound(&mut self, path: &std::path::Path, at: Option<(f32, f32)>) {
        let Some(rel) = self.import_sound(path) else {
            return;
        };
        let target = glam::Vec3::from(self.ui.camera.target);
        let spot = match at {
            Some((x, y)) => {
                let proj = crate::view::Projector::new(&self.ui.camera, self.ui.viewport_rect());
                let fallback = (proj.eye() - target).length();
                proj.ground_point(x, y, fallback)
            }
            None => target,
        };
        let name = crate::sounds::entity_name(path);
        let id = self.doc.create_sound(&rel, &name, spot.to_array());
        let name = self
            .doc
            .index_of(id)
            .map(|i| self.doc.entities[i].name().to_string())
            .unwrap_or_default();
        self.ui.log(
            LogLevel::Info,
            format!("Added {name}: plays {rel} when the game starts"),
        );
    }

    /// Brings a model into the project (copying it into `models/` when it
    /// lives outside) and checks it loads. Returns its scene-relative path,
    /// or `None` after logging why not.
    fn import_model(&mut self, path: &std::path::Path) -> Option<String> {
        let root = self.doc.base_dir();
        let dest = match crate::models::import_model(path, &root) {
            Ok(dest) => dest,
            Err(e) => {
                self.ui.log(LogLevel::Error, format!("Import failed: {e}"));
                self.ui.show_console();
                return None;
            }
        };
        let copied = dest != path;
        if let Err(e) = automata_assets::load(&dest.to_string_lossy()) {
            self.ui.log(LogLevel::Error, e.to_string());
            self.ui.show_console();
            return None;
        }
        let rel = self.doc.relative(&dest);
        if copied {
            self.ui.files_changed(Some(&dest));
            self.ui.log(LogLevel::Info, format!("Imported {rel}"));
        }
        Some(rel)
    }

    /// Adds an entity showing the model at `path`. `at` is a drop point in
    /// the scene view (the model stands on the ground there); `None` puts it
    /// where the scene camera is looking.
    fn add_model(&mut self, path: &std::path::Path, at: Option<(f32, f32)>) {
        let Some(rel) = self.import_model(path) else {
            return;
        };
        let target = glam::Vec3::from(self.ui.camera.target);
        let spot = match at {
            Some((x, y)) => {
                let proj = crate::view::Projector::new(&self.ui.camera, self.ui.viewport_rect());
                let fallback = (proj.eye() - target).length();
                proj.ground_point(x, y, fallback)
            }
            None => target,
        };
        let name = crate::models::entity_name(path);
        let id = self.doc.create_model(&rel, &name, spot.to_array());
        let summary = automata_assets::load(&rel)
            .map(|m| crate::models::describe(&m))
            .unwrap_or_default();
        let name = self
            .doc
            .index_of(id)
            .map(|i| self.doc.entities[i].name().to_string())
            .unwrap_or_default();
        self.ui
            .log(LogLevel::Info, format!("Added {name} ({summary})"));
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
                let rel = self.doc.relative(path);
                let ids = self
                    .doc
                    .instantiate_linked(&rel, &prefab, [t[0], ground, t[2]]);
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

    /// Puts a material file on an entity's Mesh Renderer (`None` clears
    /// it), stored relative to the scene folder.
    fn set_material(&mut self, id: u64, path: Option<&std::path::Path>) {
        let who = self
            .doc
            .index_of(id)
            .map(|i| self.doc.entities[i].name().to_string())
            .unwrap_or_default();
        let Some(path) = path else {
            if self.doc.set_material(id, "") {
                self.ui
                    .log(LogLevel::Info, format!("{who} uses its own colour again"));
            }
            return;
        };
        if !automata_assets::is_material_path(path) {
            self.ui.log(
                LogLevel::Warn,
                format!("{} isn't a material (.mat.ron)", path.display()),
            );
            return;
        }
        let has_mesh = self
            .doc
            .index_of(id)
            .is_some_and(|i| self.doc.entities[i].data.mesh_visual.is_some());
        if !has_mesh {
            self.ui.log(
                LogLevel::Warn,
                format!("{who} has no Mesh Renderer to put a material on"),
            );
            return;
        }
        let rel = self.doc.relative(path);
        if std::path::Path::new(&rel).is_absolute() {
            self.ui.log(
                LogLevel::Warn,
                format!(
                    "{} is outside the scene's folder and won't ship with the game: \
                     move it into the project",
                    path.display()
                ),
            );
        }
        if let Err(e) = automata_assets::load_material(&rel) {
            self.ui.log(LogLevel::Warn, format!("{e}"));
        }
        if self.doc.set_material(id, &rel) {
            self.ui.log(LogLevel::Info, format!("{who} now uses {rel}"));
        }
    }

    /// Points an entity's Animation at a graph (making the file from the
    /// model's clips if it doesn't exist), or back at its clip (`None`).
    fn set_anim_graph(&mut self, id: u64, path: Option<std::path::PathBuf>) {
        let Some(index) = self.doc.index_of(id) else {
            return;
        };
        let who = self.doc.entities[index].name().to_string();
        let Some(path) = path else {
            let changed = self.doc.modify(id, false, |d| {
                if let Some(a) = &mut d.animation {
                    a.graph.clear();
                }
            });
            if changed {
                self.ui
                    .log(LogLevel::Info, format!("{who} plays its clip again"));
            }
            return;
        };
        let path = if automata_assets::anim_graph::is_anim_graph_path(&path) {
            path
        } else {
            let name = path
                .file_name()
                .map_or_else(|| "character".into(), |s| s.to_string_lossy().into_owned());
            let stem = name
                .strip_suffix(".ron")
                .or_else(|| name.strip_suffix(".anim"))
                .unwrap_or(&name)
                .to_string();
            path.with_file_name(format!("{stem}.anim.ron"))
        };
        let rig = crate::doc::model_rig(&self.doc.entities[index].data);
        if rig.as_ref().is_none_or(|r| r.clips.is_empty()) {
            self.ui.log(
                LogLevel::Warn,
                format!("{who} needs an animated model (a .glb or .gltf with clips) for a graph"),
            );
            return;
        }
        let rel = self.doc.relative(&path);
        if std::path::Path::new(&rel).is_absolute() {
            self.ui.log(
                LogLevel::Warn,
                format!(
                    "{} is outside the scene's folder and won't ship with the game: \
                     move it into the project",
                    path.display()
                ),
            );
            return;
        }
        if !path.exists() {
            let clips = rig.as_ref().map(|r| r.clip_names()).unwrap_or_default();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Err(e) = automata_assets::AnimGraph::starter(&clips).save(&path, &name) {
                self.ui.log(
                    LogLevel::Error,
                    format!("Couldn't create {}: {e}", path.display()),
                );
                return;
            }
            self.ui.files_changed(Some(&path));
            self.ui.log(
                LogLevel::Info,
                format!("Created {rel} from the model's clips · edit it to add transitions"),
            );
        }
        let fresh = crate::doc::default_animation(&self.doc.entities[index].data);
        let changed = self.doc.modify(id, false, |d| {
            d.animation.get_or_insert(fresh).graph.clone_from(&rel);
        });
        if changed {
            self.ui
                .log(LogLevel::Info, format!("{who} is animated by {rel}"));
        }
    }

    /// Creates a material file unless it exists (any name gets the
    /// `.mat.ron` ending). Returns its path.
    fn ensure_material(&mut self, path: std::path::PathBuf) -> Option<std::path::PathBuf> {
        let path = if automata_assets::is_material_path(&path) {
            path
        } else {
            let name = path
                .file_name()
                .map_or_else(|| "material".into(), |s| s.to_string_lossy().into_owned());
            let mut stem = name.as_str();
            for suffix in [".ron", ".mat"] {
                if stem.to_ascii_lowercase().ends_with(suffix) {
                    stem = &stem[..stem.len() - suffix.len()];
                }
            }
            path.with_file_name(format!("{stem}.mat.ron"))
        };
        if path.exists() {
            return Some(path);
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match automata_assets::MaterialFile::default().save(&path) {
            Ok(()) => {
                self.ui.log(
                    LogLevel::Info,
                    format!(
                        "Created {} · drag it onto an object to use it",
                        path.display()
                    ),
                );
                Some(path)
            }
            Err(e) => {
                self.ui.log(
                    LogLevel::Error,
                    format!("Create {} failed: {e}", path.display()),
                );
                None
            }
        }
    }

    /// Shows open material tabs' unsaved edits on every object using them
    /// (and stops once they're saved or closed).
    fn sync_material_previews(&mut self) {
        let canonical =
            |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let wanted = self.ui.material_tab_previews();
        let mut changed = false;
        for (path, file) in &wanted {
            changed |= automata_assets::set_material_preview(path, file.as_ref());
        }
        for shown in automata_assets::material::material_previews() {
            if !wanted.iter().any(|(p, _)| canonical(p) == shown) {
                changed |= automata_assets::set_material_preview(&shown, None);
            }
        }
        if changed {
            self.viewport.invalidate();
        }
    }

    /// Runs open graph tabs' unsaved edits in Play (and stops once they're
    /// saved or closed), and shows the graph editor what the selected
    /// entity's graph is doing (else the first entity running it).
    fn sync_anim_editor(&mut self) {
        let canonical =
            |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let wanted = self.ui.anim_tab_previews();
        for (path, graph) in &wanted {
            automata_assets::anim_graph::set_anim_graph_preview(path, graph.as_ref());
        }
        for shown in automata_assets::anim_graph::anim_graph_previews() {
            if !wanted.iter().any(|(p, _)| canonical(p) == shown) {
                automata_assets::anim_graph::set_anim_graph_preview(&shown, None);
            }
        }
        let live = self.ui.anim_editor_path().and_then(|path| {
            let play = self.play.as_ref()?;
            let me = canonical(&path);
            let running: Vec<_> = play
                .anim_graphs()
                .into_iter()
                .filter(|(_, g, _)| canonical(&play.doc.resolve(g)) == me)
                .collect();
            let selected = play.doc.selected_index();
            let (i, _, state) = running
                .iter()
                .find(|(i, _, _)| Some(*i) == selected)
                .or_else(|| running.first())?
                .clone();
            Some(crate::ui::anim_editor::AnimLive {
                entity: play.doc.entities[i].name().to_string(),
                state,
            })
        });
        self.ui.set_anim_live(live);
    }

    /// Creates a UI document from a template unless it exists: a HUD for
    /// names ending in "hud", else a menu screen. Returns its path
    /// (always `*.ui.ron`).
    fn ensure_ui_screen(&mut self, path: std::path::PathBuf) -> Option<std::path::PathBuf> {
        let path = if crate::uidoc::is_ui_doc_path(&path) {
            path
        } else {
            let stem = path
                .file_stem()
                .map_or_else(|| "screen".into(), |s| s.to_string_lossy().into_owned());
            path.with_file_name(format!("{stem}.ui.ron"))
        };
        let inside_ui = path.parent().is_some_and(|d| {
            d.ancestors()
                .any(|a| a.file_name().is_some_and(|n| n == "ui"))
        });
        if !inside_ui {
            self.ui.log(
                LogLevel::Warn,
                "Scripts find UI screens in the project's ui/ folder: move it there to use it",
            );
        }
        if path.exists() {
            return Some(path);
        }
        let name = crate::uidoc::screen_name(&path);
        let doc = if name.ends_with("hud") {
            crate::uidoc::UiDoc::template_hud()
        } else {
            crate::uidoc::UiDoc::template_menu()
        };
        match doc.save(&path) {
            Ok(()) => {
                self.ui.log(
                    LogLevel::Info,
                    format!("Created {} · ctx.ui.screen(\"{name}\")", path.display()),
                );
                Some(path)
            }
            Err(e) => {
                self.ui.log(
                    LogLevel::Error,
                    format!("Create {} failed: {e}", path.display()),
                );
                None
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
                let mut session = PlaySession::start(&self.doc);
                if let Some(warning) = session.nav_warning.take() {
                    self.ui.log(LogLevel::Warn, warning);
                }
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
        if self.play.is_some() && paused {
            self.set_cursor_lock(false);
        }
        if let (Some(play), Some(audio)) = (&self.play, &mut self.audio) {
            play.pause_audio(audio, paused);
        }
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
        // The project's own .venv unless the preferences name a Python.
        let project = self.python_project();
        let python = crate::pyenv::project_python(self.ui.prefs.python.as_deref(), &project);
        if self.ui.prefs.python.is_none() && project.join("pyproject.toml").is_file() {
            match crate::pyenv::venv_state(&project) {
                crate::pyenv::VenvState::Outdated(v) => self.ui.log(
                    LogLevel::Warn,
                    format!(
                        "This project's engine is {v}; Oberon is {}. Run `oberon setup --upgrade` \
                         in {} to match.",
                        crate::pyenv::ENGINE_VERSION,
                        project.display()
                    ),
                ),
                crate::pyenv::VenvState::Missing | crate::pyenv::VenvState::NoEngine => {
                    self.ui.log(
                        LogLevel::Warn,
                        format!(
                            "This project has no .venv with the engine, so Play uses {python}. \
                             Run `oberon setup` in {} to give it its own.",
                            project.display()
                        ),
                    );
                }
                crate::pyenv::VenvState::Ready => {}
            }
        }
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
                    "Could not start Python ({python}): {err}. Run `oberon setup` in the \
                     project folder, or set `python` in ~/.config/oberon/prefs.ron. \
                     Scripts are disabled."
                ),
            ),
        }
    }

    /// The project whose `.venv` Play and the language server use: the
    /// nearest folder above the scene with a `pyproject.toml`, `.venv` or
    /// `build.ron`, else the scene's folder.
    fn python_project(&self) -> PathBuf {
        crate::pyenv::project_root(&self.ui.project_root)
            .unwrap_or_else(|| self.ui.project_root.clone())
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
            BridgeMsg::Frame(reply) => {
                if let Some(ms) = reply.script_ms {
                    self.last_script_ms = ms;
                }
                let mut lock = None;
                if let Some(play) = &mut self.play {
                    play.apply_frame(&reply);
                    lock = play.lock_request.take();
                }
                if let Some(locked) = lock {
                    self.lock_wanted = locked;
                    let focused = self.window.as_ref().is_some_and(|w| w.has_focus());
                    if !locked || focused {
                        self.set_cursor_lock(locked);
                    }
                    if locked {
                        self.ui.log(
                            LogLevel::Info,
                            "The game captured the mouse. Esc releases it; click the view to \
                             capture it again.",
                        );
                    }
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
        let cursor_in_view = !self.lock_active && self.pointer_in_view() && self.ui.scene_visible();
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
        if matches!(self.gamepads, Pads::NotOpened) {
            self.gamepads = automata_input::Gamepads::try_new()
                .map_or(Pads::Unavailable, |p| Pads::Open(Box::new(p)));
        }
        if let Pads::Open(pads) = &mut self.gamepads {
            pads.update();
            let _ = pads.drain_events();
            keys.extend(bridge::held_pad_keys(pads));
        }
        keys.sort_unstable();
        let mut frame = play.frame_input();
        let mut buttons: Vec<&'static str> = self.input.game_buttons.iter().copied().collect();
        buttons.sort_unstable();
        frame.mouse_buttons = buttons;
        frame.mouse_delta = [self.input.mouse_delta.0, self.input.mouse_delta.1];
        frame.mouse_locked = self.lock_active;
        frame.graphics =
            bridge::quality_key(play.graphics.unwrap_or(self.ui.prefs.graphics_quality));
        let s = {
            let dpi = self
                .window
                .as_ref()
                .map_or(1.0, |w| w.scale_factor() as f32);
            dpi * self.ui.prefs.ui_scale.clamp(0.5, 3.0)
        };
        let view = self.ui.viewport_rect();
        // The cursor in game pixels: the HUD is drawn at `view × s`.
        if cursor_in_view {
            frame.mouse_pos = Some([
                self.input.cursor.0 - view.x * s,
                self.input.cursor.1 - view.y * s,
            ]);
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        {
            frame.screen = [
                (view.w * s).round().max(1.0) as u32,
                (view.h * s).round().max(1.0) as u32,
            ];
        }
        if bridge.send_frame_with(dt, play.elapsed, &keys, &states, &frame) {
            self.input.mouse_delta = (0.0, 0.0);
        }
    }

    fn stop(&mut self) {
        if let Some(bridge) = self.bridge.take() {
            for msg in bridge.stop() {
                self.handle_bridge_msg(msg);
            }
        }
        if let Some(mut play) = self.play.take() {
            if let Some(audio) = &mut self.audio {
                play.stop_audio(audio);
            }
            self.ui.log(
                LogLevel::Info,
                format!("Stopped after {:.1}s. Scene restored.", play.elapsed),
            );
        }
        self.lock_wanted = false;
        self.set_cursor_lock(false);
        self.input.game_buttons.clear();
        self.viewport.clear_hud_cache();
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
        // Esc always gives a captured mouse back first.
        if code == KeyCode::Escape && self.lock_active && event.state == ElementState::Pressed {
            self.set_cursor_lock(false);
            return;
        }
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
        // The file browser moves its selection with the arrow keys (its
        // name field keeps the typing).
        if self.ui.file_dialog_open() {
            use crate::ui::file_browser::NavKey;
            let alt = self.input.modifiers.alt_key();
            let nav = match code {
                KeyCode::ArrowLeft if alt => Some(NavKey::Back),
                KeyCode::ArrowRight if alt => Some(NavKey::Forward),
                KeyCode::ArrowUp if alt => Some(NavKey::Parent),
                KeyCode::ArrowUp => Some(NavKey::Up),
                KeyCode::ArrowDown => Some(NavKey::Down),
                KeyCode::PageUp => Some(NavKey::PageUp),
                KeyCode::PageDown => Some(NavKey::PageDown),
                KeyCode::Home => Some(NavKey::First),
                KeyCode::End => Some(NavKey::Last),
                KeyCode::F5 => Some(NavKey::Refresh),
                _ => None,
            };
            if let Some(nav) = nav {
                if self.ui.file_dialog_key(nav) {
                    return;
                }
            }
        }
        // A modal dialog without a focused field only listens for Enter.
        if self.ui.dialog_open() && !self.ui.is_editing_text() {
            if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
                self.ui.dialog_enter();
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

        // The Files panel, after a click in it, takes F2 / Delete / arrows.
        if self.ui.files_focused() {
            use crate::ui::project_panel::FilesKey;
            let key = match code {
                KeyCode::ArrowUp => Some(FilesKey::Up),
                KeyCode::ArrowDown => Some(FilesKey::Down),
                KeyCode::ArrowLeft => Some(FilesKey::Left),
                KeyCode::ArrowRight => Some(FilesKey::Right),
                KeyCode::Enter | KeyCode::NumpadEnter => Some(FilesKey::Open),
                KeyCode::F2 => Some(FilesKey::Rename),
                KeyCode::Delete => Some(FilesKey::Delete),
                KeyCode::KeyD if ctrl => Some(FilesKey::Duplicate),
                KeyCode::KeyC if ctrl && shift => Some(FilesKey::CopyPath),
                _ => None,
            };
            if let Some(key) = key {
                let editing = self.ui.play == PlayState::Editing;
                if let Some(intent) = self.ui.files_key(key, editing) {
                    self.apply(intent);
                }
                return;
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
            (KeyCode::KeyD, true) if shift => {
                if let Some(id) = selected {
                    self.ui.open_duplicate_along(&self.doc, id);
                }
                None
            }
            (KeyCode::KeyD, true) => selected.map(Intent::Duplicate),
            (KeyCode::KeyA, true) => Some(Intent::SelectMany(
                self.doc.entities.iter().map(|e| e.id).collect(),
            )),
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
            let root = self.python_project();
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
        let t = Instant::now();
        self.tick_scripts(dt);
        self.ui.profiler.since("Scripts: talking to Python", t);
        let t = Instant::now();
        self.poll_build();
        self.ui.profiler.since("Build: checking the job", t);
        let t = Instant::now();
        self.watch_models();
        self.ui.profiler.since("Assets: watching files", t);
        let t = Instant::now();
        let mut physics_ms = 0.0;
        let ms = dt * 1000.0;
        let prev = self.ui.stats.frame_ms;
        self.ui.stats.frame_ms = if prev <= 0.0 || ms > 250.0 {
            ms.min(250.0)
        } else {
            prev + (ms - prev) * 0.08
        };
        if let Some(r) = &mut self.renderer {
            // A game's own graphics setting wins while it plays.
            let quality = self
                .play
                .as_ref()
                .and_then(|p| p.graphics)
                .unwrap_or(self.ui.prefs.graphics_quality);
            if r.quality() != quality {
                r.set_quality(quality);
            }
            if r.vsync() != self.ui.prefs.vsync {
                r.set_vsync(self.ui.prefs.vsync);
            }
        }
        self.ui.profiler.since("Graphics settings", t);
        if let Some(play) = &mut self.play {
            // With a scene camera, movement is relative to it and scripts
            // drive the view; otherwise the editor camera follows the player.
            let scene_cam = play.scene_camera();
            let yaw = scene_cam.map_or(self.ui.camera.yaw, |c| ViewCamera::from_camera3d(&c).yaw);
            play.input = self.input.play_input(yaw);
            let t = Instant::now();
            play.update(dt);
            physics_ms = t.elapsed().as_secs_f32() * 1000.0;
        }
        if self.play.is_some() {
            let root = self.doc.base_dir();
            let engine = self.audio.get_or_insert_with(AudioEngine::new);
            let messages = self
                .play
                .as_mut()
                .map(|p| p.update_audio(engine, &root))
                .unwrap_or_default();
            engine.pump_frame(f64::from(dt));
            for m in messages {
                self.ui.log(LogLevel::Warn, m);
            }
        }
        if let Some(play) = &mut self.play {
            let scene_cam = play.scene_camera();
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

        // `composited`: where the GPU should show the scene view this frame
        // (physical top-left corner), when it renders without a readback.
        let draw_start = Instant::now();
        // Time spent rendering the scene (and previews) inside the draw.
        let mut scene_ms = 0.0_f32;
        let (intents, composited) = {
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
            // The material editor's preview (its own little world).
            if let Some((path, shape, pw, ph)) = ui.material_preview_request(s) {
                let t = Instant::now();
                let img = viewport.render_material(renderer.as_mut(), &path, shape, pw, ph);
                ui.set_material_preview_image(img);
                ui.profiler.since("Material preview", t);
            }
            match play {
                // A script tab covers the scene view: skip the 3D render.
                Some(session) if !ui.scene_visible() => {
                    (ui.frame(canvas, &session.doc, pointer, None), None)
                }
                None if !ui.scene_visible() => (ui.frame(canvas, doc, pointer, None), None),
                Some(session) => match renderer.as_mut() {
                    // The GPU path: the scene stays on the GPU and is
                    // composited under the UI at present time.
                    Some(r) => {
                        let t = Instant::now();
                        viewport.render_play_gpu(r, session, cam, vw, vh);
                        scene_ms += t.elapsed().as_secs_f32() * 1000.0;
                        for e in viewport.take_hud_font_errors() {
                            ui.log(LogLevel::Warn, format!("{e}; using the built-in font"));
                        }
                        for c in viewport.take_hud_missing_chars() {
                            ui.log(
                                LogLevel::Warn,
                                automata_renderer::text::missing_glyph_message(c),
                            );
                        }
                        ui.frame_composited(canvas, &session.doc, pointer, (vw, vh))
                    }
                    None => {
                        let t = Instant::now();
                        let (frame, fw, fh) = viewport.render_play(None, session, cam, vw, vh);
                        scene_ms += t.elapsed().as_secs_f32() * 1000.0;
                        let intents =
                            ui.frame(canvas, &session.doc, pointer, Some((frame, fw, fh)));
                        (intents, None)
                    }
                },
                None => {
                    let t = Instant::now();
                    viewport.tick_particles(dt);
                    // The Inspector's animation preview follows the
                    // selection, and plays without pressing Play.
                    if ui.anim_preview.is_some_and(|p| Some(p.id) != doc.selected) {
                        ui.anim_preview = None;
                    }
                    if let Some(r) = renderer.as_mut() {
                        viewport.sync(r, doc);
                        viewport.preview_animation(doc, ui.anim_preview.as_mut(), dt);
                        // Attached entities are picked and moved where
                        // their bones hold them.
                        let frames = viewport.attach_frames(doc);
                        doc.set_attach_frames(frames);
                    }
                    // The foliage brush paints on what's under the cursor.
                    let hit = ui.brush_target(doc).and_then(|_| {
                        let proj = crate::view::Projector::new(&ui.camera, view);
                        let (origin, dir) = proj.ray(pointer.x, pointer.y);
                        viewport.surface_under(origin, dir)
                    });
                    ui.set_brush_hit(hit);
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
                    match renderer.as_mut() {
                        Some(r) => {
                            viewport.render_gpu(r, doc, cam, vw, vh);
                            scene_ms += t.elapsed().as_secs_f32() * 1000.0;
                            ui.frame_composited(canvas, doc, pointer, (vw, vh))
                        }
                        None => {
                            let (frame, fw, fh) = viewport.render(None, doc, cam, vw, vh);
                            scene_ms += t.elapsed().as_secs_f32() * 1000.0;
                            (ui.frame(canvas, doc, pointer, Some((frame, fw, fh))), None)
                        }
                    }
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
        let t = Instant::now();
        for intent in intents {
            self.apply(intent);
        }
        self.ui.profiler.since("Edits: applying", t);
        let t = Instant::now();
        self.handle_code_requests();
        self.ui.profiler.since("Code: language server", t);
        let t = Instant::now();
        if self.ui.take_prefs_dirty() {
            self.save_prefs();
        }
        self.ui.profiler.since("Preferences: saving", t);

        let draw_ms = draw_start.elapsed().as_secs_f32() * 1000.0;
        let present = Instant::now();
        if let (Some(renderer), Some(canvas)) = (&mut self.renderer, &self.canvas) {
            let scene =
                composited.and_then(|origin| Some((self.viewport.scene_texture()?, origin)));
            renderer.present_composited(canvas.pixels(), canvas.size(), scene);
        }
        let present_ms = present.elapsed().as_secs_f32() * 1000.0;
        self.ui
            .profiler
            .add("Present (includes waiting for the display)", present_ms);
        self.ui.profiler.add("Scene render", scene_ms);
        self.ui.profiler.record(crate::ui::profiler::FrameSample {
            scripts: if self.play.is_some() {
                self.last_script_ms
            } else {
                0.0
            },
            physics: physics_ms,
            render: scene_ms + present_ms,
            ui: (draw_ms - scene_ms).max(0.0),
            total: ms.min(1000.0),
        });
        let t = Instant::now();
        self.update_window_chrome(&window);
        self.autosave();
        self.ui.profiler.since("Window title, autosave", t);
        self.ui.profiler.end_frame();
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
                self.ui.prefs.vsync,
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
            WindowEvent::HoveredFile(_) => self.ui.os_file_hover = true,
            WindowEvent::HoveredFileCancelled => self.ui.os_file_hover = false,
            WindowEvent::DroppedFile(path) => {
                self.ui.os_file_hover = false;
                if !self.ui.file_dialog_drop(&path) {
                    self.dropped_file(&path);
                }
            }
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
                    // Never keep the mouse captured while another window
                    // is in front; click the view to capture it again.
                    self.set_cursor_lock(false);
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
                let at = (position.x as f32, position.y as f32);
                if !self.lock_active && self.play.is_some() {
                    // Uncaptured, scripts see the cursor's own movement
                    // (captured, raw device motion arrives instead).
                    self.input.mouse_delta.0 += at.0 - self.input.cursor.0;
                    self.input.mouse_delta.1 += at.1 - self.input.cursor.1;
                }
                if !self.lock_active {
                    self.input.cursor = at;
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                if self.game_mouse_button(button, down) {
                    // A captured mouse belongs to the game alone.
                    self.request_redraw();
                    return;
                }
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

    fn device_event(&mut self, _loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.lock_active {
                self.input.mouse_delta.0 += delta.0 as f32;
                self.input.mouse_delta.1 += delta.1 as f32;
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            self.save_prefs();
            event_loop.exit();
            return;
        }
        // Redraw continuously so play mode, hover and drags stay live. V-Sync
        // (when on) paces frames to the display; the optional FPS limit
        // and the background throttle space them out further.
        let background = self.window.as_ref().is_some_and(|w| {
            w.is_minimized() == Some(true) || (!w.has_focus() && self.play.is_none())
        });
        match frame_interval(self.ui.prefs.fps_limit, background) {
            Some(interval) if Instant::now() < self.last_frame + interval => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_frame + interval));
            }
            _ => {
                self.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }
}

/// The minimum time between frames: `fps_limit` (0 = none), or 10 FPS
/// while the editor sits in the background (unfocused and not playing, or
/// minimized) so it doesn't spend a core or the GPU on an unseen window.
fn frame_interval(fps_limit: u32, background: bool) -> Option<Duration> {
    if background {
        return Some(Duration::from_millis(100));
    }
    (fps_limit > 0).then(|| Duration::from_secs_f64(1.0 / f64::from(fps_limit)))
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

/// Where entity transform `t` goes when the lead of a group selection moves
/// from `before` to `after`: moved by the same offset, turned by the same
/// rotation about the lead's pivot, and scaled by the same factors.
fn group_transform(
    before: &TransformData,
    after: &TransformData,
    t: &TransformData,
) -> TransformData {
    use glam::{EulerRot, Quat, Vec3};

    let quat = |e: [f32; 3]| Quat::from_euler(EulerRot::XYZ, e[0], e[1], e[2]);
    let turn = quat(after.rotation_euler) * quat(before.rotation_euler).inverse();
    let pivot = Vec3::from(before.position);
    let offset = Vec3::from(t.position) - pivot;
    // A plain move leaves each rotation exactly as it was.
    let still = turn.angle_between(Quat::IDENTITY) < 1e-6;
    let position = if still {
        Vec3::from(after.position) + offset
    } else {
        Vec3::from(after.position) + turn * offset
    };
    let (ex, ey, ez) = if still {
        (
            t.rotation_euler[0],
            t.rotation_euler[1],
            t.rotation_euler[2],
        )
    } else {
        (turn * quat(t.rotation_euler)).to_euler(EulerRot::XYZ)
    };
    let mut scale = t.scale;
    for (i, s) in scale.iter_mut().enumerate() {
        if before.scale[i].abs() > 1e-6 {
            *s *= after.scale[i] / before.scale[i];
        }
    }
    TransformData {
        position: position.to_array(),
        rotation_euler: [ex, ey, ez],
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_pacing_is_uncapped_unless_limited_or_in_the_background() {
        assert_eq!(frame_interval(0, false), None);
        assert_eq!(
            frame_interval(120, false),
            Some(Duration::from_secs_f64(1.0 / 120.0))
        );
        assert_eq!(frame_interval(0, true), Some(Duration::from_millis(100)));
        assert_eq!(frame_interval(240, true), Some(Duration::from_millis(100)));
    }

    #[test]
    fn a_group_follows_its_lead() {
        let t = |p: [f32; 3], yaw: f32, s: f32| TransformData {
            position: p,
            rotation_euler: [0.0, yaw, 0.0],
            scale: [s, s, s],
        };
        let lead = t([0.0, 0.0, 0.0], 0.0, 1.0);
        let other = t([2.0, 0.0, 0.0], 0.3, 2.0);
        // A move carries the others by the same offset, rotation untouched.
        let moved = group_transform(&lead, &t([1.0, 0.5, 0.0], 0.0, 1.0), &other);
        assert_eq!(moved.position, [3.0, 0.5, 0.0]);
        assert_eq!(moved.rotation_euler, other.rotation_euler);
        // A quarter turn swings them around the lead.
        let quarter = std::f32::consts::FRAC_PI_2;
        let turned = group_transform(&lead, &t([0.0, 0.0, 0.0], quarter, 1.0), &other);
        assert!((turned.position[0]).abs() < 1e-5 && (turned.position[2] + 2.0).abs() < 1e-5);
        let r = turned.rotation_euler;
        let q = glam::Quat::from_euler(glam::EulerRot::XYZ, r[0], r[1], r[2]);
        assert!(q.angle_between(glam::Quat::from_rotation_y(0.3 + quarter)) < 1e-4);
        // Scaling the lead ×2 scales the others ×2.
        let scaled = group_transform(&lead, &t([0.0, 0.0, 0.0], 0.0, 2.0), &other);
        assert_eq!(scaled.scale, [4.0, 4.0, 4.0]);
    }
}
