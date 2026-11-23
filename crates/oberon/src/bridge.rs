//! The Python script bridge: Play mode runs behaviour scripts out of process.
//!
//! Oberon never embeds a Python interpreter. When a scene with scripts
//! enters Play mode, it starts the project's `python3` on a small runner
//! (`bridge.py`, compiled into the editor and written to
//! `<project>/.oberon/bridge.py` next to `_automata_script_api.py`, the
//! script API shared with `automata.run_scene`) and exchanges one JSON
//! object per line:
//!
//! * Oberon → runner: `frame` (time, held keys, every entity's transform,
//!   character velocity, grounded flag, attributes and script path), and
//!   `stop`.
//! * runner → Oberon: `ready`, `log` (`print` output), `error` (with the
//!   script file and line, so the Console can jump there), `reloaded` (hot
//!   reload after a save) and `frame` (the writes scripts made).
//!
//! Frames are pipelined, never awaited: Oberon sends the next frame only
//! after the previous reply arrived and applies replies as they come, so a
//! slow script lowers the script tick rate instead of freezing the editor.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use automata_core::AttributeValue;

use crate::json::Json;

/// The runner script (pure standard-library Python).
pub const RUNNER: &str = include_str!("bridge.py");

/// The behaviour-script API (`entity`, `ctx`, loading, errors), the same
/// module the engine's `run_scene` uses. The runner imports it from its own
/// folder, so Play needs no installed packages.
pub const SCRIPT_API: &str = include_str!("../../automata-python/python/automata/script_api.py");

/// The file name [`SCRIPT_API`] is written under, next to the runner.
pub const SCRIPT_API_FILE: &str = "_automata_script_api.py";

/// The default interpreter when the preferences don't name one.
#[must_use]
pub fn default_python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

/// One entity's state, sent to scripts every frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityState {
    /// Document entity id (stable for the session).
    pub id: u64,
    /// Entity name.
    pub name: String,
    /// Absolute path of the attached script, if any.
    pub script: Option<PathBuf>,
    /// Local position.
    pub position: [f32; 3],
    /// Euler rotation (XYZ, radians).
    pub rotation: [f32; 3],
    /// Scale.
    pub scale: [f32; 3],
    /// Character velocity, for character-controlled entities.
    pub velocity: Option<[f32; 3]>,
    /// Whether a character stands on something.
    pub grounded: bool,
    /// Custom attributes.
    pub attributes: BTreeMap<String, AttributeValue>,
    /// The entity's camera as `[eye xyz, target xyz, fov°]`, if it has one.
    pub camera: Option<[f32; 7]>,
    /// Whether the entity has an Audio Source.
    pub has_audio: bool,
    /// An animated model's playback, if the entity shows one.
    pub anim: Option<automata_scene::AnimationState>,
    /// The entity's joint and how it's holding, if it has one.
    pub joint: Option<(automata_physics::Joint, automata_physics::JointState)>,
}

/// A collider as the runner sees it (for `ctx.physics.raycast`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColliderShape {
    /// A box: entity id, centre, rotation `[x, y, z, w]`, half extents.
    Cuboid {
        /// Document entity id.
        id: u64,
        /// World centre.
        center: [f32; 3],
        /// World rotation quaternion.
        rotation: [f32; 4],
        /// Half extents.
        half: [f32; 3],
    },
    /// A sphere.
    Ball {
        /// Document entity id.
        id: u64,
        /// World centre.
        center: [f32; 3],
        /// Radius.
        radius: f32,
    },
    /// A capsule along its local Y axis.
    Capsule {
        /// Document entity id.
        id: u64,
        /// World centre.
        center: [f32; 3],
        /// World rotation quaternion.
        rotation: [f32; 4],
        /// Half the straight part's length.
        half_height: f32,
        /// Radius.
        radius: f32,
    },
    /// Triangles (a mesh collider or a convex hull), shared by key: the
    /// geometry itself travels once per session in [`FrameInput::meshes`].
    Mesh {
        /// Document entity id.
        id: u64,
        /// World position of the triangles' origin.
        center: [f32; 3],
        /// World rotation quaternion.
        rotation: [f32; 4],
        /// Key of the geometry in [`FrameInput::meshes`].
        mesh: u64,
    },
}

/// Collision triangles in a collider's local space.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshTris {
    /// Vertex positions.
    pub vertices: Vec<[f32; 3]>,
    /// Triangles as vertex indices.
    pub triangles: Vec<[u32; 3]>,
}

/// Everything besides entities that a frame tells scripts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameInput {
    /// Mouse buttons held (`"left"`, `"right"`, `"middle"`).
    pub mouse_buttons: Vec<&'static str>,
    /// Mouse movement since the last frame (pixels, +y down).
    pub mouse_delta: [f32; 2],
    /// Whether the cursor is captured for mouse-look.
    pub mouse_locked: bool,
    /// The game view's size in pixels (for the HUD).
    pub screen: [u32; 2],
    /// Solid colliders, for raycasts.
    pub colliders: Vec<ColliderShape>,
    /// The triangles behind every [`ColliderShape::Mesh`] key in
    /// `colliders`. The bridge sends each key's geometry only once.
    pub meshes: Vec<(u64, Arc<MeshTris>)>,
    /// Entities whose Audio Source is playing.
    pub playing: Vec<u64>,
    /// Water surfaces at the last physics step, packed by
    /// [`automata_renderer::WaterSurface::pack`] (for `water_height`).
    pub water: Vec<Vec<f32>>,
    /// The graphics preset Play renders at (`"high"`), for
    /// `ctx.settings.graphics`; empty when unknown.
    pub graphics: &'static str,
}

/// What a script asked of an entity's Audio Source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioWrite {
    /// Start (or restart) it.
    Play,
    /// Stop it.
    Stop,
}

/// A one-shot sound requested by `ctx.audio.play`.
#[derive(Debug, Clone, PartialEq)]
pub struct OneShot {
    /// The file, relative to the scene folder.
    pub path: String,
    /// Volume (1 = as recorded).
    pub volume: f32,
    /// Pitch / speed (1 = as recorded).
    pub pitch: f32,
    /// Where it plays (fades with distance), or `None` for everywhere.
    pub position: Option<[f32; 3]>,
}

/// What a HUD item draws.
#[derive(Debug, Clone, PartialEq)]
pub enum HudKind {
    /// An image file (relative to the scene folder).
    Image(String),
    /// A filled rectangle.
    Rect,
    /// A string in the built-in font, anchored at the item's `x`, `y`.
    Text {
        /// The string (`\n` starts a new line).
        text: String,
        /// Line height in pixels.
        size: f32,
        /// Where `x` sits on each line.
        align: automata_renderer::TextAlign,
        /// Drop-shadow colour.
        shadow: Option<[f32; 4]>,
        /// Outline colour.
        outline: Option<[f32; 4]>,
    },
}

