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

The finished project from these tutorials is in
[`examples/oberon/coin_run/`](../../../examples/oberon/coin_run). Open
`level.ron` in Oberon to compare with your own.

## Python track

You write games as plain Python programs, with no editor.

| Tutorial | You'll learn |
|---|---|
| [Getting started](../getting-started.md) | Install, first window, first cube |
| [Zero to hero](../zero-to-hero.md) | A complete platformer: scenes, input, physics, cameras, events, testing |
| [Cookbook](../cookbook.md) | Short recipes: double jump, pause, spawn waves, screen shake |
| [Guide: Cameras](../guides/cameras.md) | Follow, orbit, rails, shake, zoom in code |
| [Guide: Rendering & Lights](../guides/rendering-and-lights.md) | Primitives, the sun, Environment, quality presets |

The two tracks meet: scenes made in Oberon load in Python
(`at.Scene.load`), and scenes built in Python open in Oberon
(`scene.save("level.ron")`).
