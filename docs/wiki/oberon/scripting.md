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

## Types and completion

Add type hints, and your editor knows the whole API. Typing `ctx.` lists
everything with its documentation, and a misspelt `entity.animaton` is
underlined before you press Play:

```python
from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # types for your editor; nothing is imported when it runs
    from automata.script_api import Context, Entity


def update(entity: Entity, ctx: Context) -> None:
    entity.character_velocity = (ctx.input.axis("a", "d") * 5.0, 0.0, 0.0)
```

New scripts from Oberon, and projects from
[`oberon new`](command-line.md), start like this. The types come from
`automata.script_api`, which ships with the engine. Projects made by
`oberon new` have it in `.venv`, together with the basedpyright language
server, so VS Code, PyCharm and Oberon's [code editor](code-editor.md)
complete and check scripts with no setup.

Members that can be missing are typed `Optional`. For example,
`entity.animation` is `None` without an Animator:

```python
if entity.animation is not None:
    entity.animation.play("Run")
```

`pyrightconfig.json` in new projects reports unguarded uses as warnings.

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
| `entity.animation.play("Running", loop=True, fade=0.2)` | Blend to a clip of the entity's animated model over `fade` seconds. An unknown clip raises `KeyError`. Asking for the clip that's already playing does nothing, so it's safe every frame ([Animation](models.md#animation)) |
| `entity.animation.stop(fade=0)` | Blend back to the rest pose |
| `entity.animation.clip` / `.time` / `.playing` / `.clips` | The current clip, seconds into it (assign to jump), whether it's playing (False once a one-shot ends), and every clip name. `.speed` and `.root_motion` can be set too |
| `entity.joint` | The entity's Joint while it runs, or `None`: `.angle`, `.value`, `.force`, `.attached`, `.broken`; set `.motor_target`, `.motor_speed`, `.limits`, `.enabled`, `.connected`. `on_joint_broken(entity, ctx)` runs when it snaps ([Physics → Joints](../guides/physics.md#joints)) |
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
| `ctx.input.pressed("jump")` | All of the above also take an action bound with `ctx.settings.bind` ([Settings](#player-settings)) |
| `ctx.input.any_just_pressed()` | The key that went down this frame (`"k"`, `"mouse_left"`), or `None`. Use it for "press a key" screens |
| `ctx.input.mouse_delta` | `(dx, dy)` mouse movement in pixels since last frame; +y is down |
| `ctx.input.mouse_pressed("left")` | Mouse button held (`"left"`, `"right"`, `"middle"`); also `mouse_just_pressed` and `mouse_just_released` |
| `ctx.input.lock_mouse(True)` | Hide and capture the cursor for mouse look. `False` frees it, and Stop always frees it |
| `ctx.input.mouse_locked` | Whether the cursor is captured |
| `ctx.physics.grounded(entity)` | Same as `entity.grounded` |
| `ctx.physics.raycast(origin, direction, max_distance=100, ignore=None)` | The first collider along a ray, or `None`. Returns a `Hit` with `.entity`, `.point`, `.normal` and `.distance`. `ignore` skips one entity, usually the caster. Sensors are skipped |
| `ctx.physics.water_height(x, z)` | Height of the water surface at a world point, waves included, or `None` where there's no water |
| `ctx.water.height(x, z)` | The same |
| `ctx.water.splash(position, strength=1.0)` | A splash at a point or an entity: rings spread out and foam gathers. `strength` runs from 0 to 10; 1 is about a crate dropped from a couple of metres. Things that fall in or move through water make their own ripples, and hard landings splash |
| `ctx.particles.burst("sparks", position, direction=None, count=None, **fields)` | A one-off particle effect at a point or an entity that cleans up after itself: `"fire"`, `"smoke"`, `"sparks"`, `"dust"`, `"splash"`, `"magic"` or `"motes"`, with any field changed (`color=(0.3, 1, 0.4, 1)`). `direction` aims it, for example along a hit's `normal`. `count` is how many particles. See [Particles](../guides/rendering-and-lights.md#particles) |
| `ctx.particles.emit(entity, count)` | Extra particles from the entity's own Particle Emitter |
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
| `ctx.get(key)`, `ctx.set(key, value)`, `ctx.scan(pattern)`, `ctx.keys(pattern)`, `ctx.ref(key)` | Other entities' state by key, like `"Player:attr:health"` ([Keys](#keys)) |
| `ctx.subscribe(pattern, fn)`, `ctx.unsubscribe(...)`, `ctx.publish(channel, data)` | Channels: animation events and your own messages ([Channels](#channels)) |
| `ctx.tween(entity, "position", to, seconds, ease="out_cubic")`, `ctx.tween_value(start, end, seconds)` | Move a value smoothly over time ([Tweens and timers](#tweens-and-timers)) |
| `ctx.after(seconds, fn, *args)`, `ctx.every(seconds, fn, *args, times=None)` | Call a function later, or again and again ([Tweens and timers](#tweens-and-timers)) |
| `ctx.ease("out_back", t)` | `t` (0 to 1) through an easing curve |
| `ctx.save["coins"] = 12` | Values that last between runs, in save slots ([Save games](#save-games)) |
| `ctx.settings.volume`, `.graphics`, `.bind("jump", "space")` | The player's settings, remembered automatically ([Settings](#player-settings)) |
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

## Keys

Scripts can read and write any entity's state with a key, Redis style.
A key is `entity:component:field`:

```python
ctx.get("Player:animation:clip")            # "Run"
ctx.set("Player:animation:speed", 1.5)
ctx.get("Player:attr:health")                # its attributes
ctx.set("Door:transform:position", (0, 3, 0))

for anim in ctx.scan("Enemy_*:animation"):   # every enemy that has one
    anim.play("Dance", fade=0.3)
ctx.scan("Enemy_*:attr:health")              # [30, 12, 50]
ctx.keys("Player:attr:*")                    # ["Player:attr:health", ...]
```

| Component | Fields |
|---|---|
| `animation` | `clip` (setting it plays that clip), `time`, `speed`, `root_motion`; read-only `playing`, `looping`, `duration`, `clips`, `springs` |
| `transform` | `position`, `rotation` (radians, same as `rotation_euler`), `scale` |
| `joint` | `motor_speed`, `motor_target`, `limits`, `enabled`, `connected`, `break_force`; read-only `angle`, `value`, `force`, `attached`, `broken`, `kind` |
| `attr` | The entity's [attributes](prefabs-and-attributes.md), by name |

A shorter key gives more: `"Player:animation"` is the same handle as
`entity.animation`, and `"Player"` is the entity. Reads and writes follow
the [frame model](#the-frame-model), like everything else.

- **`get`, `set` and `ref` take one exact key.** If two entities share the
  name, `get` raises an error instead of guessing. `scan` returns every match,
  in hierarchy order. It skips entities that lack the component or field,
  and accepts `*` and `?` wildcards.
- **A wrong key raises `KeyError` and suggests the nearest right one**:
  `'Player:animaton:clip': there's no component 'animaton'; did you mean
  'animation'?`. Read-only fields raise `AttributeError`. `set` won't create
  attributes, so a typo can't make a new one; add them with
  `entity.attributes["mana"] = 10`.
- **`ctx.ref(key)`** makes a handle to keep, usually from `start`. It looks
  the key up again each time it's used, so it always shows this frame's
  values:

  ```python
  def start(entity, ctx):
      entity.state["boss"] = ctx.ref("Boss:animation")
      entity.state["hp"] = ctx.ref("Boss:attr:health")

  def update(entity, ctx):
      if entity.state["hp"].value < 20:
          entity.state["boss"].play("Enraged", fade=0.2)
  ```

- **Names:** keys find entities by name, so entity names can't contain `:`.
  Oberon refuses the rename, and `at.Entity("a:b")` raises `ValueError`.
  When several entities share a name, the Inspector says so under the name.
  `entity.animation.play(...)` is still the everyday way to drive your own
  entity. Keys are for reaching the others.

## Channels

`ctx.subscribe(pattern, fn)` calls `fn(msg)` for every message published
on a matching channel. A pattern is an exact name or uses `*` and `?`.
Animation events arrive on their own, as `<entity>:animation:<event>`
(see [Animation events](models.md#animation-events)), and scripts can
publish their own messages:

```python
def opened(msg):
    msg.ctx.log("door opened by", msg.data)

def start(entity, ctx):
    ctx.subscribe("door:*", opened)
    ctx.subscribe("*:animation:footstep", lambda msg: msg.ctx.audio.play("step.ogg", position=msg.entity))

# elsewhere
ctx.publish("door:open", "Player")
```

`msg` has `channel`, `data` and `ctx`. Animation events also set `entity`,
`event` and `clip`. `publish` delivers at once and returns how many
subscribers got the message.

Subscribing the same function to the same pattern again does nothing, so
it's safe in `update`. A subscription ends when the entity whose script
made it goes away. You can also end one with `ctx.unsubscribe(pattern)` or
by calling `cancel()` on what `subscribe` returned. If a subscriber raises,
its script pauses like any other script error.

## Tweens and timers

`ctx.tween` moves one of an entity's values to a target over some seconds,
along an easing curve. `ctx.after` and `ctx.every` call a function later.
Each returns a handle with `cancel()`:

```python
def start(entity, ctx):
    door = ctx.scene["Door"]
    ctx.tween(door, "position", (0, 3, 0), 1.5, ease="in_out_sine")
    ctx.after(2.0, ctx.audio.play, "sounds/clunk.ogg")
    entity.state["spawner"] = ctx.every(5.0, spawn_wave, ctx)

def collected(coin, ctx):
    ctx.tween(coin, "scale", (0, 0, 0), 0.3, ease="in_back", on_done=coin.despawn)
```

**What a tween can move.** `"position"`, `"rotation"` (Euler angles in
radians) and `"scale"`, or an attribute by name (`"score"`). Any writable
[key](#keys) field works as `component:field`, such as
`"animation:speed"` or `"attr:glow"`. Numbers and tuples of numbers
tween. If both ends are whole numbers, the value stays whole, so a score
counts up 1, 2, 3.

| `ctx.tween(...)` argument | Meaning |
|---|---|
| `entity` | The entity, or its name |
| `field`, `to`, `duration` | What moves, where to, and how many seconds it takes |
| `ease` | A curve name (below) or your own function of `t`. Default `"out_cubic"`: fast, then settling |
| `start=` | Where it starts. Otherwise it starts from the value it has when it begins |
| `delay=` | Seconds to wait before it starts |
| `loop=` | `True` repeats it forever. A number plays it that many times |
| `yoyo=` | Every other play runs backwards. `yoyo=True, loop=True` swings back and forth |
| `on_update=`, `on_done=` | `on_update(value)` runs each frame it moves, and `on_done()` runs when it ends |

A `Tween` has `value`, `progress` (0 to 1) and `done`, plus `cancel()`
(stop where it is), `finish()` (jump to the end and call `on_done`),
`pause()` and `resume()`. Starting a tween on the same entity and field
replaces the old one, so a door told to close halfway through opening
turns round smoothly.

`ctx.tween_value(start, end, seconds, on_update=None, ease=...)` tweens a
value of your own. It's handy for fades, because HUD drawing happens every
frame:

```python
def start(entity, ctx):
    entity.state["fade"] = ctx.tween_value(1.0, 0.0, 1.2, ease="in_quad", delay=1.0)

def update(entity, ctx):
    alpha = entity.state["fade"].value
    ctx.hud.text("LEVEL 1", *ctx.hud.center, size=64, align="center", color=(1, 1, 1, alpha))
```

**Easing curves** (31 of them, the standard set that
[easings.net](https://easings.net) draws): `"linear"`, plus `in_`, `out_`
and `in_out_` versions of `sine`, `quad`, `cubic`, `quart`, `quint`,
`expo`, `circ`, `back`, `elastic` and `bounce`. `in_` starts slow, `out_`
ends slow, and `in_out_` does both. `back` overshoots a little and
`elastic` wobbles past the end. `bounce` bounces at the end. Spellings like
`"easeOutCubic"` work too, and a misspelt name gets a suggestion.

**Timers.** `ctx.after(seconds, fn, *args)` calls `fn(*args)` once.
`ctx.every(seconds, fn, *args, times=None)` calls it every `seconds`,
forever or `times` times. A `Timer` has `remaining`, `count` and `done`,
and `cancel()`, `pause()` and `resume()`. Make an `every` timer in `start`
or once on an event, not in `update`, or you'll start a new one each
frame. To repeat at random intervals, let the function book itself again,
like `examples/oberon/campfire/scripts/crackle.py`:

```python
def pop(entity, ctx):
    ctx.particles.emit(entity, random.randint(8, 20))
    ctx.after(random.uniform(0.8, 3.0), pop, entity, ctx)
```

**When they run.** Timers and tweens move once a frame by
`ctx.time.delta`. They run in the order they were made, after animation
events and before any `update`, so `update` already sees this frame's
tweened values. Something made during a frame starts counting the next
frame. After a long hitch, an `every` timer catches up, but at most 8
calls in one frame. Timers and tweens stop when the entity whose script
made them goes away, and a tween also stops when the entity it moves goes
away. They pause while their script is paused by an error, and Stop ends
them all. Functions they call follow hot reloads like subscribers do. If
one raises, its script pauses like any other script error.

## Save games

`ctx.save` is a dict that outlives the run. Put anything in it that a
player expects to find next time:

```python
def start(entity, ctx):
    if "checkpoint" in ctx.save:
        entity.transform.position = ctx.save["checkpoint"]

def update(entity, ctx):
    ...
    ctx.save["coins"] = ctx.save.get("coins", 0) + 1
    ctx.save["inventory"].append("key")   # changes inside lists count too
```

- **Values** are JSON: numbers, strings, `True`/`False`/`None`, and lists
  and dicts of those. Tuples come back as lists. Anything else, such as an
  entity or a `NaN`, raises an error when you assign it, so nothing bad
  reaches the file.
- **Writing** happens at the end of the frame, and only if something
  changed. Use `ctx.save.flush()` to write now. Writes are atomic (a temp
  file, then a rename), so a crash can't leave half a save. A file that
  won't parse is renamed to `….broken`, a warning goes to the Console, and
  the slot starts empty.
- **Slots.** Everything goes to the slot named `"default"` until you switch:

  | | |
  |---|---|
  | `ctx.save.use(2)` / `use("hard mode")` | Switch slots (the old one is written first). A new slot starts empty |
  | `ctx.save.slot` | The current slot's name |
  | `ctx.save.slots()` | Saved slots: each has `name`, `saved_at` (`time.time()` seconds) and `keys` (how many values). Use them for a load screen |
  | `ctx.save.exists(slot=None)`, `ctx.save.saved_at` | Whether a slot was saved; when the current one was |
  | `ctx.save.delete(slot=None)`, `ctx.save.clear()` | Delete a slot's file; empty the current slot |
  | `get`, `setdefault`, `pop`, `update`, `keys`, `items`, `in`, `len`, `del` | The usual dict methods |

- **Where the files go.** Each slot is `saves/<slot>.json` in the save
  folder:

  | Running in | Save folder |
  |---|---|
  | Oberon's Play | `<project>/.oberon/save` (delete it to start fresh) |
  | `run_scene(...)` | `.oberon/save` next to the scene. Headless runs keep nothing unless you pass `save_dir=` |
  | An exported game | A folder named after the game (`game.json`'s `name`) in the player's user-data folder: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS, `$XDG_DATA_HOME` or `~/.local/share` on Linux |

  Setting the `AUTOMATA_SAVE_DIR` environment variable overrides all of
  them. `.oberon/` is never exported, so your test saves don't ship.

## Player settings

`ctx.settings` holds the player's settings. It's saved in the same folder
as `settings.json`, shared by every save slot, and applied as soon as the
game starts:

| | |
|---|---|
| `ctx.settings.volume = 0.5` | Master volume, 0 to 1 (1 by default) |
| `ctx.settings.graphics = "low"` | Graphics preset: `"low"`, `"medium"`, `"high"`, `"ultra"` or `"beautiful"`. Until the player picks one, it reads as the preset the game is running at |
| `ctx.settings["sensitivity"] = 0.4` | Any other setting, with the same JSON rules as `ctx.save` |
| `ctx.settings.reset()` | Everything back to the game's defaults |

In Oberon's Play mode, the graphics setting overrides the editor's preset
until you press Stop.

**Key bindings.** Name your controls as *actions* and give them default
keys in `start`. Then ask `ctx.input` about the action instead of a key:

```python
def start(entity, ctx):
    ctx.settings.bind("jump", "space")
    ctx.settings.bind("fire", "mouse_left", "f")

def update(entity, ctx):
    if ctx.input.just_pressed("jump"):
        ...
```

A settings screen changes them with `rebind`. The change is saved, and it
wins over `bind` from then on:

```python
def update(entity, ctx):
    if entity.state.get("waiting"):
        key = ctx.input.any_just_pressed()
        if key and key != "escape":
            ctx.settings.rebind("jump", key)
            entity.state["waiting"] = False
        return
    if ctx.input.just_pressed("f1"):
        entity.state["waiting"] = True
    ctx.hud.text(f"Jump: {', '.join(ctx.settings.keys('jump'))}", 16, 16)
```

| | |
|---|---|
| `bind(action, *keys)` | Default keys: [key names](#ctx) or `"mouse_left"`/`"mouse_right"`/`"mouse_middle"`. Binding again replaces the defaults |
| `rebind(action, *keys)` | The player's keys, saved. With no keys, the action has none |
| `keys(action)`, `bindings()` | One action's keys now, or every action's |
| `reset_keys(action=None)` | Back to the defaults (one action or all) |

A misspelt key (`"spce"`) raises an error that suggests the right one. An
action name that was never bound is treated as a key name, so
`ctx.input.pressed("space")` keeps working.

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

Oberon runs scripts with the project's own `.venv`, the same interpreter
that `oberon run`, `oberon test` and the language server use (see
[The project's Python](command-line.md#the-projects-python)). A folder
without `.venv` uses `python3` from your `PATH`. **Settings → Python →
Interpreter** overrides both. The runner needs only the standard library.
Packages you install with `oberon pip install …` can be imported from your
scripts.

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
