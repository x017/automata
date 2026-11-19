# API Reference

## Python API

The Python API — AUTOMATA's primary user-facing interface — landed in
**Phase 5** (PyO3 bindings, crate `automata-python`). Build and install it
with maturin:

```console
pip install maturin
maturin build --release --manifest-path crates/automata-python/Cargo.toml
pip install target/wheels/automata_engine-*.whl
```

### Example

```python
import automata as at

game = at.Game(title="Demo", resolution=(1280, 720))

game.load_scene(
    at.Scene()
    .add(at.Entity("cube").add(at.Transform(position=(0, 0, 0))))
    .add(at.Entity("camera").add(at.Camera3D(position=(0, 0.5, 4), target=(0, 0, 0))))
)

@game.update
def tick(ctx):
    if ctx.input.just_pressed("escape"):
        ctx.quit()
    cube = ctx.scene["cube"]            # like find(), but raises KeyError
    _, ry, _ = cube.transform.rotation_euler
    cube.transform.rotation_euler = (0, ry + ctx.time.delta, 0)

@game.on_event("hit")
def on_hit(payload):
    print("hit!", payload)              # runs after tick, once per event

game.run()  # uses the @game.update callback; game.run(fn) works too
```

The surface supports OO style (above), procedural style (plain functions
passed to `game.run(update)`), and functional style (`game.run(lambda ctx:
..., max_frames=10)`). Builders chain (`Scene().add(...).add(...)`),
components are mutable (`t.position = (1, 2, 0)`), and scene views support
`scene["name"]` indexing and iteration (`for e in ctx.scene`).

### Reference

| Item | Purpose |
|---|---|
| `Game(title, resolution, headless, resizable, vsync, log_filter, target_fps)` | Owns the engine; `run(update=None, max_frames=None)` drives the loop, `load_scene(scene)` replaces the scene, `quit()` requests shutdown, `state` reports the lifecycle; `@game.update` and `@game.on_event(name)` register callbacks |
| `Scene` / `Entity(name)` | Scene staging: `entity.add(component)`, `scene.add(entity)` |
| `Transform(position, rotation_euler, scale)` | Local transform; angles in radians (XYZ order) |
| `Camera3D(position, target, fov)` / `Camera2D(center, half_height)` | Frame cameras (first entity wins) |
| `FrameContext` (`ctx` in the callback) | `time`, `input`, `scene`, `events`, `resources`, `quit()` |
| `ctx.time` | `delta` / `elapsed` (seconds), `frame` (1-based frame number) |
| `ctx.input` | `pressed` / `just_pressed` / `just_released(key)`, `mouse_*` variants, `cursor`, `scroll`, `modifiers` |
| `ctx.scene` | `find(name)`, `["name"]`, `spawn(entity)`, `despawn(id)`, `entity_count`, `names()`, iteration; `EntityView.transform` get/set, attribute writes via `TransformRef` |
| `ctx.events` | `send(name, payload)` / `read(name)` — buffered: sent events are readable the next frame, then expire |
| `ctx.resources` | Immediate `set` / `get` / `remove` of arbitrary Python objects, keyed by name |
| `automata.Error` | Engine failures; exceptions raised by `update` propagate unchanged |

### Ownership and consistency rules

- `Game` owns the engine. `run` borrows it for the whole loop; a game that
  has shut down cannot run again (raises `automata.Error`).
- `Scene`/`Entity` are plain-data builders; `load_scene` copies them into
  the ECS world. After loading, entities are addressed by id or name.
- `FrameContext` carries **start-of-frame snapshots** for reads and a
  **write buffer** for changes (transforms, spawns, despawns, events).
  Buffered writes apply right after the callback returns and are visible
  from the next frame. Writes through `TransformRef` attributes compose
  field-by-field within the frame (`tref.position = ...; tref.scale = ...`
  does not clobber), and reads through them see pending same-frame writes.
  No live engine borrows cross into Python, so a context object that
  escapes its callback is stale but harmless.
- Key names are lowercase `snake_case` (`"w"`, `"arrow_left"`, `"f1"`,
  `"escape"`, ...); mouse buttons are `"left"`, `"right"`, `"middle"`,
  `"back"`, `"forward"`.

Type stubs for editors ship in `automata/_core.pyi`.

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

### Phase 3 — rendering

| Item | Purpose |
|---|---|
| `Renderer` | Pipelines, mesh/material/texture creation, frame submission from a `DrawList` |
| `DrawList` | Per-frame draw recording: `mesh`, `sprite`, `set_camera_2d/3d`, `clear_color` |
| `Camera2D` / `Camera3D` | Orthographic 2D and perspective 3D cameras |
| `Mesh` / `MeshData` / `Material` / `Texture` | GPU resource handles and CPU-side mesh data |

### Phase 4 — `automata-scene`

| Item | Purpose |
|---|---|
| `Scene` | hecs-backed world: `spawn`, `despawn`, `despawn_recursive`, `set_parent`, `update` |
| `Transform` / `GlobalTransform` / `Parent` | Local transform, propagated world transform, hierarchy link |
| `Events` | Double-buffered type-indexed event queues (`send`, `read`, `update`) |
| `Resources` | One-per-type store for singleton state |
| `Assets<T>` / `Handle<T>` | Generational asset registry; stale handles fail to resolve |
| `MeshRenderer` / `SpriteRenderer` / `record_draw_list` | Scene-to-renderer bridge |

Generated docs: `cargo doc --workspace --no-deps --open`.
