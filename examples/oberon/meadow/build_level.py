#!/usr/bin/env python3
"""Writes Meadow, the grass and foliage example.

    python examples/oberon/meadow/build_level.py   # rewrites level.ron
    cargo run -p oberon --release examples/oberon/meadow/level.ron

A windy meadow to walk through. The grass sways in the Environment's wind
and parts round the runner (she has a Character Controller) and round the
ball when it rolls. A path is cut through the field with erase strokes,
patches past its edge are painted on, and rocks and flowers are foliage
scatter: one model drawn at many spots.

In Oberon, select **Meadow**, **Wildflowers** or **Poppies** and paint more
with the brush in the Inspector (Paint / Erase, Size, Strength).

The runner model is Ponytail Park's (``../ponytail_park/make_runner.py``,
CC0); the rocks, flowers and trees are ``make_foliage.py``'s (CC0).
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
RUNNER = "models/runner.glb"
SOIL = (0.17, 0.2, 0.09, 1.0)
WIND = 0.55  # the Environment's wind, 0..1
WIND_DIRECTION = 30.0  # degrees around +Y (0 = toward +X)
FIELD = 30.0  # the meadow's square, metres
START = (0.0, 0.0, 7.0)


def path_points(steps=24):
    """The winding footpath across the field (x, z), from west to east."""
    for i in range(steps + 1):
        x = -FIELD / 2 + FIELD * i / steps
        yield x, 3.0 * math.sin(x * 0.25) + 2.0


def meadow():
    """The main field: a square, a path cut through it, patches past it."""
    grass = at.Grass(area=(FIELD, FIELD), density=55.0, height=0.55, seed=1)
    for x, z in path_points():
        grass = grass.erase((x, 0.0, z), radius=1.1)
    # Painted on past the square's edge, as the brush does.
    for x, z, radius in ((17.5, -6.0, 4.0), (-17.0, -11.0, 5.0), (12.0, 17.0, 3.5)):
        grass = grass.paint((x, 0.0, z), radius=radius)
    return grass


def wildflowers():
    """Tall golden grass in a few soft patches."""
    grass = at.Grass(
        density=30.0, height=1.05, height_variation=0.5, width=0.05,
        base_color=(0.12, 0.16, 0.04), tip_color=(0.75, 0.62, 0.22),
        color_variation=0.4, seed=2,
    )
    for x, z, radius in ((-8.0, -7.0, 4.0), (-4.5, -9.5, 3.0), (9.0, -9.0, 3.5), (7.0, 10.0, 2.5)):
        grass = grass.paint((x, 0.0, z), radius=radius, strength=0.9)
    return grass


def flowers(seed, centres, density):
    scatter = at.Scatter(density=density, scale=(0.7, 1.3), align=0.0, seed=seed)
    for x, z, radius in centres:
        scatter = scatter.paint((x, 0.0, z), radius=radius)
    return scatter


def trees():
    """A ring of woodland round the meadow, painted on in patches."""
    scatter = at.Scatter(density=0.06, scale=(0.9, 1.6), align=0.0, seed=7)
    for i in range(14):
        a = 2 * math.pi * i / 14
        scatter = scatter.paint((24.0 * math.cos(a), 0.0, 24.0 * math.sin(a)), radius=5.0)
    return scatter


def scattered(name, model, scatter):
    return (
        at.Entity(name)
        .add(at.Transform())
        .add(at.MeshRenderer.model(model))
        .add(scatter)
    )


def build():
    at.set_asset_root(HERE)  # model paths are relative to this folder
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(direction=(-0.45, -0.6, -0.5), color=(1.0, 0.94, 0.82), ambient=0.4)
        )
    )
    scene.add(at.Entity("Environment").add(at.Environment(
        sky_color=(0.62, 0.72, 0.85), ground_color=(0.32, 0.3, 0.22), exposure=1.1,
        tonemapping="aces", sky=True, clouds=0.35, fog=True, fog_color=(0.7, 0.78, 0.86),
        fog_density=0.02, fog_start=20.0, wind=WIND, wind_direction=WIND_DIRECTION,
    )))

    scene.add(
        at.Entity("Ground")
        .add(at.Transform(position=(0, -0.15, 0), scale=(2000, 0.3, 2000)))
        .add(at.MeshRenderer.cube(size=1.0, color=SOIL, roughness=0.95))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(2000, 0.3, 2000))  # full size: physics ignores scale
    )
    scene.add(at.Entity("Meadow").add(at.Transform()).add(meadow()))
    scene.add(at.Entity("Wildflowers").add(at.Transform()).add(wildflowers()))

    # Scatter: each entity's model, drawn wherever the scatter puts it.
    scene.add(scattered("Rocks", "models/rock.glb", at.Scatter(
        area=(44.0, 44.0), density=0.015, scale=(0.6, 2.4), align=0.6, seed=4)))
    scene.add(scattered("Poppies", "models/poppy.glb", flowers(
        5, [(-6.0, 6.0, 3.0), (5.0, -3.0, 2.5), (-11.0, -2.0, 2.0)], 2.0)))
    scene.add(scattered("Daisies", "models/daisy.glb", flowers(
        6, [(4.0, 9.0, 3.5), (11.0, 1.0, 3.0), (-3.0, -4.0, 2.0)], 3.0)))
    scene.add(scattered("Trees", "models/tree.glb", trees()))

    # A ball to kick through the grass: moving bodies part it too.
    ball = (
        at.Entity("Ball")
        .add(at.Transform(position=(2.0, 0.35, 4.0)))
        .add(at.MeshRenderer.sphere(0.7, color=(0.95, 0.85, 0.2, 1.0), roughness=0.5))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.ball(0.35).friction(0.6))
    )
    ball.script = "scripts/ball.py"
    scene.add(ball)

    # The runner: a Character Controller, so grass bends away from her.
    runner = (
        at.Entity("Runner")
        .add(at.Transform(position=START, rotation_euler=(0, math.pi, 0)))
        .add(at.MeshRenderer.model(RUNNER))
        .add(at.Animation("Idle"))
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider3D.capsule(0.55, 0.28).offset(0.0, 0.85, 0.0))
        .add(at.CharacterController(snap_to_ground=0.3))
        .add(at.CharacterVelocity())
    )
    runner.script = "scripts/player.py"
    scene.add(runner)

    eye = (0.0, 1.9, 11.5)
    camera = (
        at.Entity("Main Camera")
        .add(at.Transform(position=eye))
        .add(at.Camera3D(position=eye, target=(0.0, 1.0, 7.0)))
    )
    camera.script = "scripts/follow_camera.py"
    scene.add(camera)
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
