//! Headless Play benchmark: runs a scene the way Oberon's Play mode does
//! (the simulation in Rust, behaviour scripts in a Python process over the
//! bridge), with no window, and times every frame.
//!
//! ```sh
//! oberon --bench level.ron [--frames 300] [--warmup 30] [--seconds 60]
//!        [--python python3] [--out stats.json]
//! ```
//!
//! Frames run in lock-step: each one waits for the scripts' reply, so the
//! frame time is what Play costs when the scripts have to keep up. The
//! runner reports its own script time, so a frame splits into **script**
//! (Python running `update`) and **engine** (everything else: physics, the
//! simulation, and the trip over the bridge). `examples/bench/run.py`
//! drives this and prints the table.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::bridge::{BridgeMsg, ScriptBridge};
use crate::doc::Document;
use crate::play::{PlaySession, TIMESTEP};

/// How long to run.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    /// Frames to measure.
    pub frames: u32,
    /// Frames to run first without measuring (imports, physics settling).
    pub warmup: u32,
    /// Stop measuring after this many seconds even if `frames` isn't
    /// reached (slow scenes still report what they managed).
    pub seconds: f32,
    /// The Python interpreter that runs the scripts.
    pub python: String,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            frames: 300,
            warmup: 30,
            seconds: 60.0,
            python: crate::bridge::default_python().to_string(),
        }
    }
}

/// The measured frames: whole-frame and script times in milliseconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BenchStats {
    /// Whole frame, per measured frame.
    pub frame_ms: Vec<f32>,
    /// Scripts alone, per measured frame.
    pub script_ms: Vec<f32>,
}

impl BenchStats {
    /// As JSON: `{"runner": "oberon", "frame_ms": [...], "script_ms": [...]}`
    /// (the format `run_scene` writes too).
    #[must_use]
    pub fn to_json(&self) -> String {
        use crate::json::Json;
        Json::obj([
            ("runner", Json::from("oberon")),
            ("frame_ms", Json::floats(&self.frame_ms)),
            ("script_ms", Json::floats(&self.script_ms)),
        ])
        .dump()
    }
}

/// Runs the scene at `path` headless and times its frames.
///
/// # Errors
///
/// Fails when the scene doesn't load, the interpreter can't start, a script
/// raises (timings with a broken script mean nothing), or the runner stops
/// answering for 30 s.
pub fn run(path: &Path, options: &BenchOptions) -> Result<BenchStats, String> {
    let doc = Document::load(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let base = doc.base_dir();
    automata_assets::set_root(Some(&base));
    let mut play = PlaySession::start(&doc);
    play.update(TIMESTEP); // colliders exist after the first step
    let runtime = std::env::temp_dir().join(format!("oberon-bench-{}", std::process::id()));
    let mut bridge = ScriptBridge::start(&options.python, &runtime, &base)
        .map_err(|e| format!("cannot start {}: {e}", options.python))?;
    let result = frames(&mut play, &mut bridge, &doc, options);
    drop(bridge.stop());
    let _ = std::fs::remove_dir_all(&runtime);
    result
}

fn frames(
    play: &mut PlaySession,
    bridge: &mut ScriptBridge,
    doc: &Document,
    options: &BenchOptions,
) -> Result<BenchStats, String> {
    let mut stats = BenchStats::default();
    let mut measuring_since = None;
    for frame in 0..options.warmup.saturating_add(options.frames) {
        let started = Instant::now();
        let mut input = play.frame_input();
        input.screen = [1280, 720];
        let states = play.script_states(|rel| doc.resolve(rel));
        if !bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
            return Err("the script runner stopped".into());
        }
        let reply = wait_for_reply(bridge)?;
        play.apply_frame(&reply);
        play.update(TIMESTEP);
        if frame < options.warmup {
            continue;
        }
        let since = *measuring_since.get_or_insert(started);
        stats
            .frame_ms
            .push(started.elapsed().as_secs_f32() * 1000.0);
        stats.script_ms.push(reply.script_ms.unwrap_or(0.0));
        if since.elapsed().as_secs_f32() > options.seconds {
            break;
        }
    }
    Ok(stats)
}

