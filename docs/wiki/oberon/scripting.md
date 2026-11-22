# Scripting in Oberon

You write gameplay as **behaviour scripts**. A behaviour script is a Python
file with up to three functions, and you attach it to an entity. Oberon
calls those functions while the scene plays.

```python
# scripts/spinner.py
SPEED = 1.5  # radians per second


def start(entity, ctx):
    entity.state["turns"] = 0
    print(f"{entity.name} is spinning")


def update(entity, ctx):
    x, y, z = entity.transform.rotation_euler
    entity.transform.rotation_euler = (x, y + SPEED * ctx.time.delta, z)


def stop(entity, ctx):
    print("stopped")
```

All three functions are optional.
- `start(entity, ctx)` runs once, on the entity's first Play frame.
- `update(entity, ctx)` runs every frame.
- `stop(entity, ctx)` runs when you press Stop.

## Attach a script

- **Inspector → Add Component → Script…**, or right-click an entity →
  **Attach Script…**.
- Pick an existing `.py` file or type a new name. A new file starts from a
  template: a small character controller with a cheat-sheet docstring.
- The script path is saved **relative to the scene file**. You can move the
  project folder and the link still works.

Several entities can share one script. Each entity gets its own `entity`
handle and its own `entity.state`.

## The frame model

Every frame, Oberon sends the scene state to the Python runner. The runner
calls every script, and the scripts' changes come back and apply before the
frame renders. Two rules follow from this:

1. **Reads see the start of the frame**, plus every write made earlier in
   the same frame, by any script. All scripts share one handle per entity.
2. **Writes reach the simulation after all scripts have run.** Physics,
   rendering and sound see the frame's final values. If two scripts set the
   same value, the last one wins.

So when four drones each run
`ctx.scene["Player"].attributes["health"] -= 5` in one frame, the player
loses 20 health, not 5. A despawn also lands at the end of the frame:
other scripts can still find the entity until then.

`run_scene` follows the same rules, because both runners use the same code
for `entity` and `ctx`.

## `entity`

| Member | Meaning |
|---|---|
| `entity.name`, `entity.id` | Name from the hierarchy; stable numeric id |
| `entity.transform.position` | `(x, y, z)`. Read it, or assign a tuple |
| `entity.transform.rotation_euler` | `(x, y, z)` in **radians** (the Inspector shows degrees) |
| `entity.transform.scale` | `(x, y, z)` |
| `entity.transform.translate(d)` / `.rotate(d)` | Add a delta to the position / rotation |
| `entity.character_velocity` | Set it to move a Character Controller |
| `entity.grounded` | True when a character is standing on something |
| `entity.attributes["key"]` | Custom values from the Inspector; read and write like a dict |
| `entity.camera` | The entity's camera, or `None` ([Cameras](cameras.md)) |
| `entity.audio.play()` / `.stop()` | Start (or restart) / stop the entity's Audio Source ([Audio](audio.md)) |
| `entity.audio.playing` | Whether the Audio Source was playing at the start of the frame |
| `entity.despawn()` | Remove the entity until Play stops |
| `entity.state` | A dict of your own that persists between frames (not saved) |

## `ctx`

| Member | Meaning |
|---|---|
| `ctx.time.delta` | Seconds since the last frame (multiply speeds by it) |
| `ctx.time.elapsed`, `ctx.time.frame` | Seconds since Play started; frame number, starting at 1 |
| `ctx.input.pressed("w")` | Key held down |
| `ctx.input.just_pressed("space")` | Pressed this frame |
| `ctx.input.just_released("e")` | Released this frame |
| `ctx.input.axis("a", "d")` | −1, 0, or 1 from two keys |
| `ctx.input.mouse_delta` | `(dx, dy)` mouse movement in pixels since last frame; +y is down |
| `ctx.input.mouse_pressed("left")` | Mouse button held (`"left"`, `"right"`, `"middle"`); also `mouse_just_pressed` and `mouse_just_released` |
| `ctx.input.lock_mouse(True)` | Hide and capture the cursor for mouse look. `False` frees it, and Stop always frees it |
| `ctx.input.mouse_locked` | Whether the cursor is captured |
| `ctx.physics.grounded(entity)` | Same as `entity.grounded` |
| `ctx.physics.raycast(origin, direction, max_distance=100, ignore=None)` | The first collider along a ray, or `None`. Returns a `Hit` with `.entity`, `.point`, `.normal` and `.distance`. `ignore` skips one entity, usually the caster. Sensors are skipped |
| `ctx.physics.water_height(x, z)` | Height of the water surface at a world point, waves included, or `None` where there's no water |
| `ctx.water.height(x, z)` | The same |
| `ctx.water.splash(position, strength=1.0)` | A splash at a point or an entity: rings spread out and foam gathers. `strength` runs from 0 to 10; 1 is about a crate dropped from a couple of metres. Things that fall in or move through water make their own ripples |
| `ctx.audio.play("sounds/hit.ogg", volume=1, pitch=1, position=None)` | Play a sound once. With a `position` (a point or an entity) it fades with distance |
| `ctx.hud.image("sprites/crosshair.png", x, y, size=None, color=(1,1,1,1))` | Draw an image this frame, centred on `(x, y)` in screen pixels |
| `ctx.hud.rect(x, y, width, height, color)` | Draw a filled rectangle this frame, centred on `(x, y)` |
| `ctx.hud.text("100%", x, y, size=24, color=(1,1,1,1), align="left", shadow=None, outline=None)` | Draw text this frame with the built-in font (see below) |
| `ctx.hud.size`, `ctx.hud.center` | The game view's size in pixels, and its centre |
| `ctx.scene["Name"]` | Another entity by name (raises `KeyError` if missing) |
| `ctx.scene.find("Name")` | The same, but returns `None` if missing |
| `ctx.scene.by_id(id)` | An entity by id (as in `hit.entity.id`), or `None` |
| `"Name" in ctx.scene` | Whether an entity has that name |
| `for e in ctx.scene:` | Every entity |
| `ctx.camera` | The camera Play renders through, or `None` |
| `ctx.log(...)`, `ctx.warn(...)` | Write to the Console. `print()` works too |

