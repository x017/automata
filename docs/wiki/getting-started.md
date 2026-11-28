# Getting Started

This page takes you from a fresh checkout to a window on screen, and ends
with the two problems everyone hits once.

## Requirements

| Requirement | Why |
|---|---|
| Python 3.11+ | The gameplay language |
| A Rust toolchain | The engine core is Rust |
| A display/GPU (for windowed mode) | Headless mode needs neither |
| `patchelf` (Linux only) | Wheel repair during builds |

## Install

From a checkout of the repository:

```console
python scripts/build.py
```

That one script compiles the Rust workspace, builds the Python wheel with
maturin, and installs it into your active virtualenv. For a full
build-plus-verify pass:

```console
python scripts/build.py --all   # also runs fmt, clippy, tests, docs
```

## Your first window (15 lines)

Save as `hello.py` and run `python hello.py`:

```python
import automata as at

game = at.Game(title="hello", clear_color=(0.1, 0.1, 0.15, 1.0))
game.load_scene(
    at.Scene()
    .add(at.Entity("camera").add(at.Camera3D(position=(0, 1, 5), target=(0, 0, 0))))
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0, 0, 0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.4, 0.8, 0.4, 1.0)))
    )
)
game.run(lambda ctx: ctx.quit() if ctx.input.just_pressed(at.Key.ESCAPE) else None)
```

A window opens, a green cube sits in it (lit by a default light), and
`Esc` closes it. `game.run` returns the number of frames rendered when the
loop ends.

## The two problems everyone hits once

**1. "The window opens but I see nothing."**
A 3D scene is invisible until it has **both** of these:

- a `Camera3D` entity (the first camera entity wins each frame), and
- at least one entity with **both** `Transform` and `MeshRenderer`.

Check those before anything else. (For atmosphere, add a
`DirectionalLight`; without one, a default shadowless light is used.)

**2. "It says `module 'automata' has no attribute ...`"**
Your installed wheel is older than your checkout. Rebuild and reinstall:

```console
python scripts/build.py
```

## Headless mode (no window)

For tests, CI, and servers, run the same game without a window or GPU:

```python
game = at.Game(headless=True)
game.run(update, max_frames=300)   # stops after 300 frames
```

`MeshRenderer` components are inert in headless mode — physics, input
logic, and events all behave identically. Every example supports a
`--headless` flag; see [Testing](guides/testing.md).

## Next step

[Zero to Hero](zero-to-hero.md) — build a complete platformer, one concept
at a time.