/// One thing `ctx.hud` drew this frame (pixels, top-left origin, placed by
/// its centre).
#[derive(Debug, Clone, PartialEq)]
pub struct HudItem {
    /// Image or rectangle.
    pub kind: HudKind,
    /// Centre x.
    pub x: f32,
    /// Centre y.
    pub y: f32,
    /// Size; `None` for an image's own size.
    pub size: Option<[f32; 2]>,
    /// RGBA tint.
    pub color: [f32; 4],
    /// Rotation in radians.
    pub rotation: f32,
}

/// The runner's reply to one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameReply {
    /// Entity changes.
    pub writes: Vec<EntityWrite>,
    /// One-shot sounds to start.
    pub sounds: Vec<OneShot>,
    /// The HUD to draw until the next reply.
    pub hud: Vec<HudItem>,
    /// A cursor lock request.
    pub lock: Option<bool>,
    /// Splashes in the water (`ctx.water.splash`): position and strength.
    pub splashes: Vec<([f32; 3], f32)>,
    /// One-off particle effects (`ctx.particles.burst`).
    pub bursts: Vec<BurstRequest>,
    /// Extra particles from entities' own emitters (`ctx.particles.emit`):
    /// document entity id and count.
    pub emits: Vec<(u64, u32)>,
    /// How long the scripts took to run this frame, in milliseconds (as
    /// the runner measured it, without the trip over the pipe).
    pub script_ms: Option<f32>,
    /// Player settings to apply now (`ctx.settings.volume`, `.graphics`).
    pub settings: PlayerSettings,
}

/// Player settings a script changed (or that were loaded from the
/// project's `.oberon/save/settings.json` at the start).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerSettings {
    /// The master volume, 0 to 1.
    pub volume: Option<f32>,
    /// The graphics preset.
    pub graphics: Option<automata_renderer::GraphicsQuality>,
}

/// A graphics preset's script name (`"high"`), as `ctx.settings.graphics`
/// spells it.
#[must_use]
pub fn quality_key(quality: automata_renderer::GraphicsQuality) -> &'static str {
    use automata_renderer::GraphicsQuality as Q;
    match quality {
        Q::Low => "low",
        Q::Medium => "medium",
        Q::High => "high",
        Q::Ultra => "ultra",
        Q::Beautiful => "beautiful",
    }
}

fn parse_settings(v: Option<&Json>) -> PlayerSettings {
    let Some(v) = v else {
        return PlayerSettings::default();
    };
    #[allow(clippy::cast_possible_truncation)]
    PlayerSettings {
        volume: v
            .get("volume")
            .and_then(Json::as_f64)
            .filter(|x| x.is_finite())
            .map(|x| x.clamp(0.0, 1.0) as f32),
        graphics: v.get("graphics").and_then(Json::as_str).and_then(|name| {
            automata_renderer::GraphicsQuality::ALL
                .into_iter()
                .find(|q| quality_key(*q) == name)
        }),
    }
}

/// A one-off particle effect a script asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct BurstRequest {
    /// The effect, preset and changed fields applied.
    pub emitter: automata_scene::ParticleEmitter,
    /// Where (world space).
    pub position: [f32; 3],
    /// Which way it flies instead of the effect's up, if given.
    pub direction: Option<[f32; 3]>,
    /// How many particles, if not the effect's own burst.
    pub count: Option<u32>,
}

/// What a script changed on one entity.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityWrite {
    /// Document entity id.
    pub id: u64,
    /// New position.
    pub position: Option<[f32; 3]>,
    /// New Euler rotation (radians).
    pub rotation: Option<[f32; 3]>,
    /// New scale.
    pub scale: Option<[f32; 3]>,
    /// New character velocity.
    pub velocity: Option<[f32; 3]>,
    /// The full new attribute map.
    pub attributes: Option<BTreeMap<String, AttributeValue>>,
    /// New camera `[eye xyz, target xyz, fov°]`.
    pub camera: Option<[f32; 7]>,
    /// Play or stop the entity's Audio Source.
    pub audio: Option<AudioWrite>,
    /// Requests to the entity's animation, in call order.
    pub anim: Vec<automata_scene::AnimationCommand>,
    /// Changes to the entity's joint.
    pub joint: Option<automata_physics::JointEdit>,
    /// Remove the entity from the running scene.
    pub despawn: bool,
}

/// A message from the runner.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeMsg {
    /// The runner started.
    Ready {
        /// Interpreter version, e.g. `3.12.1`.
        python: String,
    },
    /// Script output (`print`) or a runner notice.
    Log {
        /// `info`, `warn` or `error`.
        level: String,
        /// The line.
        msg: String,
    },
    /// A script raised.
    Error {
        /// One-line summary.
        msg: String,
        /// The script file.
        file: PathBuf,
        /// 1-based line in `file` (0 when unknown).
        line: u32,
        /// The (user-frames-only) traceback.
        traceback: String,
    },
    /// A script file was hot-reloaded.
    Reloaded(PathBuf),
    /// The scripts' writes (and sounds, HUD, cursor lock) for one frame.
    Frame(FrameReply),
    /// The runner finished its stop hooks and is exiting.
    Bye,
    /// The runner exited (or could not be talked to).
    Exited(String),
}

enum Event {
    Line(String),
    Stderr(String),
    Eof,
}

/// A running script runner process.
pub struct ScriptBridge {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Event>,
    in_flight: Option<Instant>,
    frame: u64,
    exited: bool,
    /// Mesh keys whose triangles the runner already has.
    sent_meshes: HashSet<u64>,
    /// Frames answered (for the status readout).
    pub answered: u64,
}

