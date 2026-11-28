//! Oberon — the AUTOMATA scene editor and command line.
//!
//! A native desktop application: a winit window hosting a tiny-skia-drawn
//! editor UI with the engine's wgpu renderer driving the live viewport.
//! The same binary makes projects, runs and tests them, and exports games
//! (see `oberon::cli`):
//!
//! ```sh
//! oberon [open] [scene.ron|folder]          # the editor
//! oberon new mygame --template 3d           # a project with its own .venv
//! oberon run | test | export | doctor | …   # `oberon help` lists them
//! ```

// The editor binary links the safe library; presentation itself is safe
// (winit + wgpu surface). No `unsafe` is needed here either.
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    automata_core::logging::init("oberon=info,automata=info");
    let args: Vec<String> = std::env::args().skip(1).collect();
    oberon::cli::main(&args)
}
