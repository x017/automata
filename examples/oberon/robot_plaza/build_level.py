#!/usr/bin/env python3
"""Writes Robot Plaza, the animation example.

    python examples/oberon/robot_plaza/build_level.py   # rewrites level.ron
    cargo run -p oberon --release examples/oberon/robot_plaza/level.ron

A small town square with five copies of one animated robot. You drive one
of them; the others each show a different way to use an Animation:

- **Robot** (yours): ``scripts/player.py`` blends between Idle, Walking,
  Running, Jump and Wave as you move.
- **Greeter**: ``scripts/greeter.py`` plays Wave once when you walk up.
- **Dancer**: just an Animation set to Dance. No script needed.
- **Sitter**: Sitting with Loop off. One-shot clips hold their last frame.
- **Slowpoke**: Walking at Speed 0.35, on the spot.

The model is ``models/robot.glb`` (RobotExpressive, CC0; see CREDITS.md).
Its ``robot.glb.import.ron`` scales it to 1.8 m and moves its origin to
the middle, so it lines up with the Character Controller's capsule.
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
ROBOT = "models/robot.glb"
STAND = 0.89  # the robot's origin height when its feet are on the ground
STONE = (0.42, 0.38, 0.34, 1.0)
DARK_STONE = (0.26, 0.24, 0.23, 1.0)
GRASS = (0.17, 0.32, 0.12, 1.0)
LEAVES = (0.12, 0.3, 0.1, 1.0)
WOOD = (0.36, 0.2, 0.1, 1.0)

# The robots that aren't you: name, position, yaw (degrees), Animation.
CROWD = [
    ("Greeter", (3.0, 0.0, -2.0), -60, at.Animation("Idle")),
    ("Dancer", (-4.5, 0.0, -5.0), 30, at.Animation("Dance")),
    ("Sitter", (5.2, 0.0, -6.8), -30, at.Animation("Sitting", looping=False)),
    ("Slowpoke", (-3.5, 0.0, 1.5), 110, at.Animation("Walking", speed=0.35)),
]
SCRIPTS = {"Greeter": "scripts/greeter.py"}


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


def block(name, position, size, color, solid=True):
    """A box of ``size`` centred on ``position``."""
    entity = (
        at.Entity(name)
        .add(at.Transform(position=position, scale=size))
        .add(at.MeshRenderer.cube(size=1.0, color=color, roughness=0.85))
    )
    if solid:
        entity.add(at.RigidBody.fixed()).add(at.Collider3D.cuboid(*size))
    return entity


def robot(name, position, yaw, animation):
    x, _, z = position
    return (
        at.Entity(name)
        .add(at.Transform(position=(x, STAND, z), rotation_euler=(0, math.radians(yaw), 0)))
        .add(at.MeshRenderer.model(ROBOT))
        .add(animation)
    )


def build():
    at.set_asset_root(HERE)  # model paths are relative to this folder
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(
                direction=(-0.45, -0.75, -0.5), color=(1.0, 0.95, 0.86), ambient=0.35
            )
        )
    )
    scene.add(at.Entity("Environment").add(environment(
        "daylight", tonemapping="aces", fog_density=0.012, fog_start=18.0
    )))

    # The square: grass all round, a paved plaza, a fountain in the middle.
    scene.add(block("Grass", (0, -0.15, -2), (40, 0.3, 40), GRASS))
    scene.add(block("Plaza", (0, -0.04, -2), (16, 0.1, 16), STONE, solid=False))
    scene.add(block("Fountain Basin", (0, 0.3, -2), (3.2, 0.6, 3.2), DARK_STONE))
    scene.add(
        at.Entity("Fountain Water")
        .add(at.Transform(position=(0, 0.62, -2), scale=(2.8, 0.06, 2.8)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.35, 0.6, 0.85, 1.0), roughness=0.1))
    )
    scene.add(block("Fountain Pillar", (0, 1.1, -2), (0.5, 1.4, 0.5), STONE))

    # Benches and planters round the edge.
    for i, (x, z, yaw) in enumerate([(5.5, -6, 0), (-5.5, 2, 90), (-5.5, -6, 90)], 1):
        across = yaw == 90
        size = (0.6, 0.45, 2.2) if across else (2.2, 0.45, 0.6)
        scene.add(block(f"Bench {i}", (x, 0.22, z), size, WOOD))
    for i, (x, z) in enumerate([(7, 5), (-7, 5), (7, -9), (-7, -9)], 1):
        scene.add(block(f"Planter {i}", (x, 0.35, z), (1.4, 0.7, 1.4), DARK_STONE))
        scene.add(
            at.Entity(f"Shrub {i}")
            .add(at.Transform(position=(x, 1.4, z), scale=(1.8, 1.5, 1.8)))
            .add(at.MeshRenderer.sphere(0.5, color=LEAVES, roughness=0.9))
        )
    # A few trees beyond the paving.
    for i, (x, z) in enumerate([(-11, -4), (11, -1), (-9, -13), (10, -13), (-12, 6)], 1):
        scene.add(block(f"Trunk {i}", (x, 1.2, z), (0.4, 2.4, 0.4), WOOD))
        scene.add(
            at.Entity(f"Canopy {i}")
            .add(at.Transform(position=(x, 3.3, z), scale=(2.8, 2.4, 2.8)))
            .add(at.MeshRenderer.sphere(0.5, color=LEAVES, roughness=0.9))
        )

    # You.
    player = (
        robot("Robot", (0, 0, 3), 180, at.Animation("Idle"))
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider3D.capsule(0.55, 0.32))
        .add(at.CharacterController(snap_to_ground=0.3))
        .add(at.CharacterVelocity())
    )
    player.script = "scripts/player.py"
    scene.add(player)

    for name, position, yaw, animation in CROWD:
        bot = robot(name, position, yaw, animation)
        if name in SCRIPTS:
            bot.script = SCRIPTS[name]
        scene.add(bot)

    eye = (0.0, 3.5, 9.5)
    camera = (
        at.Entity("Main Camera")
        .add(at.Transform(position=eye))
        .add(at.Camera3D(position=eye, target=(0.0, 1.0, 0.0)))
    )
    camera.script = "scripts/follow_camera.py"
    scene.add(camera)
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