impl ScriptBridge {
    /// Writes the runner into `runtime_dir` and starts `python` on it with
    /// `cwd` as the working directory (the scene's folder, so relative
    /// paths in scripts behave as in the game).
    ///
    /// # Errors
    ///
    /// Fails when the runner can't be written or the interpreter can't be
    /// started (not installed / wrong path in preferences).
    pub fn start(python: &str, runtime_dir: &Path, cwd: &Path) -> std::io::Result<Self> {
        // The interpreter runs in `cwd`, so a relative runner path would be
        // looked up from there instead of from here.
        let runtime_dir = &std::path::absolute(runtime_dir)?;
        std::fs::create_dir_all(runtime_dir)?;
        let runner = runtime_dir.join("bridge.py");
        for (path, text) in [
            (runner.clone(), RUNNER),
            (runtime_dir.join(SCRIPT_API_FILE), SCRIPT_API),
        ] {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
                std::fs::write(&path, text)?;
            }
        }
        let mut child = Command::new(python)
            .arg("-u")
            .arg(&runner)
            .current_dir(cwd)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (tx, rx) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(Event::Line(line)).is_err() {
                        return;
                    }
                }
                let _ = tx.send(Event::Eof);
            });
        }
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(Event::Stderr(line)).is_err() {
                        return;
                    }
                }
            });
        }
        let stdin = child.stdin.take();
        Ok(Self {
            child,
            stdin,
            rx,
            in_flight: None,
            frame: 0,
            exited: false,
            sent_meshes: HashSet::new(),
            answered: 0,
        })
    }

    /// Whether a frame is awaiting its reply.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// How long the current frame has been awaiting its reply.
    #[must_use]
    pub fn waiting_for(&self) -> Option<Duration> {
        self.in_flight.map(|t| t.elapsed())
    }

    /// Whether the runner process is gone.
    #[must_use]
    pub fn exited(&self) -> bool {
        self.exited
    }

    fn send(&mut self, msg: &Json) -> bool {
        let Some(stdin) = &mut self.stdin else {
            return false;
        };
        let mut line = msg.dump();
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .is_ok()
    }

    /// Sends a frame unless the previous one is still being processed.
    /// Returns whether it was sent.
    pub fn send_frame(
        &mut self,
        dt: f32,
        time: f32,
        keys: &[&str],
        entities: &[EntityState],
    ) -> bool {
        self.send_frame_with(dt, time, keys, entities, &FrameInput::default())
    }

    /// [`send_frame`](Self::send_frame) with mouse, colliders, playing
    /// sounds and the screen size.
    pub fn send_frame_with(
        &mut self,
        dt: f32,
        time: f32,
        keys: &[&str],
        entities: &[EntityState],
        input: &FrameInput,
    ) -> bool {
        if self.busy() || self.exited {
            return false;
        }
        self.frame += 1;
        #[allow(clippy::cast_possible_wrap)]
        let mut msg = Json::obj([
            ("op", Json::from("frame")),
            ("frame", Json::Int(self.frame as i64)),
            ("dt", Json::Float(f64::from(dt))),
            ("time", Json::Float(f64::from(time))),
            (
                "keys",
                Json::Arr(keys.iter().map(|k| Json::from(*k)).collect()),
            ),
            (
                "entities",
                Json::Arr(entities.iter().map(state_json).collect()),
            ),
        ]);
        frame_input_json(&mut msg, input);
        let new_meshes: Vec<Json> = input
            .meshes
            .iter()
            .filter(|(key, _)| self.sent_meshes.insert(*key))
            .map(|(key, tris)| mesh_json(*key, tris))
            .collect();
        if !new_meshes.is_empty() {
            msg.set("meshes", Json::Arr(new_meshes));
        }
        if self.send(&msg) {
            self.in_flight = Some(Instant::now());
            true
        } else {
            false
        }
    }

    /// Drains the runner's messages.
    pub fn poll(&mut self) -> Vec<BridgeMsg> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(event) => self.handle(event, &mut out),
                Err(TryRecvError::Disconnected) => {
                    self.handle(Event::Eof, &mut out);
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
            if self.exited {
                break;
            }
        }
        out
    }

    /// Like [`poll`](Self::poll), but first blocks up to `timeout` for the
    /// runner to say something (for tools that run frames in lock-step).
    pub fn wait(&mut self, timeout: Duration) -> Vec<BridgeMsg> {
        let mut out = Vec::new();
        match self.rx.recv_timeout(timeout) {
            Ok(event) => self.handle(event, &mut out),
            Err(mpsc::RecvTimeoutError::Disconnected) => self.handle(Event::Eof, &mut out),
            Err(mpsc::RecvTimeoutError::Timeout) => return out,
        }
        if !self.exited {
            out.extend(self.poll());
        }
        out
    }

    fn handle(&mut self, event: Event, out: &mut Vec<BridgeMsg>) {
        match event {
            Event::Line(line) => {
                if let Some(msg) = parse_msg(&line) {
                    if matches!(msg, BridgeMsg::Frame(_)) {
                        self.in_flight = None;
                        self.answered += 1;
                    }
                    out.push(msg);
                } else if !line.trim().is_empty() {
                    // Something wrote to the real stdout (e.g. a C
                    // extension); show it rather than lose it.
                    out.push(BridgeMsg::Log {
                        level: "info".into(),
                        msg: line,
                    });
                }
            }
            Event::Stderr(line) => out.push(BridgeMsg::Log {
                level: "error".into(),
                msg: line,
            }),
            Event::Eof => {
                if !self.exited {
                    self.exited = true;
                    self.in_flight = None;
                    let status = self
                        .child
                        .try_wait()
                        .ok()
                        .flatten()
                        .map_or_else(|| "closed".to_string(), |s| s.to_string());
                    out.push(BridgeMsg::Exited(status));
                }
            }
        }
    }

    /// Asks the runner to call `stop` hooks and exit, then makes sure it
    /// is gone. Returns the messages it sent while stopping.
    #[must_use]
    pub fn stop(mut self) -> Vec<BridgeMsg> {
        let _ = self.send(&Json::obj([("op", Json::from("stop"))]));
        self.stdin = None; // EOF also ends the runner's read loop
        let deadline = Instant::now() + Duration::from_millis(400);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(5));
        let mut out = self.poll();
        out.retain(|m| {
            !matches!(
                m,
                BridgeMsg::Exited(_) | BridgeMsg::Frame(_) | BridgeMsg::Bye
            )
        });
        out
    }
}

