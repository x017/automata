//! Minimal AUTOMATA application entry point.
//!
//! Phase 1 deliverable: demonstrates the engine lifecycle — initialization,
//! a paced main loop, and a clean shutdown — without opening a window.
//! Phase 2 replaces the loop internals with a `winit`-driven event loop.

use automata_core::{Application, EngineConfig, Result};
use tracing::info;

/// Number of frames the demo runs before quitting (two seconds at 60 fps).
const DEMO_FRAMES: u64 = 120;

fn main() -> Result<()> {
    let mut app = Application::new(EngineConfig::new("AUTOMATA"));

    app.run(|ctx| {
        let frame = ctx.time().frame_count();
        if frame % 60 == 0 {
            info!(
                frame,
                elapsed_seconds = ctx.time().elapsed_seconds(),
                "heartbeat"
            );
        }
        if frame >= DEMO_FRAMES {
            ctx.request_quit();
        }
    })
}
