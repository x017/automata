//! The launcher finds the game, runtime and interpreter in both export
//! layouts. Shell scripts stand in for Python: they print what they were
//! started with.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    std::env::consts::ARCH
};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("automata-player-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_exe(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in Python that reports its environment and arguments on stderr
/// and exits with 0.
const FAKE_PYTHON: &str = "#!/bin/sh\n\
echo \"program=$0\" >&2\n\
echo \"home=${PYTHONHOME:-none}\" >&2\n\
echo \"bytecode=${PYTHONDONTWRITEBYTECODE:-}\" >&2\n\
echo \"cwd=$(pwd)\" >&2\n\
for a in \"$@\"; do echo \"arg=$a\" >&2; done\n\
echo \"to stdout\"\n";

fn copy_player(to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_automata-player"), to).unwrap();
}

#[test]
fn an_app_bundle_runs_its_own_interpreter_and_logs_to_library_logs() {
    let dir = scratch("bundle");
    let contents = dir.join("Coin Run.app").join("Contents");
    let exe = contents.join("MacOS").join("Coin Run");
    copy_player(&exe);
    write_exe(
        &contents.join("MacOS").join(format!("python3-{ARCH}")),
        FAKE_PYTHON,
    );
    let resources = contents.join("Resources");
    std::fs::create_dir_all(resources.join("game")).unwrap();
    std::fs::write(resources.join("game").join("game.json"), "{}").unwrap();
    // The runtime's own interpreter must not be used in a bundle.
    let runtime = resources.join(format!("runtime-{ARCH}"));
    write_exe(&runtime.join("bin").join("python3"), "#!/bin/sh\nexit 9\n");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();

    // Not a terminal: the launcher becomes the interpreter, writing to the
    // log.
    let out = Command::new(&exe)
        .args(["--headless", "--frames", "3", "-psn_0_42"])
        .env("HOME", &home)
        .env("PYTHONHOME", "/elsewhere")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let log = home
        .join("Library")
        .join("Logs")
        .join("Coin Run")
        .join("last_run.log");
    let text = std::fs::read_to_string(&log).unwrap();
    let game = std::fs::canonicalize(resources.join("game")).unwrap();
    let runtime = std::fs::canonicalize(&runtime).unwrap();
    assert!(text.contains(&format!("python3-{ARCH}")), "{text}");
    assert!(
        text.contains(&format!("home={}", runtime.display())),
        "{text}"
    );
    assert!(text.contains("bytecode=1"), "{text}");
    assert!(text.contains(&format!("cwd={}", game.display())), "{text}");
    for arg in ["-m", "automata.player", "--headless", "--frames", "3"] {
        assert!(text.contains(&format!("arg={arg}\n")), "{arg}: {text}");
    }
    assert!(
        text.contains(&format!("arg={}\n", game.display())),
        "{text}"
    );
    assert!(!text.contains("-psn_"), "{text}");
    assert!(
        text.contains("to stdout"),
        "stdout goes to the log too: {text}"
    );
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    assert!(!contents.join("MacOS").join("logs").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_game_folder_runs_the_runtime_for_this_processor() {
    let dir = scratch("folder");
    let exe = dir.join("Coin Run");
    copy_player(&exe);
    std::fs::create_dir_all(dir.join("game")).unwrap();
    std::fs::write(dir.join("game").join("game.json"), "{}").unwrap();
    write_exe(
        &dir.join("runtime").join("bin").join("python3"),
        "#!/bin/sh\nexit 9\n",
    );
    write_exe(
        &dir.join(format!("runtime-{ARCH}"))
            .join("bin")
            .join("python3"),
        FAKE_PYTHON,
    );

    let out = Command::new(&exe)
        .arg("--headless")
        .env("PYTHONHOME", "/elsewhere")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let log = std::fs::read_to_string(dir.join("logs").join("last_run.log")).unwrap();
    for text in [&stderr, &log] {
        assert!(
            text.contains(&format!("runtime-{ARCH}/bin/python3")),
            "{text}"
        );
        assert!(text.contains("home=none"), "{text}");
        assert!(text.contains("arg=--headless\n"), "{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failing_game_passes_its_exit_code_on() {
    let dir = scratch("fail");
    let exe = dir.join("Game");
    copy_player(&exe);
    std::fs::create_dir_all(dir.join("game")).unwrap();
    std::fs::write(dir.join("game").join("game.json"), "{}").unwrap();
    write_exe(
        &dir.join("runtime").join("bin").join("python3"),
        "#!/bin/sh\necho Traceback >&2\nexit 4\n",
    );
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let log = std::fs::read_to_string(dir.join("logs").join("last_run.log")).unwrap();
    assert!(log.contains("Traceback"), "{log}");
    assert!(log.contains("exited with code 4"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}
