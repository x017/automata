//! Oberon — the AUTOMATA scene editor.
//!
//! A native desktop application: a winit window hosting a tiny-skia-drawn
//! editor UI with the engine's wgpu renderer driving the live viewport.
//!
//! Run with a scene file to open it, or no arguments for the starter scene:
//!
//! ```sh
//! oberon [scene.ron]
//! ```

// The editor binary links the safe library; presentation itself is safe
// (winit + wgpu surface). No `unsafe` is needed here either.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    automata_core::logging::init("oberon=info,automata=info");

    // Optional scene path argument.
    let scene = std::env::args_os().nth(1).map(PathBuf::from);
    if let Some(path) = &scene {
        match oberon::doc::Document::load(path) {
            Ok(_) => {
                tracing::info!(path = %path.display(), "opening scene");
            }
            Err(err) => {
                eprintln!("oberon: cannot open {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }

    match oberon::present::run(scene.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("oberon: {err}");
            eprintln!(
                "oberon: a window needs a display server (X11/Wayland). \
                 On a headless machine, render a still instead: \
                 `cargo run -p oberon --example screenshot -- out.png`"
            );
            ExitCode::FAILURE
        }
    }
}
