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

1. **Reads see the start of the frame**, plus your own writes made earlier
   in the same frame.
2. **Writes apply after all scripts have run.** If two scripts write the
   same value, the last one wins.

## `entity`

| Member | Meaning |
|---|---|
| `entity.name`, `entity.id` | Name from the hierarchy; stable numeric id |
| `entity.transform.position` | `(x, y, z)`. Read it, or assign a tuple |
| `entity.transform.rotation_euler` | `(x, y, z)` in **radians** (the Inspector shows degrees) |
| `entity.transform.scale` | `(x, y, z)` |
| `entity.transform.translate(d)` / `.rotate(d)` | Add a delta (Oberon only) |
| `entity.character_velocity` | Set it to move a Character Controller |
| `entity.grounded` | True when a character is standing on something |
| `entity.attributes["key"]` | Custom values from the Inspector; read and write like a dict |
| `entity.camera` | The entity's camera, or `None` ([Cameras](cameras.md)) |
| `entity.state` | A dict of your own that persists between frames (not saved) |

## `ctx`

| Member | Meaning |
|---|---|
| `ctx.time.delta` | Seconds since the last frame (multiply speeds by it) |
| `ctx.time.elapsed`, `ctx.time.frame` | Seconds since Play started; frame number, starting at 1 |
| `ctx.input.pressed("w")` | Key held down |
| `ctx.input.just_pressed("space")` | Pressed this frame |
| `ctx.input.just_released("e")` | Released this frame |
| `ctx.input.axis("a", "d")` | −1, 0, or 1 from two keys (Oberon only) |
| `ctx.physics.grounded(entity)` | Same as `entity.grounded` |
| `ctx.scene["Name"]` | Another entity by name (raises `KeyError` if missing) |
| `ctx.scene.find("Name")` | The same, but returns `None` if missing |
| `for e in ctx.scene:` | Every entity |
| `ctx.camera` | The camera Play renders through, or `None` |
| `ctx.log(...)`, `ctx.warn(...)` | Write to the Console. `print()` works too |

**Key names** are lowercase: `"a"`–`"z"`, `"0"`–`"9"`, `"space"`,
`"enter"`, `"escape"`, `"tab"`, `"backspace"`, `"arrow_left"` (or
`"left"`) and the other arrows, and `"shift"` / `"ctrl"` / `"alt"`. The
last three mean the left-hand key; use `"shift_right"` for the right one.

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

Under `run_scene`, `ctx` also exposes the full engine:
- `ctx.events`
- `ctx.resources`
- the complete `ctx.physics` (raycasts, impulses)
- `ctx.quit()`

To keep a script portable, use only the members above that aren't marked
"Oberon only". See [From Oberon to Python](../tutorials/oberon-to-python.md).
