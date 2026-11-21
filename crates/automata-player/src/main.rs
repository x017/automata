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
//! The launcher runs `python -m automata.player game/` with the bundled
//! runtime and passes its own arguments through (`--headless`,
//! `--frames N`). Without a bundled runtime (a development template) it
//! falls back to `$AUTOMATA_PYTHON`, then `python3` / `python` on `PATH`.

// No console window behind a released Windows game.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// The bundled interpreter, relative to the game folder.
fn bundled_python(root: &Path) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &["runtime/pythonw.exe", "runtime/python.exe"]
    } else {
        &["runtime/bin/python3", "runtime/bin/python"]
    };
    candidates
        .iter()
        .map(|c| root.join(c))
        .find(|p| p.is_file())
}

/// The interpreter to run: bundled, then `$AUTOMATA_PYTHON`, then `PATH`.
fn python(root: &Path) -> (PathBuf, bool) {
    if let Some(p) = bundled_python(root) {
        return (p, true);
    }
    if let Some(p) = std::env::var_os("AUTOMATA_PYTHON") {
        return (PathBuf::from(p), false);
    }
    let fallback = if cfg!(windows) { "python" } else { "python3" };
    (PathBuf::from(fallback), false)
}

/// The folder the executable lives in.
fn root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> ExitCode {
    let root = root();
    let log_path = root.join("logs").join("last_run.log");
    let code = launch(&root, &log_path);
    if code != 0 {
        show_crash_log(&log_path);
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

/// Runs the game; returns the process exit code.
fn launch(root: &Path, log_path: &Path) -> u8 {
    let game = root.join("game");
    if let Some(dir) = log_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut log = std::fs::File::create(log_path).ok();
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
    let (py, bundled) = python(root);
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
        .args(std::env::args_os().skip(1))
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
    if let Some(mut err) = child.stderr.take() {
        let mut chunk = [0u8; 4096];
        loop {
            match err.read(&mut chunk) {
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
