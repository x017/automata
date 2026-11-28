//! The `oberon` command line: the editor plus everything around a project.
//!
//! ```text
//! oberon [open] [SCENE|FOLDER]   the editor
//! oberon new NAME [--template 3d|2d|empty]
//! oberon setup | run | test | export | bench | python | pip | doctor
//! ```
//!
//! Exit codes: 0 success, 1 failure, 2 bad usage. `run`, `test`, `python`
//! and `pip` pass on the exit code of what they ran.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use crate::newproject::{self, Template};
use crate::pyenv::{self, Bundle, SetupOptions, VenvState, ENGINE_VERSION};

/// Exit code for a bad command line.
pub const USAGE_ERROR: u8 = 2;

const HELP: &str = "\
oberon — the AUTOMATA editor and command line

usage:
  oberon [open] [SCENE|FOLDER]   open the editor (a project folder opens its main scene)
  oberon new NAME [--template 3d|2d|empty]
                                 make a project, with its own Python in .venv
  oberon setup [--upgrade]       make or repair the project's .venv
  oberon run [SCENE]             play a scene in a window, without the editor
  oberon test [ARGS…]            run the project's tests/ headless
  oberon export [--target linux|windows|macos|android|all]
                                 build the game (also: export check|init|templates)
  oberon bench SCENE [--frames N] [--out stats.json]
                                 time a scene in headless Play
  oberon python [ARGS…]          the project's own Python
  oberon pip [ARGS…]             install packages into the project's Python
  oberon doctor                  check the GPU, Python, .venv and editor support
  oberon help [COMMAND]          this, or a command's details (also COMMAND --help)
  oberon --version

Commands that work on a project find it from the current folder: the nearest
folder upwards with a pyproject.toml, .venv or build.ron.
Exit codes: 0 success, 1 failure, 2 bad usage.";