HUD shapes last one frame, so draw them in `update` every frame you want
them shown. Image paths are relative to the scene folder. Each call draws on
top of the previous one.

**Text** uses the built-in font (Geist Medium):
- `size` is the line height in pixels.
- `align` says where `x` sits on each line: `"left"` (its left edge),
  `"center"` or `"right"`.
- `y` is the vertical centre, and `\n` starts a new line.
- `shadow` and `outline` take a colour, or `True` for a translucent black
  shadow or a solid black outline.

```python
ctx.hud.text(f"{health}%", 290, ctx.hud.size[1] - 48, size=26, shadow=True)
ctx.hud.text("ARENA CLEAR", *ctx.hud.center, size=56, align="center", outline=True)
```

Text looks the same in Play and in the exported game, because both draw it
with the engine's own glyph atlas. Under `run_scene`,
`ctx.screen.measure_text(text, size)` returns the `(width, height)` text
will cover.

**Key names** are lowercase: `"a"`–`"z"`, `"0"`–`"9"`, `"space"`,
`"enter"`, `"escape"`, `"tab"`, `"backspace"`, the arrows `"arrow_left"`,
`"arrow_right"`, `"arrow_up"` and `"arrow_down"` (or just `"left"`,
`"right"`, `"up"`, `"down"`), and `"shift"` / `"ctrl"` / `"alt"`. The last
three match either side of the keyboard; use `"shift_left"` or
`"shift_right"` (and likewise for ctrl and alt) to require one side.

## Hot reload

Save a script (Ctrl+S in the code editor, or from any external editor)
while the scene is playing. Oberon reloads it within half a second, and the
Console shows `Reloaded spinner.py`. `entity.state` survives the reload,
and `start` is **not** called again. To rerun `start`, press Stop and then
Play.

## Errors

When a script raises an exception, the Console shows the traceback with a
clickable **file:line** link that jumps to the code editor. That script
then **pauses** so you don't get the same error 60 times a second. The rest
of the scene keeps running. Fix the file and save, and the script resumes.

A syntax error found when the script loads works the same way.

## Testing scripts headless

`run_scene(..., headless=True)` runs frames as fast as it can, so
`ctx.time.delta` is close to zero and timers barely move. To test gameplay
that depends on time (cooldowns, falling, damage over time), pace the run:

```python
frames = at.run_scene("level.ron", headless=True, max_frames=90, target_fps=60)
```

The [FPS tutorial](../tutorials/fps.md) and
`examples/oberon/fps/tests/test_fps.py` show this in practice.

## Which Python?

Oberon runs scripts with `python3` from your `PATH` by default. To pick a
specific interpreter, such as a virtualenv, open **Settings → Python →
Interpreter**. The runner needs only the standard library. Any packages
you install in that environment can be imported from your scripts.

## Same scripts, real engine

The scene file plus its scripts run unchanged outside Oberon:

```python
import automata as at

at.run_scene("level.ron")                          # windowed
at.run_scene("level.ron", headless=True, max_frames=600)   # CI test
```

Everything on this page works in both places: one module
(`automata/script_api.py`) implements `entity` and `ctx` for Oberon and for
`run_scene`. A shared test suite (`tests/script_conformance/`) runs the same
scenes through both runners in CI.

Under `run_scene`, `ctx` also exposes the full engine:
- `ctx.events`
- `ctx.resources`
- the complete `ctx.physics` (impulses, queued casts, and more)
- `ctx.quit()`
- `ctx.raw`, the engine's own frame context

In Oberon these raise an error that names the member and says it needs
`run_scene`. See [From Oberon to Python](../tutorials/oberon-to-python.md).