impl Drop for ScriptBridge {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// An attribute value as JSON (vectors and colors become arrays).
#[must_use]
pub fn attr_json(value: &AttributeValue) -> Json {
    match value {
        AttributeValue::Bool(b) => Json::Bool(*b),
        AttributeValue::Int(i) => Json::Int(*i),
        AttributeValue::Float(f) => Json::Float(*f),
        AttributeValue::Text(s) => Json::Str(s.clone()),
        AttributeValue::Vec3(v) => Json::floats(v),
        AttributeValue::Color(c) => Json::floats(c),
    }
}

/// An attribute value from JSON (`None` for unsupported shapes).
#[must_use]
pub fn attr_from_json(value: &Json) -> Option<AttributeValue> {
    Some(match value {
        Json::Bool(b) => AttributeValue::Bool(*b),
        Json::Int(i) => AttributeValue::Int(*i),
        Json::Float(f) => AttributeValue::Float(*f),
        Json::Str(s) => AttributeValue::Text(s.clone()),
        Json::Arr(a) if a.len() == 3 => AttributeValue::Vec3(value.as_floats::<3>()?),
        Json::Arr(a) if a.len() == 4 => AttributeValue::Color(value.as_floats::<4>()?),
        _ => return None,
    })
}

#[allow(clippy::cast_possible_wrap)]
fn collider_json(c: &ColliderShape) -> Json {
    let f = |v: f32| Json::Float(f64::from(v));
    let mut out = Vec::with_capacity(12);
    match *c {
        ColliderShape::Cuboid {
            id,
            center,
            rotation,
            half,
        } => {
            out.push(Json::Int(id as i64));
            out.push(Json::from("box"));
            out.extend(center.into_iter().chain(rotation).chain(half).map(f));
        }
        ColliderShape::Ball { id, center, radius } => {
            out.push(Json::Int(id as i64));
            out.push(Json::from("ball"));
            out.extend(center.into_iter().chain([radius]).map(f));
        }
        ColliderShape::Capsule {
            id,
            center,
            rotation,
            half_height,
            radius,
        } => {
            out.push(Json::Int(id as i64));
            out.push(Json::from("capsule"));
            out.extend(
                center
                    .into_iter()
                    .chain(rotation)
                    .chain([half_height, radius])
                    .map(f),
            );
        }
        ColliderShape::Mesh {
            id,
            center,
            rotation,
            mesh,
        } => {
            out.push(Json::Int(id as i64));
            out.push(Json::from("mesh"));
            out.extend(center.into_iter().chain(rotation).map(f));
            out.push(Json::Int(mesh as i64));
        }
    }
    Json::Arr(out)
}

/// One mesh's geometry for the runner: `[key, [x, y, z, ...], [i, j, k, ...]]`.
#[allow(clippy::cast_possible_wrap)]
fn mesh_json(key: u64, tris: &MeshTris) -> Json {
    let verts: Vec<f32> = tris.vertices.iter().flatten().copied().collect();
    Json::Arr(vec![
        Json::Int(key as i64),
        Json::floats(&verts),
        Json::Arr(
            tris.triangles
                .iter()
                .flatten()
                .map(|&i| Json::Int(i64::from(i)))
                .collect(),
        ),
    ])
}

#[allow(clippy::cast_possible_wrap)]
fn frame_input_json(msg: &mut Json, input: &FrameInput) {
    if !input.graphics.is_empty() {
        msg.set("gfx", Json::from(input.graphics));
    }
    msg.set(
        "mouse",
        Json::obj([
            (
                "buttons",
                Json::Arr(input.mouse_buttons.iter().map(|b| Json::from(*b)).collect()),
            ),
            ("delta", Json::floats(&input.mouse_delta)),
            ("locked", Json::Bool(input.mouse_locked)),
        ]),
    );
    msg.set(
        "screen",
        Json::Arr(
            input
                .screen
                .iter()
                .map(|&v| Json::Int(i64::from(v)))
                .collect(),
        ),
    );
    if !input.colliders.is_empty() {
        msg.set(
            "cols",
            Json::Arr(input.colliders.iter().map(collider_json).collect()),
        );
    }
    if !input.water.is_empty() {
        msg.set(
            "water",
            Json::Arr(input.water.iter().map(|w| Json::floats(w)).collect()),
        );
    }
    if !input.playing.is_empty() {
        msg.set(
            "playing",
            Json::Arr(
                input
                    .playing
                    .iter()
                    .map(|&id| Json::Int(id as i64))
                    .collect(),
            ),
        );
    }
}

fn state_json(e: &EntityState) -> Json {
    #[allow(clippy::cast_possible_wrap)]
    let mut obj = Json::obj([
        ("id", Json::Int(e.id as i64)),
        ("name", Json::from(e.name.as_str())),
        ("p", Json::floats(&e.position)),
        ("r", Json::floats(&e.rotation)),
        ("s", Json::floats(&e.scale)),
        ("g", Json::Bool(e.grounded)),
        (
            "attrs",
            Json::Obj(
                e.attributes
                    .iter()
                    .map(|(k, v)| (k.clone(), attr_json(v)))
                    .collect(),
            ),
        ),
    ]);
    if let Some(v) = e.velocity {
        obj.set("v", Json::floats(&v));
    }
    if let Some(c) = e.camera {
        obj.set("cam", Json::floats(&c));
    }
    if let Some(script) = &e.script {
        obj.set("script", Json::from(script.display().to_string()));
    }
    if e.has_audio {
        obj.set("snd", Json::Bool(true));
    }
    if let Some(a) = &e.anim {
        obj.set("anim", anim_json(a));
    }
    if let Some((joint, state)) = &e.joint {
        obj.set("joint", joint_json(joint, state));
    }
    obj
}

/// `{"kind", "to", "on", "value", "attached", "broken", "force", "motor",
/// "limits", "bf"}` (see the script API's frame model).
fn joint_json(j: &automata_physics::Joint, st: &automata_physics::JointState) -> Json {
    let f = |v: f32| Json::Float(f64::from(v));
    Json::obj([
        ("kind", Json::from(j.kind.key())),
        ("to", j.connected.as_deref().map_or(Json::Null, Json::from)),
        ("on", Json::Bool(j.enabled)),
        ("value", f(st.value)),
        ("attached", Json::Bool(st.attached)),
        ("broken", Json::Bool(st.broken)),
        ("force", f(st.force)),
        (
            "motor",
            j.motor.map_or(Json::Null, |m| {
                Json::Arr(vec![f(m.speed), m.target.map_or(Json::Null, f)])
            }),
        ),
        ("limits", j.limits.map_or(Json::Null, |l| Json::floats(&l))),
        ("bf", f(j.break_force)),
    ])
}

/// One `entity.joint` write: only the keys a script set (`"on"`,
/// `"motor"`: `[speed, target or null]` or null, `"limits"`: `[min, max]`
/// or null, `"bf"`, `"to"`: a name or null).
fn parse_joint(v: &Json) -> Option<automata_physics::JointEdit> {
    v.as_obj()?;
    let finite = |v: &Json| num(v).filter(|x| x.is_finite());
    let edit = automata_physics::JointEdit {
        enabled: v.get("on").and_then(Json::as_bool),
        motor: v.get("motor").map(|m| {
            let a = m.as_arr()?;
            Some((a.first().and_then(finite)?, a.get(1).and_then(finite)))
        }),
        limits: v.get("limits").map(Json::as_floats::<2>),
        break_force: v.get("bf").and_then(finite),
        connected: v.get("to").map(|t| t.as_str().map(str::to_string)),
    };
    (!edit.is_empty()).then_some(edit)
}

/// `{"clip", "time", "len", "playing", "speed", "loop", "clips", "lens",
/// "springs"}`
/// (see the script API's frame model).
fn anim_json(a: &automata_scene::AnimationState) -> Json {
    let events = a.events.iter().map(|e| {
        let seq = Json::Int(i64::try_from(e.seq).unwrap_or(i64::MAX));
        Json::Arr(vec![
            seq,
            Json::from(e.clip.as_str()),
            Json::from(e.name.as_str()),
        ])
    });
    Json::obj([
        ("clip", a.clip.as_deref().map_or(Json::Null, Json::from)),
        ("time", Json::Float(f64::from(a.time))),
        ("len", Json::Float(f64::from(a.duration))),
        ("playing", Json::Bool(a.playing)),
        ("speed", Json::Float(f64::from(a.speed))),
        ("loop", Json::Bool(a.looping)),
        (
            "clips",
            Json::Arr(a.clips.iter().map(|c| Json::from(c.as_str())).collect()),
        ),
        ("lens", Json::floats(&a.lengths)),
        ("springs", Json::Bool(a.springs)),
        ("rm", Json::Bool(a.root_motion)),
        // The latest events passed: [seq, clip, name], oldest first.
        ("ev", Json::Arr(events.collect())),
    ])
}

/// One `entity.animation` request: `["play", clip, loop, fade, speed or
/// null, restart]`, `["stop", fade]`, `["seek", t]`, `["speed", s]`,
/// `["reset_springs"]` or `["root_motion", on]`.
fn parse_anim(v: &Json) -> Option<automata_scene::AnimationCommand> {
    use automata_scene::{AnimationCommand as C, PlayOptions};
    let a = v.as_arr()?;
    let n = |i: usize| a.get(i).and_then(num).filter(|x| x.is_finite());
    Some(match a.first()?.as_str()? {
        "play" => C::Play {
            clip: a.get(1)?.as_str()?.to_string(),
            options: PlayOptions {
                looping: a.get(2).and_then(Json::as_bool).unwrap_or(true),
                fade: n(3).unwrap_or(0.0).max(0.0),
                speed: n(4),
                restart: a.get(5).and_then(Json::as_bool).unwrap_or(false),
            },
        },
        "stop" => C::Stop(n(1).unwrap_or(0.0).max(0.0)),
        "seek" => C::Seek(n(1)?),
        "speed" => C::Speed(n(1)?),
        "reset_springs" => C::ResetSprings,
        "root_motion" => C::RootMotion(a.get(1)?.as_bool()?),
        _ => return None,
    })
}

fn parse_write(w: &Json) -> Option<EntityWrite> {
    let id = u64::try_from(w.get("id")?.as_i64()?).ok()?;
    Some(EntityWrite {
        id,
        position: w.get("p").and_then(Json::as_floats::<3>),
        rotation: w.get("r").and_then(Json::as_floats::<3>),
        scale: w.get("s").and_then(Json::as_floats::<3>),
        velocity: w.get("v").and_then(Json::as_floats::<3>),
        camera: w.get("cam").and_then(Json::as_floats::<7>),
        attributes: w.get("attrs").and_then(Json::as_obj).map(|pairs| {
            pairs
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), attr_from_json(v)?)))
                .collect()
        }),
        audio: match w.get("audio").and_then(Json::as_str) {
            Some("play") => Some(AudioWrite::Play),
            Some("stop") => Some(AudioWrite::Stop),
            _ => None,
        },
        anim: w
            .get("anim")
            .and_then(Json::as_arr)
            .unwrap_or_default()
            .iter()
            .filter_map(parse_anim)
            .collect(),
        joint: w.get("joint").and_then(parse_joint),
        despawn: matches!(w.get("despawn"), Some(Json::Bool(true))),
    })
}