/// The help text of one command.
#[must_use]
pub fn command_help(command: &str) -> Option<&'static str> {
    Some(match command {
        "open" => {
            "\
usage: oberon open [SCENE|FOLDER]

Opens the editor. A scene file opens as is; a project folder opens its main
scene (pyproject.toml's [tool.oberon] scene, else level.ron, else its only
scene). With nothing, the editor starts with a fresh scene."
        }
        "new" => {
            "\
usage: oberon new NAME [--template 3d|2d|empty] [--python PATH] [--no-venv]

Makes the folder NAME with a ready-to-play project:
  level.ron, scripts/player.py (typed), tests/test_level.py, models/,
  sounds/, README.md, .gitignore, build.ron, pyproject.toml (pinning
  automata-engine to this Oberon's version), pyrightconfig.json and
  .vscode/settings.json pointing at .venv.
Then makes .venv from Oberon's bundled Python and wheels, offline (with uv
when it's installed), so completions work in VS Code and PyCharm at once.

  --template   3d (default): walk and jump around a yard of crates
               2d: a side-on platformer
               empty: a sun, a sky and a camera
  --python     the interpreter to make .venv from (default: the bundled one,
               else python3 on the PATH)
  --no-venv    skip .venv (run `oberon setup` later)"
        }
        "setup" => {
            "\
usage: oberon setup [PROJECT] [--upgrade] [--python PATH] [--no-uv]

Makes the project's .venv if it's missing (after cloning, say), installs the
automata-engine wheel matching this Oberon, plus basedpyright and pytest when
Oberon carries them. Offline when Oberon has bundled wheels.

  --upgrade    reinstall the engine at this Oberon's version
  --python     the interpreter to make .venv from
  --no-uv      use python -m venv and pip even if uv is installed"
        }
        "run" => {
            "\
usage: oberon run [SCENE] [--headless] [--frames N]

Plays SCENE (default: the project's main scene) in a window with the
project's Python, like an exported game.

  --headless   no window (for CI); implies a fixed 1/60 s step
  --frames N   stop after N frames"
        }
        "test" => {
            "\
usage: oberon test [ARGS…]

Runs the project's tests/ with its own Python: pytest when the project has it
(ARGS go to pytest), else Oberon's small runner, which calls every test_*
function in tests/test_*.py. Exits non-zero when a test fails."
        }
        "export" => {
            "\
usage: oberon export [PROJECT] [--target linux|windows|macos|android|web|all] [--template DIR]
       oberon export check [PROJECT]
       oberon export init [PROJECT] [--scene SCENE.ron] [--name NAME]
       oberon export serve [PROJECT | FOLDER] [--port 8000]
       oberon export stage [PROJECT] [--out DIR] [--zip FILE]
       oberon export keygen FILE [--name NAME]
       oberon export templates path|list|download|install|dev|pack …

Builds the game into build/ using the project's build.ron (what
automata-cli does)."
        }
        "bench" => {
            "\
usage: oberon bench SCENE [--frames N] [--warmup N] [--seconds S] [--python P] [--out stats.json]

Runs SCENE the way Play does, without a window, and prints frame-time
statistics as JSON."
        }
        "python" => {
            "\
usage: oberon python [ARGS…]

Runs the project's Python (.venv) with ARGS: `oberon python -m pytest -x`,
`oberon python tools/make_level.py`."
        }
        "pip" => {
            "\
usage: oberon pip [ARGS…]

Runs pip for the project's Python: `oberon pip install numpy`. Uses uv's pip
when the venv was made by uv."
        }
        "doctor" => {
            "\
usage: oberon doctor [PROJECT]

Checks what Oberon needs and says how to fix what's missing: the GPU, the
bundled Python and wheels, the project's .venv and engine version, and the
language server behind code completion. Exits 1 when something is broken."
        }
        _ => return None,
    })
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Open the editor, optionally on a scene or folder.
    Open(Option<PathBuf>),
    /// Make a project.
    New {
        /// The folder to make.
        dir: PathBuf,
        /// What it starts with.
        template: Template,
        /// The interpreter to make `.venv` from.
        python: Option<PathBuf>,
        /// Skip `.venv`.
        no_venv: bool,
    },
    /// Make or repair `.venv`.
    Setup {
        /// The project (default: found from the current folder).
        project: Option<PathBuf>,
        /// Reinstall the engine.
        upgrade: bool,
        /// The interpreter to make `.venv` from.
        python: Option<PathBuf>,
        /// Don't use uv.
        no_uv: bool,
    },
    /// Play a scene without the editor.
    Run {
        /// The scene (default: the project's main scene).
        scene: Option<PathBuf>,
        /// No window.
        headless: bool,
        /// Stop after this many frames.
        frames: Option<u64>,
    },
    /// Run the tests.
    Test(Vec<String>),
    /// `automata-cli`'s command line.
    Export(Vec<String>),
    /// Headless Play timing.
    Bench(Vec<String>),
    /// The project's Python.
    Python(Vec<String>),
    /// The project's pip.
    Pip(Vec<String>),
    /// Diagnostics.
    Doctor(Option<PathBuf>),
    /// Help, for everything or one command.
    Help(Option<String>),
    /// The version.
    Version,
}

