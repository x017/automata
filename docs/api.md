# API Reference

## Python API

The Python API — AUTOMATA's primary user-facing interface — landed in
**Phase 5** (PyO3 bindings, crate `automata-python`). Build and install
everything with one script:

```console
python scripts/build.py            # engine + Python wheel + install
python scripts/build.py --all      # also run the gates and test suites
python scripts/build.py --help     # all stages and flags
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

### Object-oriented style

`Game`, `Scene`, and `Entity` are subclassable. Engine options are class
attributes; lifecycle hooks are methods; methods named `on_<event>` are
auto-registered as event handlers:

```python
import automata as at

class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(0.0, 1.15, 0.0)))
        self.add(at.RigidBody.kinematic_velocity())
        self.add(at.Collider3D.capsule(0.5, 0.4))
        self.add(at.CharacterController(snap_to_ground=0.5))
        self.add(at.CharacterVelocity())

class Level(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(Player())

class Platformer(at.Game):
    title = "Platformer"
    resolution = (1280, 720)

    def on_start(self):                # once, before the first frame
        self.load_scene(Level())

    def on_update(self, ctx):          # every frame
        player = ctx.scene["player"]
        if ctx.input.just_pressed("escape"):
            ctx.quit()

    def on_hit(self, payload):         # every readable "hit" event
        print("hit!", payload)

Platformer().run()
```

The decorator and procedural styles keep working unchanged; constructor
keyword arguments override class attributes. See
`examples/python/oop_platformer.py` for a complete game.

The surface supports OO style (above), procedural style (plain functions
passed to `game.run(update)`), and functional style (`game.run(lambda ctx:
..., max_frames=10)`). Builders chain (`Scene().add(...).add(...)`),
components are mutable (`t.position = (1, 2, 0)`), and scene views support
`scene["name"]` indexing and iteration (`for e in ctx.scene`).

### Loading scenes made in Oberon

The Oberon editor saves the engine's RON scene-file format (Ctrl+S writes
`scene.ron`, then `scene-2.ron`, … in the directory Oberon was started
from), so its files load directly:

```python
import automata as at

game = at.Game(title="my level")
game.load_scene(at.Scene.load("scene.ron"))   # IOError / ValueError on failure

def update(ctx):
    player = ctx.scene["Player"]              # entity names from the Hierarchy
    player.character_velocity = (3.0, -2.0, 0.0)

game.run(update)
```

Physics, meshes, lights and Character Controllers come across as authored;
gameplay (input → movement, cameras) is your Python code.
`examples/python/play_oberon_scene.py scene.ron` reproduces Oberon's Play
mode (WASD/arrows, Space, Shift, follow camera) as a starting point.

### Behaviour scripts

Attach a script to an entity in Oberon (Inspector → **Script → New…**).
A script is a plain module with optional `start(entity, ctx)`,
`update(entity, ctx)` and `stop(entity, ctx)` functions:

```python
# scripts/spinner.py
def start(entity, ctx):
    entity.state["speed"] = entity.attributes.get("speed", 1.5)

def update(entity, ctx):
    rx, ry, rz = entity.transform.rotation_euler
    entity.transform.rotation_euler = (rx, ry + entity.state["speed"] * ctx.time.delta, rz)
    if ctx.input.just_pressed("space"):
        print("boing")                      # → Oberon's Console
```

In Oberon's Play mode the scripts run in a separate Python process bridged
to the editor (any `python3`; set `python` in `~/.config/oberon/prefs.ron`
to pick one). Saving a script while playing hot-reloads it, and errors show
in the Console with a clickable `File "…", line N`. The same files run in
the engine with:

```python
at.run_scene("level.ron")                  # Game options as keywords
```

`entity` has `name`, `transform`, `character_velocity`, `attributes`,
`grounded` and a persistent `state` dict; `ctx` has `input`, `time`,
`physics`, `scene`, `log(...)` (plus `events`, `resources`, `quit()` in
the engine). Scripted characters skip Oberon's built-in WASD controller.

### Attributes and prefabs

Custom **attributes** added in Oberon's inspector (bool, int, float, text,
vector, color) travel with the scene file and read like a dict — on staged
entities and live ones alike:

```python
enemy = ctx.scene["Enemy"]
hp = enemy.attributes.get("health", 100)
enemy.attributes["health"] = hp - 10      # buffered, like transform writes
enemy.script                              # "scripts/enemy.py" if attached
```

Values map to `bool`, `int`, `float`, `str`, `(x, y, z)` and `(r, g, b, a)`.

**Prefabs** (`.prefab` files saved from Oberon's Hierarchy, or
`Prefab(entity).save(path)` from code) are reusable entity templates:

```python
crate = at.Prefab.load("prefabs/crate.prefab")
scene.add(crate.instantiate(position=(2, 0, 0), name="crate_1"))   # staged
ctx.scene.spawn(crate, position=(0, 5, 0))                         # at runtime
```

### Reference

| Item | Purpose |
|---|---|
| `Game(title, resolution, headless, resizable, vsync, log_filter, target_fps, clear_color, graphics)` | Owns the engine; `run(update=None, max_frames=None)` drives the loop, `load_scene(scene)` replaces the scene, `quit()` requests shutdown, `state` reports the lifecycle; `@game.update` and `@game.on_event(name)` register callbacks; `clear_color=(r, g, b, a)` sets the window background (default very dark blue); `graphics=GraphicsQuality.HIGH` picks a quality preset (`LOW` disables shadows) |
| `Key` / `MouseButton` / `GamepadButton` / `GamepadAxis` / `GraphicsQuality` | Typed `str` enums for every bindable input and quality preset — `Binding.key(Key.SPACE)`, `AxisBinding.key_pair(Key.A, Key.D)`; plain strings (`"space"`) keep working everywhere |
| `Scene` / `Entity(name)` | Scene staging: `entity.add(component)`, `scene.add(entity_or_prefab)`, `scene.find(name)`, `scene.entities()`; `entity.attributes` (dict-like), `entity.script` |
| `run_scene(path, max_frames=None, update=None, **game_options)` | Runs an Oberon scene with its behaviour scripts (`start/update/stop(entity, ctx)`) |
| `Attributes` | Dict-like custom values: `[key]`, `get`, `in`, `del`, `keys/values/items`, `update`, `clear`, `to_dict` |
| `Prefab(entity_or_list)` | Entity template: `Prefab.load(path)`, `save(path)`, `instantiate(position, name)`, `instantiate_all(position)`, `entities()` |
| `Transform(position, rotation_euler, scale)` | Local transform; angles in radians (XYZ order) |
| `Camera3D(position, target, fov)` / `Camera2D(center, half_height)` | Frame cameras (first entity wins) |
| `MeshRenderer` | `MeshRenderer.cube(size=…, color=…)` / `.quad(…)` — plain-data visuals; the windowed loop materializes them on the GPU, headless runs ignore them |
| `DirectionalLight` | `DirectionalLight(direction=…, color=…, ambient=…, shadows=True)` — the scene's sun; the first entity carrying one lights the frame with Blinn-Phong shading and shadow-mapped shadows |
| `FrameContext` (`ctx` in the callback) | `time`, `input`, `scene`, `events`, `resources`, `quit()` |
| `ctx.time` | `delta` / `elapsed` (seconds), `frame` (1-based frame number) |
| `ctx.input` | `pressed` / `just_pressed` / `just_released(key)`, `mouse_*` variants, `cursor`, `scroll`, `modifiers`, `gamepads` |
| `ctx.input.gamepads` | Connected pads: `ids()`, `get(id)`, `primary()`, `len()`, iteration; state: `pressed(id, btn)` / `pressed_any(btn)`, `axis(id, ax)` / `axis_any(ax)`; `pad.left_stick()` / `right_stick()`; `events` — this frame's gamepad events |
| `ActionMap` / `Binding` / `AxisBinding` | Action-based input: `map.bind("jump", Binding.key("space"))`, `map.bind_axis("move_x", AxisBinding.key_pair("a", "d"))`, then `map.pressed("jump", ctx.input)` / `map.axis("move_x", ctx.input)`; repeated `bind`/`bind_axis` calls stack (WASD + arrows + stick can share one axis; values sum, clamped to -1..1); `rebind` / `unbind` / `unbind_axis` for remapping |
| `ctx.scene` | `find(name)`, `["name"]`, `spawn(entity_or_prefab, position=None)`, `despawn(id)`, `entity_count`, `names()`, iteration; `EntityView.transform` get/set (attribute writes via `TransformRef`), `EntityView.character_velocity` get/set (per-frame drive for character controllers), `EntityView.camera_3d` get/set (follow cameras: assign a fresh `Camera3D` each frame), `EntityView.attributes` (buffered writes), `EntityView.script` |
| `ctx.events` | `send(name, payload)` / `read(name)` — buffered: sent events are readable the next frame, then expire |
| `ctx.resources` | Immediate `set` / `get` / `remove` of arbitrary Python objects, keyed by name |
| `ctx.physics` (3D) | `cast_ray(origin, dir, max_distance, exclude)` → result key (read next frame via `ray_result(key)`), `apply_impulse` / `apply_force` / `set_velocity(entity, vec)`, `velocity(entity)`, `grounded(entity)`, `gravity()` / `set_gravity(vec)`, `body_count()` |
| `ctx.physics2d` | `apply_impulse(entity, vec2)`, `grounded(entity)`, `body_count()` |
| `RigidBody` | `RigidBody.dynamic() / .fixed() / .kinematic_velocity() / .kinematic_position()`, or `RigidBody(type, gravity_scale=…, linear_damping=…)` |
| `Collider3D` / `Collider2D` | `.ball(r)`, `.cuboid(x, y[, z])`, `.capsule(half_height, r)`, 3D only: `.mesh(model)`, `.convex_hull(model)`, chainable `.sensor()/.density()/.friction()/.restitution()` |
| `CharacterController` | `CharacterController(mode=…, max_slope_climb_angle=…, autostep=…, snap_to_ground=…)` — kinematic body mover |
| `CharacterVelocity` | `CharacterVelocity((vx, vy, vz))` — set `.velocity` each frame to drive the controller |
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

## Phase 6 — Physics and Input

### Physics model

- Attach a `RigidBody` plus a `Collider3D` (3D) or `Collider2D` (2D) to an
  entity to make it physical. The engine syncs the physics world from the
  scene, steps it at a fixed timestep (`EngineConfig::physics_timestep`,
  default 1/60 s), and writes resulting transforms back.
- Entities with a 2D collider live in the 2D world; those with a 3D
  collider (or none) live in the 3D world. A single scene can mix both.
- Collision events (`PhysicsEvent::{CollisionStarted, CollisionStopped}`)
  and contact-force events arrive on the event bus.
- Kinematic character controllers (`CharacterController` +
  `CharacterVelocity`) move with slide/slope/autostep/snap-to-ground logic;
  `ctx.physics.grounded(entity)` reports ground contact. Drive a character
  per frame with `view.character_velocity = (vx, vy, vz)` (buffered like
  transform writes); game code composes gravity/jumps into the velocity.
- Scripts never see Rapier: the engine owns the simulation and exposes
  queries/mutations through the frame context only.

### Input model

- Keyboard/mouse state: `ctx.input.pressed(key)` / `just_pressed` /
  `just_released`, plus mouse buttons, cursor, scroll, modifiers.
- Gamepads (gilrs): `ctx.input.gamepads` — connected pads by id
  (`ids`, `get`, `primary`), button/axis state (`pressed`, `pressed_any`,
  `axis`, `axis_any`), sticks (`pad.left_stick()`), and this frame's
  connection/button/axis events (`gamepads.events`).
- Action mapping: `automata.ActionMap` binds named actions and axes to any
  device (`Binding.key` / `mouse` / `gamepad_button` / axis thresholds,
  `AxisBinding.gamepad_axis` / `key_pair` / `button_pair`), evaluates
  against `ctx.input`, and is remappable at runtime (`bind`, `rebind`,
  `unbind`). Game code never touches raw device ids.
