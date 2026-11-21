# Tutorial: From Oberon to Python

**Goal:** run your Oberon game outside the editor, test it automatically,
and package it to share.

**You'll learn:**
- `at.run_scene`
- headless runs
- writing pytest tests for gameplay
- the differences between the editor's Play mode and the engine

## 1. Install the engine package

From the repository root:

```console
python scripts/build.py         # builds the engine + wheel and installs `automata`
python -c "import automata; print(automata.__version__)"
```

## 2. Run the scene

Create `my_game/main.py` next to `level.ron`:

```python
import os
import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
at.run_scene(os.path.join(HERE, "level.ron"), title="Coin Run")
```

```console
python my_game/main.py
```

`run_scene` does the following:
1. loads the scene file
2. finds every entity with a script and resolves the path relative to
   the scene
3. imports the scripts
4. calls `start` and `update` every frame, exactly like Oberon's Play mode

The camera, physics, lighting, Environment, and attributes all come from
the file.

> **The built-in Player controller is an editor convenience.** In the
> engine, characters move only when a script sets
> `entity.character_velocity`. Attach
> [`player.py`](../../../examples/oberon/coin_run/scripts/player.py) to
> your player so it behaves the same everywhere.

## 3. What's different from the editor

| | Oberon Play | `at.run_scene` |
|---|---|---|
| Scripts | Separate Python process, hot reload | In-process, no reload |
| A script error | Pauses that script and logs it | Raises and stops the game, so you see it immediately |
| `ctx.input.axis`, key aliases | ✓ | ✓ |
| `entity.camera`, `ctx.camera` | ✓ | ✓ |
| `ctx.events`, `ctx.resources`, `ctx.quit()` | — | ✓ |
| Full physics (raycasts, impulses): `ctx.physics.*` | `grounded` only | ✓ |
| Spawning: `ctx.raw.scene.spawn(prefab)` | — | ✓ |
| `transform.translate` / `rotate` | ✓ | Assign `position` / `rotation_euler` instead |

Stick to the common column and a script runs identically in both.

## 4. Test it headlessly

`headless=True` runs the real engine without a window or GPU: the same
physics, the same scripts, as fast as possible. That makes gameplay
testable in CI.

`tests/test_level.py`:

```python
import math
import os
import automata as at

LEVEL = os.path.join(os.path.dirname(__file__), "..", "my_game", "level.ron")


def test_level_loads_and_runs():
    frames = at.run_scene(LEVEL, headless=True, max_frames=120)
    assert frames == 120


def test_player_stands_on_the_ground():
    heights = []

    def after(ctx):
        heights.append(ctx.scene["Player"].transform.position[1])

    at.run_scene(LEVEL, headless=True, max_frames=120, update=after)
    assert 0.5 < heights[-1] < 1.5, "the player fell through the floor"


def test_intro_starts_at_the_first_shot():
    seen = []

    def after(ctx):
        seen.append(ctx.scene["Main Camera"].camera_3d.position)

    at.run_scene(LEVEL, headless=True, max_frames=10, update=after)
    shot = at.Scene.load(LEVEL).find("Shot 1")
    position = next(c for c in shot.components() if isinstance(c, at.Transform)).position
    assert math.dist(seen[-1], position) < 0.5
```

```console
pip install pytest
pytest -q
```

`update=` runs after your scripts every frame and receives the raw engine
context, so it's the place for assertions.

> **Headless time is real time.** A headless run goes as fast as it can,
> and `ctx.time` follows the real clock, so 120 frames last a fraction of
> a second. Anything timed, like the five-second intro, won't have
> happened yet. Test timed logic directly instead, as below.

**Testing logic that needs time or input:** keep the logic in plain
functions and test those directly. Behaviour scripts are ordinary modules:

```python
from automata.scripting import load_script

rail = load_script("my_game/scripts/camera_rail.py")


def test_smoothstep_eases_in_and_out():
    assert rail.smoothstep(0.5) == 0.5
    assert rail.smoothstep(0.1) < 0.1


def test_the_rail_ends_after_the_last_shot():
    keys = [((0, 5, 10), (0, 0, 0)), ((10, 5, 0), (0, 0, 0))]
    assert rail.rail(keys, 0.0) == keys[0]
    assert rail.rail(keys, rail.SHOT_TIME) is None
```

To test `update` itself, pass small fake objects for `entity` and `ctx`
that have just the attributes the script uses, such as
`ctx.time.elapsed`, `ctx.input.just_pressed` and `entity.camera`. The
finished project does this to check that Space skips the intro, that Q / E
orbit, and that sprinting widens the view.

**Finished files:** the example project has the game launcher
[`main.py`](../../../examples/oberon/coin_run/main.py) (with a `--headless`
flag and a choice of `dusk` or `night`) and the full test suite
[`tests/test_level.py`](../../../examples/oberon/coin_run/tests/test_level.py),
which CI runs on every push:

```console
python examples/oberon/coin_run/main.py night
pytest examples/oberon/coin_run/tests
```

## 5. Share it

A game is a folder: `main.py`, `level.ron`, `scripts/`, and any prefabs.
To share it:

- **With Python users:** zip the folder. They run `pip install
  automata-engine` (the wheel from `scripts/build.py`) and then `python
  main.py`.
- **As one download:** use a Python bundler such as PyInstaller or
  Briefcase. Include the scene and scripts as data files. The engine is a
  normal compiled extension module, so bundlers pick it up automatically.

## What you learned

- `at.run_scene` plays an Oberon scene with its scripts in the real
  engine.
- Characters need a script outside the editor.
- `headless=True` with `max_frames` and `update=` turns gameplay into
  unit tests.

That's the whole loop: **build in Oberon → script in Python → test
headlessly → ship.**