fn wait_for_reply(bridge: &mut ScriptBridge) -> Result<crate::bridge::FrameReply, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        for msg in bridge.wait(Duration::from_millis(100)) {
            match msg {
                BridgeMsg::Frame(reply) => return Ok(reply),
                BridgeMsg::Error { msg, traceback, .. } => {
                    return Err(format!("a script failed: {msg}\n{traceback}"));
                }
                BridgeMsg::Exited(status) => {
                    return Err(format!("the script runner exited ({status})"));
                }
                BridgeMsg::Log { level, msg } if level == "error" => eprintln!("{msg}"),
                _ => {}
            }
        }
    }
    Err("the scripts took over 30 s to answer one frame".into())
}

/// Parses `--bench` arguments (everything after `--bench`): the scene, then
/// `--frames N`, `--warmup N`, `--seconds S`, `--python P`, `--out FILE`.
/// Returns the scene, the options and the output file.
///
/// # Errors
///
/// Describes the first bad or missing argument.
pub fn parse_args(
    args: &[String],
) -> Result<(std::path::PathBuf, BenchOptions, Option<std::path::PathBuf>), String> {
    let mut options = BenchOptions::default();
    let mut scene = None;
    let mut out = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--frames" => {
                options.frames = value("--frames")?
                    .parse()
                    .map_err(|_| "--frames takes a whole number")?;
            }
            "--warmup" => {
                options.warmup = value("--warmup")?
                    .parse()
                    .map_err(|_| "--warmup takes a whole number")?;
            }
            "--seconds" => {
                options.seconds = value("--seconds")?
                    .parse()
                    .map_err(|_| "--seconds takes a number")?;
            }
            "--python" => options.python = value("--python")?,
            "--out" => out = Some(value("--out")?.into()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => scene = Some(std::path::PathBuf::from(other)),
        }
    }
    let scene = scene.ok_or("--bench needs a scene file")?;
    Ok((scene, options, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_parse() {
        let args: Vec<String> = ["level.ron", "--frames", "12", "--out", "s.json"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let (scene, options, out) = parse_args(&args).unwrap();
        assert_eq!(scene, Path::new("level.ron"));
        assert_eq!(options.frames, 12);
        assert_eq!(options.warmup, 30);
        assert_eq!(out.as_deref(), Some(Path::new("s.json")));
        assert!(parse_args(&["--frames".into()]).is_err());
        assert!(parse_args(&["--nope".into()]).is_err());
        assert!(parse_args(&[]).is_err());
    }

    /// A scripted scene runs headless through the real bridge and every
    /// measured frame has a script share (skipped without python3).
    #[test]
    fn scenes_run_headless_and_report_script_time() {
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("oberon_bench_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("spin.py"),
            "def update(entity, ctx):\n\
             \x20   x, y, z = entity.transform.position\n\
             \x20   entity.transform.position = (x + 0.01, y, z)\n",
        )
        .unwrap();
        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        let i = doc.add_entity_named("Spinner");
        let id = doc.entities[i].id;
        doc.modify(id, false, |d| {
            d.script = Some(automata_core::Script {
                path: "spin.py".into(),
            });
        });
        doc.save(None).unwrap();
        let options = BenchOptions {
            frames: 5,
            warmup: 2,
            ..BenchOptions::default()
        };
        let stats = run(&dir.join("level.ron"), &options).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(stats.frame_ms.len(), 5);
        assert_eq!(stats.script_ms.len(), 5);
        assert!(stats
            .frame_ms
            .iter()
            .zip(&stats.script_ms)
            .all(|(f, s)| *s > 0.0 && f >= s));
        assert!(stats.to_json().contains("\"runner\":\"oberon\""));
    }
}
