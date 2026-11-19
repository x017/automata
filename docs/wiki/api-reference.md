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
)
```

| Member | Type | Notes |
|---|---|---|
| `run(update=None, max_frames=None) -> int` | method | drives the loop; returns frames run; a Game runs once |
| `load_scene(scene)` | method | replaces the world; callable before `run` or inside `update` |
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
at.MeshRenderer.quad(size=1.0, color=(1, 1, 1, 1))
m.kind                      # "cube" or "quad"
m.color                     # (r, g, b, a)
```

Plain data; materialized on the GPU by the windowed driver, inert headless.
`Transform.scale` sizes the primitive.

### DirectionalLight

```python
at.DirectionalLight(direction=(-0.5, -1.0, -0.35),
                    color=(1.0, 1.0, 1.0), ambient=0.3, shadows=True)
```

The scene's sun; the first entity carrying one lights the frame. Getters:
`direction`, `color`, `ambient`, `shadows`. Without one, a neutral
shadowless default is used.

### Physics components

```text
at.RigidBody(body_type="dynamic", *, gravity_scale=1.0,
             linear_damping=0.0, angular_damping=0.0, enabled=True)
at.RigidBody.dynamic() / .fixed() / .kinematic_velocity() / .kinematic_position()
body.body_type

at.Collider3D.cuboid(x, y, z)      # FULL sizes
at.Collider3D.ball(radius)
at.Collider3D.capsule(half_height, radius)
# modifiers chain: .sensor(bool) .density(d) .friction(f) .restitution(r)

at.Collider2D.cuboid(x, y) / .ball(radius) / .capsule(half_height, radius)
# same modifiers

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
```

`ctx.physics2d`: `apply_impulse(entity, (x, y))`, `grounded(entity)`,
`body_count()`. Entities are `EntityView`s or integer ids.

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
| `at.GraphicsQuality` | `LOW` (no shadows), `MEDIUM` (1024), `HIGH` (2048), `ULTRA` (4096) | 4 |
