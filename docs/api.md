# API Reference

## Python API

The Python API — AUTOMATA's primary user-facing interface — is introduced in
**Phase 5** (PyO3 integration). The design target is the sketch in
`build.md` section 3.10:

```python
import automata as at

game = at.Game(title="Project Zero", resolution=(1920, 1080))

scene = at.Scene()
player = at.Entity("Player")
player.add(at.Camera())
player.add(at.FirstPersonController())
scene.add(player)

game.load_scene(scene)
game.run()
```

Until Phase 5 lands, this document tracks the native Rust API only.

## Native API (Rust)

### Phase 1–2 — `automata-core`, `automata-input`, `automata-renderer`

| Item | Purpose |
|---|---|
| `Application` | Owns the engine lifecycle: `Created → Running → Shutdown` |
| `AppState` | Lifecycle state enum |
| `AppContext` | Per-frame context: `time()`, `input()`, `request_quit()` |
| `EngineConfig` | Title, version, log filter, frame pacing, window mode |
| `WindowMode` / `WindowConfig` | Windowed (default) vs headless; size, resizable, vsync |
| `Time` | Frame delta, elapsed time, frame count |
| `Error` / `Result` | Core error enum and result alias |
| `InputState` | Per-frame keyboard/mouse state: `pressed`, `just_pressed`, `just_released`, buttons, cursor, scroll, modifiers |
| `RenderContext` | wgpu device/queue/surface; `resize`, `render_clear` |
| `logging::init` | One-time `tracing` subscriber installation |

Canonical usage:

```rust
use automata_core::{Application, EngineConfig, Result, WindowMode};
use automata_input::KeyCode;

fn main() -> Result<()> {
    let config = EngineConfig {
        // Headless runs without a window/GPU: tests, CI, tooling.
        window_mode: WindowMode::default(),
        ..EngineConfig::new("My Game")
    };
    let mut app = Application::new(config);
    app.run(|ctx| {
        // one call per frame
        if ctx.input().just_pressed(KeyCode::Escape) {
            ctx.request_quit();
        }
    })
}
```

Generated docs: `cargo doc --workspace --no-deps --open`.
