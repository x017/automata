# API Reference

Every public name in the `automata` package. Conventions: angles are
radians (`Transform`) or degrees (`Camera3D.fov`); colors are float tuples
in `0.0..=1.0`; `Vec3 = tuple[float, float, float]`.

```python
import automata as at
at.__version__          # engine version string
at.Error                # engine runtime failure exception
```

## Game

```python
at.Game(
    title="AUTOMATA",
    resolution=(1280, 720),
    headless=False,                     # no window/GPU; for tests and CI
    resizable=True,
    vsync=True,
    log_filter=None,                    # e.g. "debug"; RUST_LOG overrides
    target_fps=None,                    # int fps cap; None = engine default 60
    clear_color=None,                   # (r, g, b, a) window background
    graphics=None,                      # at.GraphicsQuality.HIGH etc.
    fixed_delta=None,                   # headless: every frame advances exactly this many seconds
)
```

| Member | Type | Notes |
|---|---|---|
| `run(update=None, max_frames=None) -> int` | method | drives the loop; returns frames run; a Game runs once |
| `load_scene(scene)` | method | replaces the world: at once before `run`, at the end of the frame inside `update` or a handler |
| `quit()` | method | requests shutdown |
| `state` | property | `"created"` / `"running"` / `"shutdown"` |
| `update(fn)` | decorator | registers the per-frame callback |
| `on_event(name)` | decorator | registers a handler for events named `name` |

OOP subclass hooks: `on_start()`, `on_update(ctx)`, `on_<event>(payload)`.
Class attributes mirror every constructor option; kwargs win over them.

## Scene & Entity (staging)

```text
scene = at.Scene().add(entity)     # chainable
len(scene)  scene.entity_count

scene.to_ron() -> str              # staged scene -> RON text
scene.save(path)                   # write the RON scene file
at.Scene.from_ron(text) -> Scene   # ValueError on bad/newer files
at.Scene.load(path) -> Scene       # IOError on missing files

game.scene_to_ron() -> str         # the LIVE world (post-physics) -> RON
game.save_scene(path)              # same, straight to disk

entity = at.Entity(name=None)      # name optional, used for ctx.scene lookup
entity.add(component)              # chainable; TypeError for unknown types
entity.name                        # get/set
```

See [Guide: Scene Files](guides/scene-files.md) for the format and its
durability rules.

`at.Component` — the union of everything `add` accepts (for annotations).

### Transform

```text
at.Transform(position=(0, 0, 0), rotation_euler=(0, 0, 0), scale=(1, 1, 1))
t.position / t.rotation_euler / t.scale      # get/set (radians, XYZ order)
t.copy() -> Transform
```

### Cameras

```python
at.Camera3D(position=(0, 0, 0), target=(0, 0, -1), fov=60.0)   # fov in degrees
at.Camera2D(center=(0.0, 0.0), half_height=100.0)
```

Read-only properties: `position`, `target`, `fov` / `center`,
`half_height`. The first camera entity in the scene wins each frame;
replace per frame via `EntityView.camera_3d`.

### Visuals

```python
at.MeshRenderer.cube(size=1.0, color=(1, 1, 1, 1))
at.MeshRenderer.sphere(size=1.0, color=(1, 1, 1, 1))  # size = diameter
at.MeshRenderer.quad(size=1.0, color=(1, 1, 1, 1))
at.MeshRenderer.model(path, color=(1, 1, 1, 1))   # .glb / .gltf / .obj
# every constructor also takes the keyword-only surface arguments:
#   roughness=None, metallic=None, emissive=(0, 0, 0), emissive_strength=1.0,
#   glow_light=1.0
m.kind                      # "cube", "sphere", "quad" or "model"
m.color                     # (r, g, b, a); tints a model
m.roughness                 # 0 = mirror .. 1 = chalk, or None (default)
m.metallic                  # 0 = plastic/wood/stone, 1 = metal, or None
m.emissive                  # (r, g, b) glow colour, (0, 0, 0) = none
m.emissive_strength         # glow multiplier
m.glow_light                # how much the glow lights its surroundings
m.path                      # the model file, or None
m.bounds                    # ((min x, y, z), (max x, y, z)) or None

at.set_asset_root(path)     # where relative model paths resolve (None resets)
at.asset_root()             # the current root, or None
```