#[allow(clippy::cast_possible_truncation)]
fn num(v: &Json) -> Option<f32> {
    match v {
        Json::Int(i) => Some(*i as f32),
        Json::Float(f) => Some(*f as f32),
        _ => None,
    }
}

fn parse_sound(v: &Json) -> Option<OneShot> {
    let a = v.as_arr()?;
    Some(OneShot {
        path: a.first()?.as_str()?.to_string(),
        volume: a.get(1).and_then(num).unwrap_or(1.0).max(0.0),
        pitch: a.get(2).and_then(num).unwrap_or(1.0).clamp(0.01, 16.0),
        position: a.get(3).and_then(Json::as_floats::<3>),
    })
}

/// `[x, y, z, strength]`.
fn parse_splash(v: &Json) -> Option<([f32; 3], f32)> {
    let a = v.as_arr()?;
    let n = |i: usize| a.get(i).and_then(num).filter(|v| v.is_finite());
    Some(([n(0)?, n(1)?, n(2)?], n(3)?.clamp(0.0, 10.0)))
}

/// `{"e": preset or null, "f": {field: value}, "p": [x, y, z], "d": [x, y, z] or null, "n": count or null}`.
/// Unknown presets or bad fields drop the burst.
fn parse_burst(v: &Json) -> Option<BurstRequest> {
    use automata_scene::{ParticleEmitter, ParticleField};
    let mut emitter = match v.get("e").and_then(Json::as_str) {
        Some(name) => ParticleEmitter::preset(name)?,
        None => ParticleEmitter::default(),
    };
    for (key, value) in v.get("f").and_then(Json::as_obj).unwrap_or_default() {
        #[allow(clippy::cast_possible_truncation)]
        let field = match value {
            Json::Bool(b) => ParticleField::Bool(*b),
            Json::Str(s) => ParticleField::Text(s.clone()),
            Json::Null => ParticleField::Text(String::new()),
            Json::Arr(items) => {
                ParticleField::Numbers(items.iter().map(|x| num(x).unwrap_or(f32::NAN)).collect())
            }
            other => ParticleField::Number(other.as_f64()? as f32),
        };
        emitter.set_field(key, &field).ok()?;
    }
    let finite = |a: [f32; 3]| a.iter().all(|x| x.is_finite()).then_some(a);
    Some(BurstRequest {
        emitter,
        position: finite(v.get("p")?.as_floats::<3>()?)?,
        direction: v.get("d").and_then(Json::as_floats::<3>).and_then(finite),
        count: v
            .get("n")
            .and_then(Json::as_i64)
            .map(|n| u32::try_from(n.clamp(0, 100_000)).unwrap_or(0)),
    })
}

/// `[entity id, count]`.
fn parse_emit(v: &Json) -> Option<(u64, u32)> {
    let a = v.as_arr()?;
    let id = u64::try_from(a.first()?.as_i64()?).ok()?;
    let n = u32::try_from(a.get(1)?.as_i64()?.clamp(0, 100_000)).ok()?;
    Some((id, n))
}

fn parse_hud(v: &Json) -> Option<HudItem> {
    let field = |k: &str| v.get(k).and_then(num);
    let kind = match v.get("k")?.as_str()? {
        "img" => HudKind::Image(v.get("path")?.as_str()?.to_string()),
        "rect" => HudKind::Rect,
        "text" => HudKind::Text {
            text: v.get("text")?.as_str()?.to_string(),
            size: field("size").unwrap_or(24.0),
            align: v
                .get("align")
                .and_then(Json::as_str)
                .and_then(automata_renderer::TextAlign::parse)
                .unwrap_or_default(),
            shadow: v.get("shadow").and_then(Json::as_floats::<4>),
            outline: v.get("outline").and_then(Json::as_floats::<4>),
        },
        _ => return None,
    };
    Some(HudItem {
        kind,
        x: field("x")?,
        y: field("y")?,
        size: field("w").zip(field("h")).map(|(w, h)| [w, h]),
        color: v
            .get("c")
            .and_then(Json::as_floats::<4>)
            .unwrap_or([1.0; 4]),
        rotation: field("rot").unwrap_or(0.0),
    })
}

