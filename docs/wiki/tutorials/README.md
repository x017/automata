# Tutorials

These are hands-on walkthroughs, and each one ends with something that
runs. Do them in order the first time; later tutorials assume the earlier
ones.

## Oberon track

You build in the editor and write small Python behaviour scripts.

| Tutorial | You'll learn | Time |
|---|---|---|
| [Your first scene](first-scene.md) | Creating, moving, and lighting objects; a camera; Play; saving | 10 min |
| [Coin collector](coin-collector.md) | Behaviour scripts, attributes, scoring, the Console, hot reload | 25 min |
| [Third-person camera](third-person-camera.md) | Follow, orbit, look-ahead, and sprint zoom with `ctx.camera` | 20 min |
| [Camera rail & intro shot](camera-rail.md) | Keyframed camera moves, easing, handing off to gameplay | 15 min |
| [Lighting & mood](lighting-and-mood.md) | The Graphics tab: sun, environment, tone mapping, fog, test scenes | 15 min |
| [From Oberon to Python](oberon-to-python.md) | `at.run_scene`, headless tests in CI, shipping a game | 15 min |
| [A first-person shooter](fps.md) | Kenney's CC0 models and sounds, import settings, mouse look, ray-cast shooting, a HUD, export | 40 min |

The finished project from these tutorials is in
[`examples/oberon/coin_run/`](../../../examples/oberon/coin_run). Open
`level.ron` in Oberon to compare with your own:

```console
cargo run -p oberon --release examples/oberon/coin_run/level.ron
```

| Tutorial | Finished result in `coin_run/` |
|---|---|
| Your first scene | `level.ron`: ground, ledge, player, sun, environment, camera |
| Coin collector | `scripts/coin.py`, `scripts/rules.py`, `scripts/player.py`; six coins, one worth 5 |
| Third-person camera | `scripts/follow_camera.py`: follow, Q / E orbit, look-ahead, sprint zoom |
| Camera rail & intro shot | `Shot 1`–`Shot 3` markers and `scripts/camera_rail.py`, attached to Main Camera |
| Lighting & mood | `level.ron` (afternoon), `level_dusk.ron`, `level_night.ron` |
| From Oberon to Python | `main.py` and `tests/test_level.py` |

The shooter is a separate project,
[`examples/oberon/fps/`](../../../examples/oberon/fps) (*Blaster Arena*).

`build_level.py` regenerates the three scene files from code. CI plays the
project in both Oberon's script runner and the engine, and checks that the
scripts on these pages match the files.

## Python track

You write games as plain Python programs, with no editor.

| Tutorial | You'll learn |
|---|---|
| [Getting started](../getting-started.md) | Install, first window, first cube |
| [Zero to hero](../zero-to-hero.md) | A complete platformer: scenes, input, physics, cameras, events, testing |
| [Cookbook](../cookbook.md) | Short recipes: double jump, pause, spawn waves, screen shake |
| [Guide: Cameras](../guides/cameras.md) | Follow, orbit, rails, shake, zoom in code |
| [Guide: Rendering & Lights](../guides/rendering-and-lights.md) | Primitives, the sun, Environment, quality presets |

Every Zero to Hero module and exercise solution is a runnable file in
[`examples/python/zero_to_hero/`](../../../examples/python/zero_to_hero).
Every Cookbook recipe works together in
[`examples/python/cookbook_platformer.py`](../../../examples/python/cookbook_platformer.py).
CI runs them all headless:

```console
pytest examples/python/zero_to_hero examples/python/tests
```

The two tracks meet: scenes made in Oberon load in Python
(`at.Scene.load`), and scenes built in Python open in Oberon
(`scene.save("level.ron")`).
