#!/usr/bin/env python3
"""Writes Ponytail Park, the spring bones example.

    python examples/oberon/ponytail_park/build_level.py   # rewrites level.ron
    cargo run -p oberon --release examples/oberon/ponytail_park/level.ron

A park with a running track and four copies of one runner. Her ponytail
and skirt have no animation of their own: ``models/runner.glb.import.ron``
makes them spring bones, so they swing whenever she moves.

- **Runner** (you): ``scripts/player.py`` walks (W A S D) and runs (Shift).
- **Jogger**: ``scripts/jogger.py`` runs up the track, stops dead and
  turns round. The hair streams out behind and swings on when she stops.
- **Stroller**: ``scripts/stroller.py`` walks round the flowerbed.
- **Spectator**: just an Animation set to Idle. No script needed.

The model is made by ``make_runner.py`` (CC0).
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
RUNNER = "models/runner.glb"
GRASS = (0.2, 0.36, 0.14, 1.0)
TRACK = (0.62, 0.27, 0.2, 1.0)
LINE = (0.92, 0.9, 0.86, 1.0)
PATH = (0.55, 0.5, 0.42, 1.0)
SOIL = (0.3, 0.2, 0.13, 1.0)
LEAVES = (0.14, 0.32, 0.12, 1.0)
WOOD = (0.36, 0.2, 0.1, 1.0)
FLOWERS = [(0.9, 0.3, 0.4, 1.0), (0.95, 0.8, 0.25, 1.0), (0.6, 0.4, 0.85, 1.0)]

TRACK_Z = -3.0  # the track runs along X here
JOG_START = (-4.5, 0.0, TRACK_Z)
BED = (-7.0, -6.0)  # the flowerbed the Stroller walks round
STROLL_RADIUS = 3.2


def environment(preset, **changes):
    """A preset with some fields changed (Environment values are read-only)."""
    base = at.Environment.preset(preset)
    fields = (
        "sky_color", "ground_color", "background", "exposure", "tonemapping",
        "fog", "fog_color", "fog_density", "fog_start",
        "sky", "zenith_color", "sun_size", "sun_glow", "clouds", "lens_flare",
        "ambient_occlusion",
    )
    return at.Environment(**{**{f: getattr(base, f) for f in fields}, **changes})


def block(name, position, size, color, roughness=0.9):
    """A box of ``size`` centred on ``position``."""
    return (
        at.Entity(name)
        .add(at.Transform(position=position, scale=size))
        .add(at.MeshRenderer.cube(size=1.0, color=color, roughness=roughness))
    )


def runner(name, position, yaw, animation, script=None):
    entity = (
        at.Entity(name)
        .add(at.Transform(position=position, rotation_euler=(0, math.radians(yaw), 0)))
        .add(at.MeshRenderer.model(RUNNER))
        .add(animation)
    )
    if script:
        entity.script = script
    return entity


def build():
    at.set_asset_root(HERE)  # model paths are relative to this folder
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(
                direction=(-0.5, -0.7, -0.45), color=(1.0, 0.95, 0.85), ambient=0.35
            )
        )
    )
    scene.add(at.Entity("Environment").add(environment(
        "daylight", tonemapping="aces", fog_density=0.01, fog_start=20.0
    )))

    # Grass, the running track with its lane lines, and a path across.
    scene.add(block("Grass", (0, -0.15, -2), (40, 0.3, 40), GRASS))
    scene.add(block("Track", (0, 0.005, TRACK_Z), (14, 0.02, 2.4), TRACK, roughness=0.95))
    for i, z in enumerate((TRACK_Z - 1.2, TRACK_Z, TRACK_Z + 1.2), 1):
        scene.add(block(f"Lane Line {i}", (0, 0.02, z), (14, 0.012, 0.06), LINE))
    scene.add(block("Path", (2.5, 0.004, 3), (2.0, 0.02, 10), PATH))

    # The flowerbed the Stroller walks round.
    bx, bz = BED
    scene.add(block("Flowerbed", (bx, 0.12, bz), (2.6, 0.24, 2.6), SOIL))
    for i in range(9):
        fx, fz = bx - 0.8 + 0.8 * (i % 3), bz - 0.8 + 0.8 * (i // 3)
        scene.add(
            at.Entity(f"Flowers {i + 1}")
            .add(at.Transform(position=(fx, 0.38, fz), scale=(0.5, 0.36, 0.5)))
            .add(at.MeshRenderer.sphere(0.5, color=FLOWERS[i % 3], roughness=0.8))
        )

    # Benches by the track, and trees round the edge.
    for i, x in enumerate((-3.0, 3.0), 1):
        scene.add(block(f"Bench {i}", (x, 0.22, TRACK_Z - 2.3), (2.0, 0.45, 0.55), WOOD))
    for i, (x, z) in enumerate([(-11, 2), (9, -9), (11, 1), (-3, -12), (5, -13), (-12, -11)], 1):
        scene.add(block(f"Trunk {i}", (x, 1.2, z), (0.4, 2.4, 0.4), WOOD))
        scene.add(
            at.Entity(f"Canopy {i}")
            .add(at.Transform(position=(x, 3.3, z), scale=(3.0, 2.6, 3.0)))
            .add(at.MeshRenderer.sphere(0.5, color=LEAVES, roughness=0.9))
        )

    scene.add(runner("Runner", (0.0, 0.0, 3.0), 180, at.Animation("Idle"), "scripts/player.py"))
    scene.add(runner("Jogger", JOG_START, 90, at.Animation("Idle"), "scripts/jogger.py"))
    scene.add(
        runner(
            "Stroller", (bx, 0.0, bz + STROLL_RADIUS), 90, at.Animation("Walk"),
            "scripts/stroller.py",
        )
    )
    scene.add(runner("Spectator", (1.2, 0.0, TRACK_Z - 2.2), 200, at.Animation("Idle")))

    eye = (0.0, 1.8, 7.5)
    camera = (
        at.Entity("Main Camera")
        .add(at.Transform(position=eye))
        .add(at.Camera3D(position=eye, target=(0.0, 1.1, 3.0)))
    )
    camera.script = "scripts/follow_camera.py"
    scene.add(camera)
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
