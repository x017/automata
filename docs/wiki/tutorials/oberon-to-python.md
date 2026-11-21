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


def test_camera_follows_the_player():
    seen = []

    def after(ctx):
        seen.append(ctx.scene["Main Camera"].camera_3d.target)

    at.run_scene(LEVEL, headless=True, max_frames=60, update=after)
    assert seen[-1][1] > 0.5, "the camera looks at the player, not the floor"
```

```console
pip install pytest
pytest -q
```

`update=` runs after your scripts every frame and receives the raw engine
context, so it's the place for assertions. The finished example project
has a test like this:
[`test_coin_run_example_plays_headless`](../../../crates/automata-python/tests/python/test_attributes_prefabs.py).

**Testing logic that needs input:** keep the logic in plain functions and
test those directly. Behaviour scripts are ordinary modules:

```python
from automata.scripting import load_script

rail = load_script("my_game/scripts/camera_rail.py")
assert rail.smoothstep(0.5) == 0.5
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