/// Parses one runner line (`None` for anything that isn't a message).
#[must_use]
pub fn parse_msg(line: &str) -> Option<BridgeMsg> {
    let v = Json::parse(line).ok()?;
    let text = |key: &str| {
        v.get(key)
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Some(match v.get("op")?.as_str()? {
        "ready" => BridgeMsg::Ready {
            python: text("python"),
        },
        "log" => BridgeMsg::Log {
            level: text("level"),
            msg: text("msg"),
        },
        "error" => BridgeMsg::Error {
            msg: text("msg"),
            file: PathBuf::from(text("file")),
            line: v
                .get("line")
                .and_then(Json::as_i64)
                .and_then(|l| u32::try_from(l).ok())
                .unwrap_or(0),
            traceback: text("traceback"),
        },
        "reloaded" => BridgeMsg::Reloaded(PathBuf::from(text("file"))),
        "frame" => {
            let list = |key: &str| v.get(key).and_then(Json::as_arr).unwrap_or_default();
            BridgeMsg::Frame(FrameReply {
                writes: list("writes").iter().filter_map(parse_write).collect(),
                sounds: list("sounds").iter().filter_map(parse_sound).collect(),
                hud: list("hud").iter().filter_map(parse_hud).collect(),
                splashes: list("splashes").iter().filter_map(parse_splash).collect(),
                bursts: list("bursts").iter().filter_map(parse_burst).collect(),
                emits: list("emits").iter().filter_map(parse_emit).collect(),
                lock: match v.get("lock") {
                    Some(Json::Bool(b)) => Some(*b),
                    _ => None,
                },
                #[allow(clippy::cast_possible_truncation)]
                script_ms: v.get("ms").and_then(Json::as_f64).map(|ms| ms as f32),
                settings: parse_settings(v.get("settings")),
            })
        }
        "bye" => BridgeMsg::Bye,
        _ => return None,
    })
}

/// The Python key name (`"a"`, `"space"`, `"arrow_left"` …) of a physical
/// key — the same names the `automata` runtime uses.
#[must_use]
pub fn key_name(code: winit::keyboard::KeyCode) -> Option<&'static str> {
    use winit::keyboard::KeyCode as K;
    Some(match code {
        K::KeyA => "a",
        K::KeyB => "b",
        K::KeyC => "c",
        K::KeyD => "d",
        K::KeyE => "e",
        K::KeyF => "f",
        K::KeyG => "g",
        K::KeyH => "h",
        K::KeyI => "i",
        K::KeyJ => "j",
        K::KeyK => "k",
        K::KeyL => "l",
        K::KeyM => "m",
        K::KeyN => "n",
        K::KeyO => "o",
        K::KeyP => "p",
        K::KeyQ => "q",
        K::KeyR => "r",
        K::KeyS => "s",
        K::KeyT => "t",
        K::KeyU => "u",
        K::KeyV => "v",
        K::KeyW => "w",
        K::KeyX => "x",
        K::KeyY => "y",
        K::KeyZ => "z",
        K::Digit0 => "0",
        K::Digit1 => "1",
        K::Digit2 => "2",
        K::Digit3 => "3",
        K::Digit4 => "4",
        K::Digit5 => "5",
        K::Digit6 => "6",
        K::Digit7 => "7",
        K::Digit8 => "8",
        K::Digit9 => "9",
        K::ArrowLeft => "arrow_left",
        K::ArrowRight => "arrow_right",
        K::ArrowUp => "arrow_up",
        K::ArrowDown => "arrow_down",
        K::Space => "space",
        K::Enter => "enter",
        K::Escape => "escape",
        K::Tab => "tab",
        K::Backspace => "backspace",
        K::ShiftLeft => "shift_left",
        K::ShiftRight => "shift_right",
        K::ControlLeft => "ctrl_left",
        K::ControlRight => "ctrl_right",
        K::AltLeft => "alt_left",
        K::AltRight => "alt_right",
        K::F1 => "f1",
        K::F2 => "f2",
        K::F3 => "f3",
        K::F4 => "f4",
        K::F5 => "f5",
        K::F6 => "f6",
        K::F7 => "f7",
        K::F8 => "f8",
        K::F9 => "f9",
        K::F10 => "f10",
        K::F11 => "f11",
        K::F12 => "f12",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_runner_messages() {
        assert_eq!(
            parse_msg(r#"{"op":"ready","protocol":1,"python":"3.12.1"}"#),
            Some(BridgeMsg::Ready {
                python: "3.12.1".into()
            })
        );
        let frame = parse_msg(
            r#"{"op":"frame","frame":3,"writes":[{"id":7,"p":[1.0,2.0,3.0],"attrs":{"hp":2,"tint":[1.0,0.0,0.0,1.0]}}]}"#,
        );
        let Some(BridgeMsg::Frame(FrameReply { writes, .. })) = frame else {
            panic!("{frame:?}");
        };
        assert_eq!(writes[0].id, 7);
        assert_eq!(writes[0].position, Some([1.0, 2.0, 3.0]));
        assert_eq!(writes[0].velocity, None);
        let attrs = writes[0].attributes.as_ref().unwrap();
        assert_eq!(attrs["hp"], AttributeValue::Int(2));
        assert_eq!(attrs["tint"], AttributeValue::Color([1.0, 0.0, 0.0, 1.0]));
        assert!(parse_msg("not json").is_none());
        assert!(parse_msg(r#"{"op":"???"}"#).is_none());
    }

    #[test]
    fn splashes_cross_the_bridge() {
        let msg = parse_msg(
            r#"{"op":"frame","frame":1,"writes":[],"splashes":[[1,0.5,-2,2.0],[0,0,0,99],["x",0,0,1]]}"#,
        );
        let Some(BridgeMsg::Frame(FrameReply { splashes, .. })) = msg else {
            panic!("{msg:?}");
        };
        // Strength is clamped to 0..10; malformed entries are skipped.
        assert_eq!(splashes, vec![([1.0, 0.5, -2.0], 2.0), ([0.0; 3], 10.0)]);
    }

    #[test]
    fn player_settings_cross_the_bridge() {
        use automata_renderer::GraphicsQuality as Q;
        let msg = parse_msg(
            r#"{"op":"frame","frame":1,"writes":[],"settings":{"volume":1.5,"graphics":"low"}}"#,
        );
        let Some(BridgeMsg::Frame(FrameReply { settings, .. })) = msg else {
            panic!("{msg:?}");
        };
        assert_eq!(
            settings,
            PlayerSettings {
                volume: Some(1.0),
                graphics: Some(Q::Low)
            }
        );
        let msg = parse_msg(r#"{"op":"frame","frame":2,"settings":{"graphics":"potato"}}"#);
        let Some(BridgeMsg::Frame(FrameReply { settings, .. })) = msg else {
            panic!("{msg:?}");
        };
        assert_eq!(settings, PlayerSettings::default());
        for q in Q::ALL {
            assert_eq!(q.label().to_lowercase(), quality_key(q));
        }
    }

    #[test]
    fn particle_bursts_cross_the_bridge() {
        let msg = parse_msg(concat!(
            r#"{"op":"frame","frame":1,"writes":[],"bursts":["#,
            r#"{"e":"sparks","f":{"color":[6,1.6,0.4,1]},"p":[1,2,3],"d":[0,0,1],"n":12},"#,
            r#"{"e":null,"f":{"rate":5,"sprite":"puff","lit":true},"p":[0,0,0],"d":null,"n":null},"#,
            r#"{"e":"lava","f":{},"p":[0,0,0]},"#,
            r#"{"e":"fire","f":{"shape":"donut"},"p":[0,0,0]},"#,
            r#"{"e":"fire","f":{},"p":[0,"x",0]}],"#,
            r#""emits":[[7,5],[-1,2],[8,"x"]]}"#
        ));
        let Some(BridgeMsg::Frame(FrameReply { bursts, emits, .. })) = msg else {
            panic!("{msg:?}");
        };
        // Unknown presets, bad fields and bad positions drop the burst.
        assert_eq!(bursts.len(), 2, "{bursts:?}");
        let mut sparks = automata_scene::ParticleEmitter::preset("sparks").unwrap();
        sparks.color = [6.0, 1.6, 0.4, 1.0];
        assert_eq!(bursts[0].emitter, sparks);
        assert_eq!(bursts[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(bursts[0].direction, Some([0.0, 0.0, 1.0]));
        assert_eq!(bursts[0].count, Some(12));
        assert_eq!(
            bursts[1].emitter.sprite,
            automata_scene::ParticleSprite::Puff
        );
        assert!(bursts[1].emitter.lit);
        assert_eq!(bursts[1].direction, None);
        assert_eq!(bursts[1].count, None);
        assert_eq!(emits, vec![(7, 5)]);
    }

    #[test]
    fn joint_edits_and_state_cross_the_bridge() {
        use automata_physics::{Joint, JointEdit, JointMotor, JointState};
        let msg = parse_msg(concat!(
            r#"{"op":"frame","frame":1,"writes":[{"id":3,"joint":{"on":false,"#,
            r#""motor":[0.5,1.2],"limits":null,"to":"Frame"}},"#,
            r#"{"id":4,"joint":{"motor":null,"bf":120}},{"id":5,"joint":{"nope":1}}]}"#
        ));
        let Some(BridgeMsg::Frame(FrameReply { writes, .. })) = msg else {
            panic!("{msg:?}");
        };
        assert_eq!(writes[2].joint, None, "unknown keys change nothing");
        assert_eq!(
            writes[0].joint,
            Some(JointEdit {
                enabled: Some(false),
                motor: Some(Some((0.5, Some(1.2)))),
                limits: Some(None),
                connected: Some(Some("Frame".into())),
                ..JointEdit::default()
            })
        );
        assert_eq!(
            writes[1].joint,
            Some(JointEdit {
                motor: Some(None),
                break_force: Some(120.0),
                ..JointEdit::default()
            })
        );
        let state = EntityState {
            joint: Some((
                Joint::hinge(glam::Vec3::Y)
                    .connected_to("Frame")
                    .motor(JointMotor::target(1.0)),
                JointState {
                    value: 0.25,
                    attached: true,
                    ..JointState::default()
                },
            )),
            ..EntityState::default()
        };
        let json = state_json(&state).dump();
        assert!(
            json.contains(r#""joint":{"kind":"hinge","to":"Frame","on":true,"value":0.25,"attached":true,"broken":false"#),
            "{json}"
        );
        assert!(
            json.contains(r#""motor":[0.0,1.0]"#) || json.contains(r#""motor":[0,1]"#),
            "{json}"
        );
        assert!(!state_json(&EntityState::default()).dump().contains("joint"));
    }

    #[test]
    fn animation_requests_and_state_cross_the_bridge() {
        use automata_scene::{AnimationCommand as C, AnimationState, PlayOptions};
        let msg = parse_msg(concat!(
            r#"{"op":"frame","frame":1,"writes":[{"id":3,"anim":["#,
            r#"["play","Walking",true,0.2,null,false],["play","Jump",false,0,1.5,true],"#,
            r#"["stop",0.5],["seek",1.25],["speed",-1],["seek","x"],["dance"],[],"#,
            r#"["reset_springs"],["root_motion",true],["root_motion"]]}]}"#
        ));
        let Some(BridgeMsg::Frame(FrameReply { writes, .. })) = msg else {
            panic!("{msg:?}");
        };
        assert_eq!(
            writes[0].anim,
            vec![
                C::Play {
                    clip: "Walking".into(),
                    options: PlayOptions {
                        fade: 0.2,
                        ..PlayOptions::default()
                    },
                },
                C::Play {
                    clip: "Jump".into(),
                    options: PlayOptions {
                        looping: false,
                        speed: Some(1.5),
                        fade: 0.0,
                        restart: true,
                    },
                },
                C::Stop(0.5),
                C::Seek(1.25),
                C::Speed(-1.0),
                C::ResetSprings,
                C::RootMotion(true),
            ],
            "malformed requests are skipped"
        );
        let state = EntityState {
            anim: Some(AnimationState {
                clip: Some("Idle".into()),
                time: 0.5,
                duration: 2.0,
                playing: true,
                speed: 1.0,
                looping: true,
                clips: vec!["Idle".into(), "Wave".into()],
                lengths: vec![2.0, 1.5],
                springs: false,
                root_motion: true,
                events: vec![automata_scene::FiredEvent {
                    seq: 7,
                    clip: "Idle".into(),
                    name: "blink".into(),
                }],
            }),
            ..EntityState::default()
        };
        let json = state_json(&state).dump();
        assert!(
            json.contains(r#""anim":{"clip":"Idle","time":0.5,"len":2.0,"playing":true"#)
                && json.contains(r#""clips":["Idle","Wave"],"lens":[2.0,1.5]"#)
                && json.contains(r#""rm":true,"ev":[[7,"Idle","blink"]]"#),
            "{json}"
        );
        assert!(!state_json(&EntityState::default()).dump().contains("anim"));
    }

    #[test]
    fn hud_text_crosses_the_bridge() {
        let msg = parse_msg(
            r#"{"op":"frame","frame":1,"writes":[],"hud":[{"k":"text","text":"100%","x":10,"y":20,"size":26,"c":[1,1,1,1],"align":"center","shadow":[0,0,0,0.6]},{"k":"text","text":"x","x":0,"y":0,"size":12,"c":[1,0,0,1],"align":"sideways"}]}"#,
        );
        let Some(BridgeMsg::Frame(FrameReply { hud, .. })) = msg else {
            panic!("frame expected: {msg:?}");
        };
        assert_eq!(
            hud[0].kind,
            HudKind::Text {
                text: "100%".into(),
                size: 26.0,
                align: automata_renderer::TextAlign::Center,
                shadow: Some([0.0, 0.0, 0.0, 0.6]),
                outline: None,
            }
        );
        assert_eq!((hud[0].x, hud[0].y), (10.0, 20.0));
        assert!(
            matches!(
                hud[1].kind,
                HudKind::Text {
                    align: automata_renderer::TextAlign::Left,
                    ..
                }
            ),
            "unknown alignments fall back to left"
        );
    }

    #[test]
    fn camera_state_and_writes_cross_the_bridge() {
        let state = EntityState {
            id: 4,
            camera: Some([0.0, 2.0, 6.0, 0.0, 0.0, 0.0, 60.0]),
            ..EntityState::default()
        };
        let json = state_json(&state);
        assert_eq!(
            json.get("cam").and_then(Json::as_floats::<7>),
            Some([0.0, 2.0, 6.0, 0.0, 0.0, 0.0, 60.0])
        );
        let msg =
            parse_msg(r#"{"op":"frame","frame":1,"writes":[{"id":4,"cam":[1,2,3,4,5,6,50]}]}"#);
        let Some(BridgeMsg::Frame(FrameReply { writes, .. })) = msg else {
            panic!("frame expected");
        };
        assert_eq!(writes[0].camera, Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 50.0]));
    }

    #[test]
    fn attribute_values_round_trip_through_json() {
        for v in [
            AttributeValue::Bool(true),
            AttributeValue::Int(-4),
            AttributeValue::Float(2.0),
            AttributeValue::Text("hi \"there\"".into()),
            AttributeValue::Vec3([1.0, 2.0, 3.0]),
            AttributeValue::Color([0.5, 0.25, 0.0, 1.0]),
        ] {
            let text = attr_json(&v).dump();
            assert_eq!(attr_from_json(&Json::parse(&text).unwrap()), Some(v));
        }
    }

    fn python() -> Option<&'static str> {
        Command::new(default_python())
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| default_python())
    }

    fn pump(bridge: &mut ScriptBridge, until: impl Fn(&BridgeMsg) -> bool) -> Vec<BridgeMsg> {
        let mut all = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let msgs = bridge.poll();
            let done = msgs.iter().any(&until);
            all.extend(msgs);
            if done {
                return all;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out; got {all:?}");
    }

    /// End to end with a real interpreter (skipped when none is installed).
    #[test]
    fn a_relative_runtime_dir_still_starts_the_runner() {
        let Some(python) = python() else {
            eprintln!("no python; skipping");
            return;
        };
        // Python runs in `cwd`; the runner path must not be relative to us.
        let cwd = std::env::temp_dir().join(format!("oberon_bridge_cwd_{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let runtime = PathBuf::from(format!(
            "../../target/oberon_bridge_rel_{}",
            std::process::id()
        ));
        let mut bridge = ScriptBridge::start(python, &runtime, &cwd).unwrap();
        let msgs = pump(&mut bridge, |m| matches!(m, BridgeMsg::Ready { .. }));
        assert!(
            msgs.iter().any(|m| matches!(m, BridgeMsg::Ready { .. })),
            "{msgs:?}"
        );
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn saves_and_settings_live_in_the_project() {
        let Some(python) = python() else {
            eprintln!("no python; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("oberon_bridge_save_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("saver.py");
        std::fs::write(
            &script,
            "def start(entity, ctx):\n    ctx.save['coins'] = ctx.save.get('coins', 0) + 12\n    \
             entity.attributes['gfx'] = ctx.settings.graphics\n    \
             ctx.settings.graphics = 'low'\n    ctx.settings.volume = 0.25\n",
        )
        .unwrap();
        let entities = vec![EntityState {
            id: 1,
            name: "Saver".into(),
            script: Some(script.clone()),
            scale: [1.0; 3],
            ..EntityState::default()
        }];
        let input = FrameInput {
            graphics: "medium",
            ..FrameInput::default()
        };
        for run in 1..=2 {
            let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
            pump(&mut bridge, |m| matches!(m, BridgeMsg::Ready { .. }));
            assert!(bridge.send_frame_with(0.016, 0.016, &[], &entities, &input));
            let msgs = pump(&mut bridge, |m| matches!(m, BridgeMsg::Frame(_)));
            let reply = msgs
                .iter()
                .find_map(|m| match m {
                    BridgeMsg::Frame(r) => Some(r.clone()),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                reply.settings,
                PlayerSettings {
                    volume: Some(0.25),
                    graphics: Some(automata_renderer::GraphicsQuality::Low),
                },
                "{msgs:?}"
            );
            // The first run sees Play's preset; the second the saved one.
            let gfx = reply.writes[0].attributes.as_ref().unwrap()["gfx"].clone();
            let want = if run == 1 { "medium" } else { "low" };
            assert_eq!(gfx, AttributeValue::Text(want.into()));
            drop(bridge.stop());
            let saved =
                std::fs::read_to_string(dir.join(".oberon/save/saves/default.json")).unwrap();
            assert!(
                saved.contains(&format!("\"coins\": {}", 12 * run)),
                "{saved}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runs_scripts_and_reports_errors() {
        let Some(python) = python() else {
            eprintln!("no python; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("oberon_bridge_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mover = dir.join("mover.py");
        std::fs::write(
            &mover,
            "def start(entity, ctx):\n    print('hello', entity.name)\n\n\
             def update(entity, ctx):\n    x, y, z = entity.transform.position\n    \
             entity.transform.position = (x + 1.0, y, z)\n    \
             entity.attributes['ticks'] = entity.attributes.get('ticks', 0) + 1\n    \
             if ctx.input.pressed('space'):\n        entity.character_velocity = (0, 5, 0)\n",
        )
        .unwrap();
        let broken = dir.join("broken.py");
        std::fs::write(&broken, "def update(entity, ctx):\n    return 1 / 0\n").unwrap();

        let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
        let msgs = pump(&mut bridge, |m| matches!(m, BridgeMsg::Ready { .. }));
        assert!(msgs.iter().any(|m| matches!(m, BridgeMsg::Ready { .. })));

        let entities = vec![
            EntityState {
                id: 1,
                name: "Hero".into(),
                script: Some(mover.clone()),
                velocity: Some([0.0; 3]),
                scale: [1.0; 3],
                ..EntityState::default()
            },
            EntityState {
                id: 2,
                name: "Bad".into(),
                script: Some(broken.clone()),
                ..EntityState::default()
            },
        ];
        assert!(bridge.send_frame(0.016, 0.016, &["space"], &entities));
        assert!(bridge.busy());
        assert!(
            !bridge.send_frame(0.016, 0.032, &[], &entities),
            "one frame in flight"
        );
        let msgs = pump(&mut bridge, |m| matches!(m, BridgeMsg::Frame(_)));
        assert!(!bridge.busy());
        assert!(
            msgs.iter()
                .any(|m| matches!(m, BridgeMsg::Log { msg, .. } if msg == "hello Hero")),
            "{msgs:?}"
        );
        let err = msgs
            .iter()
            .find_map(|m| match m {
                BridgeMsg::Error {
                    file,
                    line,
                    traceback,
                    ..
                } => Some((file.clone(), *line, traceback.clone())),
                _ => None,
            })
            .expect("error reported");
        assert_eq!(err.0, broken);
        assert_eq!(err.1, 2);
        assert!(
            err.2.contains("ZeroDivisionError")
                && !err.2.contains("bridge.py")
                && !err.2.contains(SCRIPT_API_FILE),
            "{}",
            err.2
        );
        let writes = msgs
            .iter()
            .find_map(|m| match m {
                BridgeMsg::Frame(w) => Some(w.writes.clone()),
                _ => None,
            })
            .unwrap();
        let hero = writes.iter().find(|w| w.id == 1).unwrap();
        assert_eq!(hero.position, Some([1.0, 0.0, 0.0]));
        assert_eq!(hero.velocity, Some([0.0, 5.0, 0.0]));
        assert_eq!(
            hero.attributes.as_ref().unwrap()["ticks"],
            AttributeValue::Int(1)
        );
        assert!(
            writes.iter().all(|w| w.id != 2),
            "a broken script writes nothing"
        );

        let leftover = bridge.stop();
        assert!(leftover
            .iter()
            .all(|m| !matches!(m, BridgeMsg::Error { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
