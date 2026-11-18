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

### Phase 1 — `automata-core`

| Item | Purpose |
|---|---|
| `Application` | Owns the engine lifecycle: `Created → Running → Shutdown` |
| `AppState` | Lifecycle state enum |
| `AppContext` | Per-frame context passed to the update callback |
| `EngineConfig` | Title, version, log filter, frame pacing |
| `Time` | Frame delta, elapsed time, frame count |
| `Error` / `Result` | Core error enum and result alias |
| `logging::init` | One-time `tracing` subscriber installation |

Canonical usage:

```rust
use automata_core::{Application, EngineConfig, Result};

fn main() -> Result<()> {
    let mut app = Application::new(EngineConfig::new("My Game"));
    app.run(|ctx| {
        // one call per frame
        if ctx.time().frame_count() >= 600 {
            ctx.request_quit();
        }
    })
}
```

Generated docs: `cargo doc --workspace --no-deps --open`.