Plain data; materialized on the GPU by the windowed driver, inert headless.
`Transform.scale` sizes the primitive.

Surface arguments: `None` roughness / metallic means "default", which is
0.6 / 0 for primitives and the file's own material values for models.
Setting them overrides every part of a model. Roughness and metallic must be
in `0..=1`, and emissive values must not be negative, or `ValueError` is
raised. A glowing mesh also lights the surfaces around it like a real
glowing object of its size; `glow_light` scales that (0 = off, no shadows).
The glow added is `emissive × emissive_strength`; above 1 it blooms
on Ultra and Beautiful. Guide: [Rendering & Lights](guides/rendering-and-lights.md#surfaces).

`model` loads the file right away, so a missing file raises
`FileNotFoundError` and a broken one raises `ValueError` where you create the
component. Relative paths resolve against the asset root.
`Scene.load` / `run_scene` set the root to the scene's folder. If no root is
set, the folder of the running script is used, then the working directory.
The file is cached, so a thousand trees load it once. Guide:
[Models](oberon/models.md).

### DirectionalLight

```python
at.DirectionalLight(direction=(-0.5, -1.0, -0.35),
                    color=(1.0, 1.0, 1.0), ambient=0.3, shadows=True)
```

The scene's sun; the first entity carrying one lights the frame. Getters:
`direction`, `color`, `ambient`, `shadows`. Without one, a neutral
shadowless default is used.

### Environment

```python
at.Environment(sky_color=(1, 1, 1), ground_color=(1, 1, 1),
               background=(0.012, 0.012, 0.012), exposure=1.0,
               tonemapping="none", fog=False, fog_color=(0.5, 0.55, 0.6),
               fog_density=0.02, fog_start=0.0, sky=False,
               zenith_color=(0.16, 0.34, 0.78), sun_size=2.0, sun_glow=1.0,
               clouds=0.0, lens_flare=1.0)
at.Environment.preset("neutral" | "daylight" | "dusk" | "night")
```

The scene look. The first entity carrying one wins. It sets hemisphere
ambient (sky above, ground below, scaled by the sun's `ambient`), the
background clear colour (overriding `Game.clear_color`), exposure, tone
mapping (`"none"`, `"reinhard"`, `"aces"`, `"agx"`), and exponential
distance fog. `sky=True` draws a sky dome instead of the flat background:
a gradient from `background` (the horizon) to `zenith_color`, a sun disk
`sun_size` degrees wide where the scene's `DirectionalLight` shines from,
a halo (`sun_glow`), `clouds` cover from 0 to 1, and on `BEAUTIFUL` a lens
flare (`lens_flare` strength) when the camera looks toward the sun. The
non-neutral presets turn the sky on. All arguments are readable as properties. Unknown tone
mapping or preset names raise `ValueError`. It's saved in scene files.
Guide: [Rendering & Lights](guides/rendering-and-lights.md#environment).

### Water

```python
at.Water(size=(20, 20), deep_color=(0.01, 0.07, 0.11),
         shallow_color=(0.1, 0.42, 0.44), clarity=2.5, wave_height=0.12,
         wave_length=5.0, wave_speed=1.0, wave_direction=30.0, foam=0.6,
         density=2.0, drag=1.0, endless=False)
at.Water.preset("pond" | "lake" | "ocean" | "sea" | "default")
```

A rectangle of animated water on the entity's local XZ plane, `size`
metres wide, centred on its `Transform`. Waves are Gerstner waves
(`wave_height` crest to rest, `wave_length` of the main wave, travelling
toward `wave_direction` degrees around +Y). Every quality preset draws it;
`BEAUTIFUL` adds refraction of what's underneath, depth-based absorption
(`clarity` is how many metres you can see down) and shoreline `foam`.
`endless=True` (on the ocean and sea presets) continues the surface,
flat and calm, out to the horizon; waves, foam and physics stay within
`size`. It's also a volume of water for physics. Dynamic bodies in it float by
buoyancy (`density` against their colliders' density, which defaults to
1.0, so a default box floats half under) and are slowed by `drag`.
All arguments are readable as properties, and it's saved in scene files.
Guides: [Rendering & Lights](guides/rendering-and-lights.md#water),
[Physics](guides/physics.md#water).

### Physics components

```text
at.RigidBody(body_type="dynamic", *, gravity_scale=1.0,
             linear_damping=0.0, angular_damping=0.0, enabled=True)
at.RigidBody.dynamic() / .fixed() / .kinematic_velocity() / .kinematic_position()
body.body_type

at.Collider3D.cuboid(x, y, z)      # FULL sizes
at.Collider3D.ball(radius)
at.Collider3D.capsule(half_height, radius)
at.Collider3D.mesh(model)          # a model's triangles (static scenery)
at.Collider3D.convex_hull(model)   # a model's convex hull (moving bodies)
# modifiers chain: .sensor(bool) .density(d) .friction(f) .restitution(r)
#                  .offset(x, y, z)   shape centre relative to the entity

at.Collider2D.cuboid(x, y) / .ball(radius) / .capsule(half_height, radius)
# same modifiers (.offset(x, y))

at.CharacterController(mode="normal", max_slope_climb_angle=0.7853982,
                       min_slope_slide_angle=0.5235988, autostep=None,
                       snap_to_ground=None, offset=0.05, slide=True)

at.CharacterVelocity(velocity=(0, 0, 0))   # .velocity get/set
```

## FrameContext (`ctx`)

Passed to every update callback. Reads are start-of-frame snapshots
(reading an entity you just wrote returns your pending write); writes apply
after the callback.

| Member | Contents |
|---|---|
| `ctx.time` | `FrameTime`: `delta` (s), `elapsed` (s), `frame` (1-based) |
| `ctx.input` | `Input` snapshot (below) |
| `ctx.scene` | `SceneView` (below) |
| `ctx.events` | `Events`: `send(name, payload=None)`, `read(name) -> list` |
| `ctx.resources` | `Resources`: `set/get/remove/clear/names`, `in`, `len` |
| `ctx.physics` | 3D physics queries (below) |
| `ctx.physics2d` | 2D physics queries |
| `ctx.audio` | `Audio`: play and control sounds (below) |
| `ctx.quit()` | ends the loop |

### SceneView / EntityView

```text
ctx.scene.entity_count
ctx.scene.names() -> list[str]
ctx.scene.find(name) -> EntityView | None
ctx.scene["name"]                    # KeyError when missing
for view in ctx.scene: ...
ctx.scene.spawn(entity)              # appears next frame
ctx.scene.despawn(id)
ctx.scene.load(scene)                # switch levels; the next frame sees only `scene`
```

```text
view.id                  # stable while alive
view.name
view.transform           # TransformRef; assign a Transform to replace
view.transform.position / .rotation_euler / .scale    # get/set
view.transform.set(transform); view.transform.copy()
view.character_velocity  # (vx, vy, vz) | None; assign to drive characters
view.camera_3d           # Camera3D copy | None; assign to move the camera
view.despawn()           # queues removal (children too)
```

### Input

```python
ctx.input.pressed(key) / just_pressed(key) / just_released(key)
ctx.input.mouse_pressed(btn) / mouse_just_pressed(btn) / mouse_just_released(btn)
ctx.input.cursor        # (x, y) | None
ctx.input.scroll        # (dx, dy)
ctx.input.modifiers     # .shift .ctrl .alt .super_key
ctx.input.gamepads      # Gamepads (below)
```

`Gamepads`: `ids()`, `get(id)`, `primary()`, `any_connected()`,
`pressed(id, button)`, `pressed_any(button)`, `axis(id, axis)`,
`axis_any(axis)`, `events`, `len`, iteration.
`Gamepad`: `id`, `name`, `pressed(button)`, `axis(axis)`,
`left_stick()`, `right_stick()`.
`GamepadEvent`: `kind` ("connected"/"disconnected"/"button"/"axis"),
`id`, `name`, `button`, `pressed`, `axis`, `value`.

### Physics (ctx.physics)

```text
ctx.physics.grounded(entity) -> bool
ctx.physics.velocity(entity) -> Vec3
ctx.physics.set_velocity(entity, (vx, vy, vz))
ctx.physics.apply_impulse(entity, (ix, iy, iz))
ctx.physics.apply_force(entity, (fx, fy, fz))
ctx.physics.gravity() -> Vec3
ctx.physics.set_gravity((gx, gy, gz))
ctx.physics.body_count() -> int
key = ctx.physics.cast_ray(origin, direction, max_distance=1000.0, exclude=None)
ctx.physics.ray_result(key) -> (entity_id, point, normal, distance) | None
ctx.physics.water_height(x, z) -> float | None   # surface height, waves included
```

`ctx.physics2d`: `apply_impulse(entity, (x, y))`, `grounded(entity)`,
`body_count()`. Entities are `EntityView`s or integer ids.

### Audio (ctx.audio)

Plays and controls sounds. Unlike scene/physics writes, audio operations apply
**immediately** and queries observe live state — including within the frame
that issued them. `play` returns an integer sound id used by the other
methods. Volume is a linear amplitude (`1.0` = full, `0.0` = silent).

```python
sid = ctx.audio.play("assets/jump.wav")          # -> sound id (int)
ctx.audio.play("assets/music.ogg", volume=0.6, loop_=True)
ctx.audio.play("assets/beep.wav", playback_rate=1.5, panning=-0.3)

ctx.audio.stop(sid, fade_out=0.2)                # fade out over 0.2 s, then stop
ctx.audio.stop_all(fade_out=0.0)                 # stop every sound
ctx.audio.pause(sid) / ctx.audio.resume(sid)
ctx.audio.seek_to(sid, 1.5)                      # seconds
ctx.audio.set_volume(sid, 0.8, fade=0.1)
ctx.audio.set_playback_rate(sid, 0.9)
ctx.audio.set_panning(sid, 0.5)                  # -1.0 left .. 1.0 right
ctx.audio.set_master_volume(0.7, fade=0.5)

ctx.audio.is_playing(sid) -> bool
ctx.audio.position(sid) -> float | None          # seconds; None if unknown
ctx.audio.sound_count() -> int
```

Supported formats: WAV, OGG/Vorbis, MP3, FLAC, PCM. A missing or undecodable
file raises `ValueError`; an unknown sound id raises `KeyError`.

**Backend:** the default build runs a *silent* backend — no audio device is
touched, no sound is produced, but playback positions and states still advance
deterministically, so headless runs and CI exercise identical code paths. Real
device output requires the `audio-cpal` build feature (`scripts/build.py`
enables it automatically when ALSA headers are present). If a device build
finds no output device, it falls back to silent rather than failing.

## Input mapping

```text
controls = at.ActionMap()
controls.bind(action, binding)        # stacks
controls.bind_axis(name, axis)        # stacks; values sum, clamp -1..1
controls.rebind(action, bindings)     # replace
controls.unbind(action)  /  controls.unbind_axis(name)
controls.bindings(action)  /  controls.actions()  /  controls.axis_names()
controls.pressed(action, ctx.input)
controls.just_pressed(action, ctx.input)
controls.axis(name, ctx.input) -> float
controls.axis_pair(x, y, ctx.input) -> (float, float)
action in controls  /  len(controls)
```

```text
at.Binding.key(key) / .mouse(button) / .gamepad_button(button)
at.Binding.gamepad_axis_positive(axis) / .gamepad_axis_negative(axis)
at.AxisBinding.key_pair(neg_key, pos_key)
at.AxisBinding.button_pair(neg_binding, pos_binding)
at.AxisBinding.gamepad_axis(axis, scale=1.0)
```

Unknown key/button/axis names raise `KeyError` at bind time.

## Enums

All are `str` enums — members work anywhere the plain string does.

| Enum | Members (sample) | Full set |
|---|---|---|
| `at.Key` | `A`–`Z`, `DIGIT_0`–`9`, `SPACE`, `ENTER`, `ESCAPE`, `TAB`, arrows, `F1`–`F12`, modifiers, punctuation | 80 |
| `at.MouseButton` | `LEFT`, `RIGHT`, `MIDDLE`, `BACK`, `FORWARD` | 5 |
| `at.GamepadButton` | `SOUTH`, `EAST`, `NORTH`, `WEST`, triggers, thumbs, d-pad, `START`, `SELECT`… | 19 |
| `at.GamepadAxis` | `LEFT_STICK_X/Y`, `RIGHT_STICK_X/Y`, `LEFT_Z`, `RIGHT_Z`, `DPAD_X/Y` | 8 |
| `at.GraphicsQuality` | `LOW` (no shadows), `MEDIUM` (1024), `HIGH` (2048), `ULTRA` (4096), `BEAUTIFUL` (soft cascaded shadows, PBR, bloom, refractive water) | 5 |

## Behaviour scripts (`automata.run_scene`)

```python
at.run_scene(path, *, max_frames=None, update=None, **game_options) -> int
```

Plays an Oberon scene file with its behaviour scripts:
`start(entity, ctx)`, `update(entity, ctx)`, `stop(entity, ctx)`. It
returns the number of frames run.

| `entity` | |
|---|---|
| `name`, `id`, `state` (dict), `grounded`, `script` | |
| `transform` | `position`, `rotation_euler` (radians), `scale`; `translate(d)`, `rotate(d)`. Read, or assign |
| `character_velocity` | Set to move a character |
| `attributes` | Dict-like, saved in the scene |
| `camera` | `ScriptCamera` or `None` |
| `audio` | `play()`, `stop()`, `playing`, `source` |
| `camera_3d` | The camera as a `Camera3D` (get/set) |
| `view` | The engine's `EntityView` for this frame |
| `despawn()` | At the end of the frame |

| `ctx` | |
|---|---|
| `time` | `delta`, `elapsed`, `frame` (from 1) |
| `scene` | Script entities: `["Name"]`, `find`, `by_id`, `in`, iteration |
| `input` | Keys with the aliases `"shift"`, `"ctrl"`, `"alt"` (either side) and `"left"`/`"right"`/`"up"`/`"down"`; `axis(neg, pos)`; mouse buttons, `mouse_delta`, `lock_mouse` |
| `physics` | `grounded`, `raycast`, `water_height(x, z)`; the rest of the engine's physics, which also accepts script entities |
| `audio`, `hud` | One-shot sounds; HUD images, rectangles and text (`hud.text(text, x, y, size=24, color, align, shadow, outline)`) |
| `camera` | The first `Camera3D` in the scene as a `ScriptCamera` |
| `events`, `resources`, `screen`, `raw`, `quit()` | Engine extras (`run_scene` only). `screen.measure_text(text, size)` gives text's pixel size |
| `log(...)`, `warn(...)` | |

Reads see every write made earlier in the frame; writes apply after all
scripts have run. That's the same model as Oberon's Play mode, from the same
module (`automata.script_api`).

**`ScriptCamera`**: `position`, `target`, `fov` (read/assign),
`look_at(point_or_entity)`, and
`follow(entity, offset=(0, 3, 8), smooth=8.0, look_height=0.6)`. Several
writes in one frame accumulate. The same API exists in Oberon's Play mode.
See [Oberon: Cameras](oberon/cameras.md).