/// Parses a command line (without the program name).
///
/// # Errors
///
/// Returns the message for a bad command line.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Ok(Command::Open(None));
    };
    let rest = &args[1..];
    // `oberon COMMAND --help` / `-h` (except where the arguments belong to
    // someone else).
    let passthrough = ["python", "pip", "test", "export"];
    if !passthrough.contains(&first.as_str())
        && rest.iter().any(|a| a == "--help" || a == "-h")
        && command_help(first).is_some()
    {
        return Ok(Command::Help(Some(first.clone())));
    }
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help(rest.first().cloned())),
        "-V" | "--version" | "version" => Ok(Command::Version),
        // The old spelling.
        "--bench" | "bench" => Ok(Command::Bench(rest.to_vec())),
        "open" => {
            let (pos, _) = split(rest, &[], &[])?;
            at_most(&pos, 1, "open")?;
            Ok(Command::Open(pos.first().map(PathBuf::from)))
        }
        "new" => {
            let (pos, flags) = split(rest, &["template", "python"], &["no-venv"])?;
            at_most(&pos, 1, "new")?;
            let name = pos
                .first()
                .ok_or("oberon new needs a NAME (try `oberon new mygame`)")?;
            let template = match flag(&flags, "template") {
                None => Template::ThreeD,
                Some(t) => Template::parse(t)
                    .ok_or_else(|| format!("unknown template {t:?} (use 3d, 2d or empty)"))?,
            };
            Ok(Command::New {
                dir: PathBuf::from(name),
                template,
                python: flag(&flags, "python").map(PathBuf::from),
                no_venv: flag(&flags, "no-venv").is_some(),
            })
        }
        "setup" => {
            let (pos, flags) = split(rest, &["python"], &["upgrade", "no-uv"])?;
            at_most(&pos, 1, "setup")?;
            Ok(Command::Setup {
                project: pos.first().map(PathBuf::from),
                upgrade: flag(&flags, "upgrade").is_some(),
                python: flag(&flags, "python").map(PathBuf::from),
                no_uv: flag(&flags, "no-uv").is_some(),
            })
        }
        "run" => {
            let (pos, flags) = split(rest, &["frames"], &["headless"])?;
            at_most(&pos, 1, "run")?;
            let frames = flag(&flags, "frames")
                .map(|n| {
                    n.parse::<u64>()
                        .map_err(|_| format!("--frames needs a whole number, not {n:?}"))
                })
                .transpose()?;
            Ok(Command::Run {
                scene: pos.first().map(PathBuf::from),
                headless: flag(&flags, "headless").is_some(),
                frames,
            })
        }
        "test" => Ok(Command::Test(rest.to_vec())),
        "export" => Ok(Command::Export(rest.to_vec())),
        "python" => Ok(Command::Python(rest.to_vec())),
        "pip" => Ok(Command::Pip(rest.to_vec())),
        "doctor" => {
            let (pos, _) = split(rest, &[], &[])?;
            at_most(&pos, 1, "doctor")?;
            Ok(Command::Doctor(pos.first().map(PathBuf::from)))
        }
        other if other.starts_with('-') => Err(format!("unknown option {other}")),
        // `oberon level.ron` / `oberon mygame/`: open, as before.
        other
            if Path::new(other).exists()
                || Path::new(other)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ron")) =>
        {
            at_most(rest, 0, "open")?;
            Ok(Command::Open(Some(PathBuf::from(other))))
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

type Flags = Vec<(String, Option<String>)>;

/// Splits arguments into positionals and `--flags` (`valued` take a value,
/// `switches` don't).
fn split(
    args: &[String],
    valued: &[&str],
    switches: &[&str],
) -> Result<(Vec<String>, Flags), String> {
    let mut pos = Vec::new();
    let mut flags = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let Some(name) = a.strip_prefix("--") else {
            pos.push(a.clone());
            continue;
        };
        let (name, inline) = match name.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (name, None),
        };
        if valued.contains(&name) {
            let value = inline
                .or_else(|| it.next().cloned())
                .ok_or_else(|| format!("--{name} needs a value"))?;
            flags.push((name.to_string(), Some(value)));
        } else if switches.contains(&name) && inline.is_none() {
            flags.push((name.to_string(), None));
        } else {
            return Err(format!("unknown option --{name}"));
        }
    }
    Ok((pos, flags))
}

fn flag<'a>(flags: &'a Flags, name: &str) -> Option<&'a str> {
    flags
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_deref().unwrap_or(""))
}

fn at_most(pos: &[String], n: usize, command: &str) -> Result<(), String> {
    if pos.len() > n {
        Err(format!(
            "too many arguments for `oberon {command}`: {}",
            pos[n..].join(" ")
        ))
    } else {
        Ok(())
    }
}

/// Runs the command line; `main` just calls this.
pub fn main(args: &[String]) -> ExitCode {
    let command = match parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oberon: {e}");
            eprintln!("run `oberon help` for the commands");
            return ExitCode::from(USAGE_ERROR);
        }
    };
    let result = match command {
        Command::Help(None) => {
            println!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Help(Some(c)) => match command_help(&c) {
            Some(text) => {
                println!("{text}");
                Ok(ExitCode::SUCCESS)
            }
            None => {
                eprintln!("oberon: no command {c:?}; run `oberon help`");
                return ExitCode::from(USAGE_ERROR);
            }
        },
        Command::Version => {
            println!("oberon {ENGINE_VERSION}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Open(path) => open(path.as_deref()),
        Command::New {
            dir,
            template,
            python,
            no_venv,
        } => new(&dir, template, python, no_venv),
        Command::Setup {
            project,
            upgrade,
            python,
            no_uv,
        } => setup(project.as_deref(), upgrade, python, no_uv),
        Command::Run {
            scene,
            headless,
            frames,
        } => run(scene.as_deref(), headless, frames),
        Command::Test(args) => test(&args),
        Command::Export(args) => export(&args),
        Command::Bench(args) => Ok(bench(&args)),
        Command::Python(args) => python(&args),
        Command::Pip(args) => pip(&args),
        Command::Doctor(project) => Ok(doctor(project.as_deref())),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("oberon: {e}");
            ExitCode::FAILURE
        }
    }
}

type CliResult = Result<ExitCode, String>;

/// The project around the current folder (or `start`), else the folder
/// itself.
fn project(start: Option<&Path>) -> PathBuf {
    let start = start.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        Path::to_path_buf,
    );
    pyenv::project_root(&start).unwrap_or(start)
}

