//! Minimal AUTOMATA application entry point.
//!
//! Opens a window and clears it to a dark background (the Phase 2
//! deliverable). Press Escape or close the window to quit. Pass `--headless`
//! to run the paced loop without a window or GPU — for machines without a
//! display.

use automata_core::{Application, EngineConfig, Result, WindowMode};
use automata_input::KeyCode;
use tracing::info;

/// Number of frames the demo runs in headless mode before quitting on its
/// own (two seconds at 60 fps). Windowed mode runs until the user quits.
const HEADLESS_DEMO_FRAMES: u64 = 120;

fn main() -> Result<()> {
    let headless = std::env::args().any(|arg| arg == "--headless");
    let window_mode = if headless {
        WindowMode::Headless
    } else {
        WindowMode::default()
    };

    let mut app = Application::new(EngineConfig {
        window_mode,
        ..EngineConfig::new("AUTOMATA")
    });

    app.run(|ctx| {
        let frame = ctx.time().frame_count();
        if frame % 60 == 0 {
            info!(
                frame,
                elapsed_seconds = ctx.time().elapsed_seconds(),
                "heartbeat"
            );
        }

        if ctx.input().just_pressed(KeyCode::Escape) {
            info!("escape pressed");
            ctx.request_quit();
        }

        if headless && frame >= HEADLESS_DEMO_FRAMES {
            ctx.request_quit();
        }
    })
}
