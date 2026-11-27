//! The launcher of an exported AUTOMATA game.
//!
//! An exported game is a folder:
//!
//! ```text
//! MyGame(.exe)      this program, renamed
//! game/             scenes, scripts, assets, and game.json
//! runtime/          a portable Python with the engine installed
//! logs/last_run.log written on every run
//! ```
//!
//! On macOS the game is an app bundle instead, and the log goes to
//! `~/Library/Logs/<name>/last_run.log` (Console.app lists it):
//!
//! ```text
//! MyGame.app/Contents/MacOS/MyGame          this program, renamed
//! MyGame.app/Contents/MacOS/python3-arm64   the interpreter per processor
//! MyGame.app/Contents/Resources/game/
//! MyGame.app/Contents/Resources/runtime-arm64/
//! ```
//!
//! The launcher runs `python -m automata.player game/` with the bundled
//! runtime and passes its own arguments through (`--headless`,
//! `--frames N`). A `runtime-<arch>/` folder for this processor wins over
//! `runtime/`. Without a bundled runtime (a development template) it falls
//! back to `$AUTOMATA_PYTHON`, then `python3` / `python` on `PATH`.
//!
//! In an app bundle the interpreter sits in `Contents/MacOS`, so macOS
//! treats the running game as the app (its name, icon and Dock tile). When
//! started from Finder (no terminal) the launcher *becomes* the interpreter
//! (`exec`), writing straight to the log, so there is one process; from a
//! terminal it runs it as a child and copies its output to the log too.

// No console window behind a released Windows game.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// This processor in template folder names (`runtime-arm64`,
/// `python3-x86_64`).
const ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    std::env::consts::ARCH
};

/// Where the game's files and log are.
struct Layout {
    /// The folder holding `game/` and the runtime.
    root: PathBuf,
    /// The folder holding the executable.
    exe_dir: PathBuf,
    /// The log file.
    log: PathBuf,
    /// Whether this is a macOS app bundle.
    bundle: bool,
}

impl Layout {
    /// The layout around the executable `exe`: an app bundle when it sits
    /// in `Contents/MacOS` next to `Contents/Resources/game`, otherwise a
    /// game folder.
    fn of(exe: &Path) -> Self {
        let exe_dir = exe
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let contents = exe_dir
            .parent()
            .filter(|_| exe_dir.file_name().is_some_and(|n| n == "MacOS"))
            .filter(|c| c.file_name().is_some_and(|n| n == "Contents"));
        if let Some(resources) = contents
            .map(|c| c.join("Resources"))
            .filter(|r| r.join("game").is_dir())
        {
            let name = exe
                .file_name()
                .map_or_else(|| "AUTOMATA".into(), |n| n.to_string_lossy().into_owned());
            let logs = std::env::var_os("HOME").map_or_else(std::env::temp_dir, |h| {
                PathBuf::from(h).join("Library").join("Logs")
            });
            return Self {
                root: resources,
                exe_dir,
                log: logs.join(name).join("last_run.log"),
                bundle: true,
            };
        }
        Self {
            log: exe_dir.join("logs").join("last_run.log"),
            root: exe_dir.clone(),
            exe_dir,
            bundle: false,
        }
    }
}

/// The bundled runtime: `runtime-<arch>/` for this processor, else
/// `runtime/`.
fn runtime(root: &Path) -> Option<PathBuf> {
    [format!("runtime-{ARCH}"), "runtime".to_string()]
        .iter()
        .map(|d| root.join(d))
        .find(|d| d.is_dir())
}

/// The interpreter inside a runtime.
fn runtime_python(runtime: &Path) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &["pythonw.exe", "python.exe"]
    } else {
        &["bin/python3", "bin/python"]
    };
    candidates
        .iter()
        .map(|c| runtime.join(c))
        .find(|p| p.is_file())
}

/// The interpreter to run.
struct Python {
    program: PathBuf,
    /// `PYTHONHOME`, for an interpreter that lives outside its runtime.
    home: Option<PathBuf>,
    bundled: bool,
}

/// The interpreter to run: the app bundle's, the runtime's, then
/// `$AUTOMATA_PYTHON`, then `PATH`.
fn python(layout: &Layout) -> Python {
    if let Some(rt) = runtime(&layout.root) {
        let inside = layout.exe_dir.join(format!("python3-{ARCH}"));
        if layout.bundle && inside.is_file() {
            return Python {
                program: inside,
                home: Some(rt),
                bundled: true,
            };
        }
        if let Some(program) = runtime_python(&rt) {
            return Python {
                program,
                home: None,
                bundled: true,
            };
        }
    }
    let program = std::env::var_os("AUTOMATA_PYTHON").map_or_else(
        || PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        PathBuf::from,
    );
    Python {
        program,
        home: None,
        bundled: false,
    }
}