/// The interpreter for project commands, with a hint when it isn't the
/// project's own.
fn python_for(root: &Path) -> String {
    let prefs = crate::project::Prefs::load();
    let python = pyenv::project_python(prefs.python.as_deref(), root);
    if pyenv::venv_state(root) == VenvState::Missing && prefs.python.is_none() {
        eprintln!(
            "oberon: {} has no .venv; using {python} (run `oberon setup` to give it its own)",
            root.display()
        );
    }
    python
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(c) => ExitCode::from(u8::try_from(c).unwrap_or(1).max(1)),
        None => ExitCode::FAILURE,
    }
}

fn spawn(cmd: &mut Process) -> CliResult {
    let shown = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run {shown}: {e}"))?;
    Ok(exit_code(status))
}

fn open(path: Option<&Path>) -> CliResult {
    let scene = match path {
        Some(p) if p.is_dir() => {
            let root = pyenv::project_root(p).unwrap_or_else(|| p.to_path_buf());
            Some(newproject::main_scene(&root).ok_or_else(|| {
                format!(
                    "{} has no scene to open (expected level.ron, or [tool.oberon] scene in pyproject.toml)",
                    p.display()
                )
            })?)
        }
        Some(p) => Some(p.to_path_buf()),
        // Inside a project: its main scene. Elsewhere: the starter scene.
        None => std::env::current_dir()
            .ok()
            .and_then(|cwd| pyenv::project_root(&cwd))
            .and_then(|root| newproject::main_scene(&root)),
    };
    if let Some(path) = &scene {
        crate::doc::Document::load(path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        tracing::info!(path = %path.display(), "opening scene");
    }
    crate::present::run(scene.as_deref()).map_err(|e| {
        format!(
            "{e}\na window needs a display server (X11/Wayland). On a headless machine, \
             `oberon run --headless` plays a scene and `oberon test` runs the tests"
        )
    })?;
    Ok(ExitCode::SUCCESS)
}

fn new(dir: &Path, template: Template, python: Option<PathBuf>, no_venv: bool) -> CliResult {
    let files = newproject::create(dir, template)?;
    println!(
        "made {} ({} template, {} files)",
        dir.display(),
        template.name(),
        files.len()
    );
    if no_venv {
        println!(
            "skipped .venv; run `oberon setup` in {} to make it",
            dir.display()
        );
    } else {
        let options = SetupOptions {
            base_python: python,
            ..SetupOptions::default()
        };
        let bundle = Bundle::find();
        if let Err(e) = pyenv::setup_venv(dir, &bundle, &options, &mut |m| println!("  {m}")) {
            eprintln!("oberon: the project is made, but .venv isn't: {e}");
            eprintln!(
                "oberon: fix that, then run `oberon setup` in {}",
                dir.display()
            );
            return Ok(ExitCode::FAILURE);
        }
    }
    let name = dir.display();
    println!("\nnext:\n  cd {name}\n  oberon open      # the editor\n  oberon run       # play it\n  oberon test      # run tests/\n  code .           # VS Code, with completions from .venv");
    Ok(ExitCode::SUCCESS)
}

fn setup(
    project_dir: Option<&Path>,
    upgrade: bool,
    python: Option<PathBuf>,
    no_uv: bool,
) -> CliResult {
    let root = project(project_dir);
    println!("setting up {}", root.display());
    let options = SetupOptions {
        base_python: python,
        upgrade,
        no_uv,
    };
    pyenv::setup_venv(&root, &Bundle::find(), &options, &mut |m| println!("  {m}"))?;
    match pyenv::venv_state(&root) {
        VenvState::Ready => println!("ready: .venv has automata-engine {ENGINE_VERSION}"),
        VenvState::Outdated(v) => println!(
            "note: .venv has automata-engine {v}; this Oberon is {ENGINE_VERSION} (`oberon setup --upgrade`)"
        ),
        VenvState::NoEngine | VenvState::Missing => {
            return Err("the venv was made but has no automata-engine".into());
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs a scene with the engine, with a clear message when it's missing.
const RUN_SCENE: &str = r#"
import sys
try:
    import automata
except ImportError as e:
    sys.exit(f"oberon run: this Python has no automata-engine ({e}); run `oberon setup`")
scene, frames, headless = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
options = {}
if frames:
    options["max_frames"] = int(frames)
if headless:
    options["headless"] = True
    options["fixed_delta"] = 1 / 60
ran = automata.run_scene(scene, **options)
if headless:
    print(f"ran {ran} frames")
"#;

fn run(scene: Option<&Path>, headless: bool, frames: Option<u64>) -> CliResult {
    let (root, scene) = match scene {
        Some(s) => (project(s.parent()), s.to_path_buf()),
        None => {
            let root = project(None);
            let scene = newproject::main_scene(&root).ok_or_else(|| {
                format!(
                    "no scene to run in {} (pass one, or add level.ron)",
                    root.display()
                )
            })?;
            (root, scene)
        }
    };
    let scene = std::fs::canonicalize(&scene).map_err(|e| format!("{}: {e}", scene.display()))?;
    let mut cmd = Process::new(python_for(&root));
    cmd.arg("-c")
        .arg(RUN_SCENE)
        .arg(&scene)
        .arg(frames.map(|f| f.to_string()).unwrap_or_default())
        .arg(if headless { "1" } else { "0" })
        .current_dir(&root);
    spawn(&mut cmd)
}

/// pytest when it's installed, else a small runner for plain `test_*`
/// functions.
const RUN_TESTS: &str = r#"
import importlib.util, inspect, os, sys, traceback
args = sys.argv[1:]
try:
    import pytest
except ImportError:
    pytest = None
if pytest is not None:
    sys.exit(pytest.main(args or ["tests"]))
if args:
    print("oberon test: pytest isn't installed, so arguments are ignored", file=sys.stderr)
root = os.getcwd()
tests = os.path.join(root, "tests")
sys.path[:0] = [root, tests]
files = sorted(f for f in os.listdir(tests) if f.startswith("test_") and f.endswith(".py")) if os.path.isdir(tests) else []
passed = failed = skipped = 0
for name in files:
    spec = importlib.util.spec_from_file_location(name[:-3], os.path.join(tests, name))
    module = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(module)
    except Exception:
        failed += 1
        print(f"ERROR {name}")
        traceback.print_exc()
        continue
    for attr, fn in list(vars(module).items()):
        if not attr.startswith("test_") or not callable(fn):
            continue
        if inspect.signature(fn).parameters:
            skipped += 1
            print(f"SKIP  {name}::{attr} (uses fixtures; install pytest)")
            continue
        try:
            fn()
        except Exception:
            failed += 1
            print(f"FAIL  {name}::{attr}")
            traceback.print_exc()
        else:
            passed += 1
            print(f"PASS  {name}::{attr}")
total = passed + failed
print(f"\n{passed} passed, {failed} failed" + (f", {skipped} skipped" if skipped else ""))
sys.exit(1 if failed else (5 if total == 0 else 0))
"#;

fn test(args: &[String]) -> CliResult {
    let root = project(None);
    let mut cmd = Process::new(python_for(&root));
    cmd.arg("-c").arg(RUN_TESTS).args(args).current_dir(&root);
    spawn(&mut cmd)
}

fn export(args: &[String]) -> CliResult {
    if matches!(args.first().map(String::as_str), Some("--help" | "-h")) {
        println!("{}", command_help("export").unwrap_or_default());
        return Ok(ExitCode::SUCCESS);
    }
    let full = export_args(args, &project(None).to_string_lossy());
    automata_cli::cli::run(full, "oberon export")?;
    Ok(ExitCode::SUCCESS)
}

/// `oberon export …` as `automata-cli` arguments: `build` unless a
/// subcommand is named, and the project when a project-taking command
/// names none.
fn export_args(args: &[String], project: &str) -> Vec<String> {
    let subcommands = [
        "build",
        "init",
        "check",
        "stage",
        "serve",
        "keygen",
        "templates",
        "help",
    ];
    let mut full: Vec<String> = args.to_vec();
    if !full
        .first()
        .is_some_and(|first| subcommands.contains(&first.as_str()))
    {
        full.insert(0, "build".into());
    }
    if matches!(
        full[0].as_str(),
        "build" | "init" | "check" | "stage" | "serve"
    ) && full.get(1).is_none_or(|a| a.starts_with("--"))
    {
        full.insert(1, project.into());
    }
    full
}

/// `oberon bench`: times a scene in headless Play (see `crate::bench`).
fn bench(args: &[String]) -> ExitCode {
    let (scene, options, out) = match crate::bench::parse_args(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("oberon bench: {err}");
            eprintln!("{}", command_help("bench").unwrap_or_default());
            return ExitCode::from(USAGE_ERROR);
        }
    };
    match crate::bench::run(&scene, &options) {
        Ok(stats) => {
            let json = stats.to_json();
            match out {
                Some(path) => {
                    if let Err(err) = std::fs::write(&path, json) {
                        eprintln!("oberon bench: cannot write {}: {err}", path.display());
                        return ExitCode::FAILURE;
                    }
                }
                None => println!("{json}"),
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("oberon bench: {err}");
            ExitCode::FAILURE
        }
    }
}

fn python(args: &[String]) -> CliResult {
    let root = project(None);
    let mut cmd = Process::new(python_for(&root));
    cmd.args(args);
    spawn(&mut cmd)
}

fn pip(args: &[String]) -> CliResult {
    let root = project(None);
    let venv = pyenv::venv_dir(&root);
    let python = python_for(&root);
    let has_pip = ["pip", "pip3", "pip.exe"]
        .iter()
        .any(|p| pyenv::bin_dir(&venv).join(p).is_file());
    let uv = crate::lsp::find_executable("uv", &[]);
    let mut cmd = match (has_pip, uv, args.first()) {
        // A venv made by uv has no pip of its own: uv's pip, aimed at it.
        (false, Some(uv), Some(sub)) if venv.is_dir() => {
            let mut c = Process::new(uv);
            c.arg("pip")
                .arg(sub)
                .arg("--python")
                .arg(&python)
                .args(&args[1..]);
            c
        }
        _ => {
            let mut c = Process::new(&python);
            c.args(["-m", "pip"]).args(args);
            c
        }
    };
    cmd.current_dir(&root);
    spawn(&mut cmd)
}

/// How a check went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Ok,
    Warn,
    Fail,
}

fn report(mark: Mark, what: &str, detail: &str) {
    let tag = match mark {
        Mark::Ok => "  ok  ",
        Mark::Warn => " warn ",
        Mark::Fail => " FAIL ",
    };
    println!("[{tag}] {what}: {detail}");
}

/// Asks the interpreter for its version and the engine's.
const VERSIONS: &str = r#"
import sys
try:
    from importlib.metadata import version
    engine = version("automata-engine")
except Exception:
    engine = "-"
print(sys.version.split()[0], engine)
"#;

fn doctor(project_dir: Option<&Path>) -> ExitCode {
    let mut failed = false;
    let mut check = |mark: Mark, what: &str, detail: &str| {
        failed |= mark == Mark::Fail;
        report(mark, what, detail);
    };
    println!(
        "oberon {ENGINE_VERSION} ({} {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    match pollster::block_on(automata_renderer::probe_gpu()) {
        Ok(gpu) if gpu.software => check(
            Mark::Warn,
            "GPU",
            &format!(
                "{} ({}) renders on the CPU: it works, slowly. Install your graphics driver for speed",
                gpu.name, gpu.backend
            ),
        ),
        Ok(gpu) => check(Mark::Ok, "GPU", &format!("{} ({})", gpu.name, gpu.backend)),
        Err(e) => check(
            Mark::Warn,
            "GPU",
            &format!("none found ({e}); the editor and `oberon run` need one, `oberon test` doesn't"),
        ),
    }

    let bundle = Bundle::find();
    match (&bundle.python, bundle.engine_wheel()) {
        (Some(p), Some(_)) => check(Mark::Ok, "bundled Python", &p.display().to_string()),
        (Some(p), None) => check(
            Mark::Warn,
            "bundled Python",
            &format!(
                "{} is here but there's no automata-engine {ENGINE_VERSION} wheel in wheels/",
                p.display()
            ),
        ),
        (None, _) => check(
            Mark::Warn,
            "bundled Python",
            "not in this Oberon (a source build?): new projects use python3 from the PATH; \
             set OBERON_WHEELS to offer the engine wheel",
        ),
    }

    let root = project(project_dir);
    let is_project = ["pyproject.toml", "build.ron", ".venv"]
        .iter()
        .any(|m| root.join(m).exists());
    if !is_project {
        println!("\n(no project here; run `oberon doctor` inside one to check it)");
        return if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }
    println!("\nproject {}", root.display());
    match newproject::main_scene(&root) {
        Some(scene) => check(Mark::Ok, "main scene", &scene.display().to_string()),
        None => check(
            Mark::Warn,
            "main scene",
            "none (add level.ron or [tool.oberon] scene)",
        ),
    }
    match pyenv::venv_state(&root) {
        VenvState::Ready => check(
            Mark::Ok,
            ".venv",
            &format!("automata-engine {ENGINE_VERSION}"),
        ),
        VenvState::Outdated(v) => check(
            Mark::Warn,
            ".venv",
            &format!(
                "automata-engine {v}, Oberon is {ENGINE_VERSION}: run `oberon setup --upgrade`"
            ),
        ),
        VenvState::NoEngine => check(
            Mark::Fail,
            ".venv",
            "no automata-engine: run `oberon setup`",
        ),
        VenvState::Missing => check(Mark::Fail, ".venv", "missing: run `oberon setup`"),
    }
    let prefs = crate::project::Prefs::load();
    let python = pyenv::project_python(prefs.python.as_deref(), &root);
    match Process::new(&python)
        .arg("-c")
        .arg(VERSIONS)
        .current_dir(&root)
        .output()
    {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut parts = text.split_whitespace();
            let version = parts.next().unwrap_or("?").to_string();
            let engine = parts.next().unwrap_or("-").to_string();
            check(Mark::Ok, "Python", &format!("{python} ({version})"));
            if engine == "-" {
                check(
                    Mark::Fail,
                    "engine",
                    &format!("{python} can't import automata: run `oberon setup`"),
                );
            } else {
                check(
                    Mark::Ok,
                    "engine",
                    &format!("automata-engine {engine} imports"),
                );
            }
        }
        Ok(out) => check(
            Mark::Fail,
            "Python",
            &format!(
                "{python} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ),
        Err(e) => check(Mark::Fail, "Python", &format!("cannot run {python}: {e}")),
    }
    let dirs = crate::lsp::tool_dirs(&root, prefs.python.as_deref());
    let server = crate::lsp::AUTO_SERVERS
        .iter()
        .find_map(|(name, _)| crate::lsp::find_executable(name, &dirs));
    match (prefs.language_server.as_deref().map(str::trim), server) {
        (Some("off"), _) => check(
            Mark::Warn,
            "language server",
            "turned off in the preferences",
        ),
        (Some(cmd), _) if !cmd.is_empty() && cmd != "auto" => {
            check(
                Mark::Ok,
                "language server",
                &format!("{cmd} (from the preferences)"),
            );
        }
        (_, Some(path)) => check(Mark::Ok, "language server", &path.display().to_string()),
        (_, None) => check(
            Mark::Warn,
            "language server",
            "none: completions fall back to words (`oberon pip install basedpyright`)",
        ),
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_arguments() {
        let args = |a: &[&str]| {
            export_args(
                &a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
                "proj",
            )
        };
        assert_eq!(args(&[]), ["build", "proj"]);
        assert_eq!(
            args(&["--target", "web"]),
            ["build", "proj", "--target", "web"]
        );
        assert_eq!(
            args(&["game", "--target", "web"]),
            ["build", "game", "--target", "web"]
        );
        assert_eq!(args(&["serve"]), ["serve", "proj"]);
        assert_eq!(
            args(&["serve", "--port", "9000"]),
            ["serve", "proj", "--port", "9000"]
        );
        assert_eq!(
            args(&["serve", "build/web/Game"]),
            ["serve", "build/web/Game"]
        );
        assert_eq!(
            args(&["stage", "--zip", "g.zip"]),
            ["stage", "proj", "--zip", "g.zip"]
        );
        assert_eq!(args(&["keygen", "k.keystore"]), ["keygen", "k.keystore"]);
        assert_eq!(args(&["templates", "list"]), ["templates", "list"]);
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse(&[]).unwrap(), Command::Open(None));
        assert_eq!(
            parse(&args("open game/level.ron")).unwrap(),
            Command::Open(Some("game/level.ron".into()))
        );
        assert_eq!(
            parse(&args("level.ron")).unwrap(),
            Command::Open(Some("level.ron".into())),
            "the old `oberon scene.ron` still opens"
        );
        assert_eq!(
            parse(&args("new mygame --template 2d --no-venv")).unwrap(),
            Command::New {
                dir: "mygame".into(),
                template: Template::TwoD,
                python: None,
                no_venv: true
            }
        );
        assert_eq!(
            parse(&args("new mygame --python=/opt/py")).unwrap(),
            Command::New {
                dir: "mygame".into(),
                template: Template::ThreeD,
                python: Some("/opt/py".into()),
                no_venv: false
            }
        );
        assert_eq!(
            parse(&args("run --headless --frames 30")).unwrap(),
            Command::Run {
                scene: None,
                headless: true,
                frames: Some(30)
            }
        );
        assert_eq!(
            parse(&args("setup --upgrade")).unwrap(),
            Command::Setup {
                project: None,
                upgrade: true,
                python: None,
                no_uv: false
            }
        );
        // Arguments after these belong to someone else, --help included.
        assert_eq!(
            parse(&args("test -x --help")).unwrap(),
            Command::Test(args("-x --help"))
        );
        assert_eq!(
            parse(&args("pip install numpy")).unwrap(),
            Command::Pip(args("install numpy"))
        );
        assert_eq!(
            parse(&args("python -m site")).unwrap(),
            Command::Python(args("-m site"))
        );
        assert_eq!(
            parse(&args("export --target windows")).unwrap(),
            Command::Export(args("--target windows"))
        );
        assert_eq!(
            parse(&args("--bench a.ron")).unwrap(),
            Command::Bench(args("a.ron"))
        );
        assert_eq!(
            parse(&args("bench a.ron")).unwrap(),
            Command::Bench(args("a.ron"))
        );
        assert_eq!(parse(&args("doctor")).unwrap(), Command::Doctor(None));
        assert_eq!(parse(&args("--version")).unwrap(), Command::Version);
    }

    #[test]
    fn help_is_there_for_every_command() {
        assert_eq!(parse(&args("help")).unwrap(), Command::Help(None));
        assert_eq!(
            parse(&args("help new")).unwrap(),
            Command::Help(Some("new".into()))
        );
        assert_eq!(
            parse(&args("new --help")).unwrap(),
            Command::Help(Some("new".into()))
        );
        assert_eq!(
            parse(&args("run -h")).unwrap(),
            Command::Help(Some("run".into()))
        );
        for c in [
            "open", "new", "setup", "run", "test", "export", "bench", "python", "pip", "doctor",
        ] {
            let text = command_help(c).unwrap_or_else(|| panic!("no help for {c}"));
            assert!(text.starts_with(&format!("usage: oberon {c}")), "{c}");
            assert!(
                HELP.contains(&format!("oberon {c}")) || HELP.contains(&format!("oberon [{c}]")),
                "{c} is in the overview"
            );
        }
    }

    #[test]
    fn bad_command_lines_say_what_is_wrong() {
        assert!(parse(&args("new")).unwrap_err().contains("needs a NAME"));
        assert!(parse(&args("new a --template 4d"))
            .unwrap_err()
            .contains("unknown template"));
        assert!(parse(&args("new a b"))
            .unwrap_err()
            .contains("too many arguments"));
        assert!(parse(&args("run --frames many"))
            .unwrap_err()
            .contains("whole number"));
        assert!(parse(&args("run --fast"))
            .unwrap_err()
            .contains("unknown option --fast"));
        assert!(parse(&args("frobnicate"))
            .unwrap_err()
            .contains("unknown command"));
        assert!(parse(&args("--frobnicate"))
            .unwrap_err()
            .contains("unknown option"));
        assert!(parse(&args("setup --python"))
            .unwrap_err()
            .contains("needs a value"));
    }
}