/// Drops Windows' verbatim prefix (`\\?\C:\…` → `C:\…`,
/// `\\?\UNC\host\…` → `\\host\…`) that `canonicalize` adds. In verbatim
/// form `/` is not a separator, so Python opening `game\scripts/x.py` fails.
/// Other paths are returned unchanged.
fn plain_path(p: PathBuf) -> PathBuf {
    let plain = p.to_str().and_then(|s| {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            Some(format!(r"\\{rest}"))
        } else {
            s.strip_prefix(r"\\?\")
                .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
                .map(str::to_string)
        }
    });
    plain.map_or(p, PathBuf::from)
}

/// The executable's own path, links resolved.
fn exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map_or_else(|| PathBuf::from("automata-player"), plain_path)
}

fn main() -> ExitCode {
    let layout = Layout::of(&exe());
    let code = launch(&layout);
    if code != 0 {
        show_crash_log(&layout.log);
    }
    ExitCode::from(code)
}

/// Windows release builds have no console, so a crash would vanish without
/// a trace: open the log so the player (or developer) sees what happened.
#[cfg(all(windows, not(debug_assertions)))]
fn show_crash_log(log: &Path) {
    let _ = Command::new("notepad.exe").arg(log).spawn();
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn show_crash_log(_log: &Path) {}

/// Replaces this process with the interpreter `cmd` runs, writing all its
/// output straight to `log`, so the game is a single process that macOS
/// knows as this app. Returns only when the interpreter can't start.
#[cfg(unix)]
fn become_python(cmd: &mut Command, log: Option<std::fs::File>, log_path: &Path) -> u8 {
    if let (Some(out), Some(err)) = (
        log.as_ref().and_then(|f| f.try_clone().ok()),
        log.as_ref().and_then(|f| f.try_clone().ok()),
    ) {
        cmd.stdout(out).stderr(err);
    }
    drop(log);
    let err = std::os::unix::process::CommandExt::exec(cmd);
    let line = format!(
        "automata-player: could not start {}: {err}",
        Path::new(cmd.get_program()).display()
    );
    eprintln!("{line}");
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(log_path) {
        let _ = writeln!(f, "{line}");
    }
    3
}

/// Copies `from` to our stderr and the log until it closes.
fn tee(mut from: impl Read, mut log: Option<&mut std::fs::File>) {
    let mut chunk = [0u8; 4096];
    loop {
        match from.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let _ = std::io::stderr().write_all(&chunk[..n]);
                if let Some(f) = &mut log {
                    let _ = f.write_all(&chunk[..n]);
                }
            }
        }
    }
}

/// Runs the game; returns the process exit code.
fn launch(layout: &Layout) -> u8 {
    let game = layout.root.join("game");
    if let Some(dir) = layout.log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut log = std::fs::File::create(&layout.log).ok();
    let mut note = |line: &str| {
        eprintln!("{line}");
        if let Some(f) = &mut log {
            let _ = writeln!(f, "{line}");
        }
    };

    if !game.join("game.json").is_file() {
        note(&format!(
            "automata-player: no game found at {} (expected game/game.json next to the executable)",
            game.display()
        ));
        return 2;
    }
    let Python {
        program: py,
        home,
        bundled,
    } = python(layout);
    if !bundled {
        note(&format!(
            "automata-player: no bundled runtime; using {}",
            py.display()
        ));
    }

    let mut cmd = Command::new(&py);
    cmd.arg("-X")
        .arg("utf8")
        .arg("-m")
        .arg("automata.player")
        .arg(&game)
        // (Old macOS passes Finder's process serial number: not ours.)
        .args(
            std::env::args_os()
                .skip(1)
                .filter(|a| !a.to_string_lossy().starts_with("-psn_")),
        )
        .current_dir(&game)
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped());
    if bundled {
        // Ignore a system Python's environment entirely.
        cmd.env_remove("PYTHONHOME").env_remove("PYTHONPATH");
    }
    if let Some(home) = &home {
        cmd.env("PYTHONHOME", home);
    }
    // An app started from Finder has no terminal to show output in.
    #[cfg(unix)]
    if layout.bundle && !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        return become_python(&mut cmd, log.take(), &layout.log);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            note(&format!(
                "automata-player: could not start {}: {e}",
                py.display()
            ));
            return 3;
        }
    };

    // Tee the game's stderr (logs, tracebacks) to our stderr and the log.
    if let Some(err) = child.stderr.take() {
        tee(err, log.as_mut());
    }
    match child.wait() {
        Ok(status) if status.success() => 0,
        Ok(status) => {
            let code = status.code().unwrap_or(1);
            if let Some(f) = &mut log {
                let _ = writeln!(f, "automata-player: the game exited with code {code}");
            }
            u8::try_from(code.clamp(1, 255)).unwrap_or(1)
        }
        Err(e) => {
            if let Some(f) = &mut log {
                let _ = writeln!(f, "automata-player: {e}");
            }
            1
        }
    }
}
