#!/usr/bin/env python3
"""Writes the FPS example's scene, laid out like Kenney's Starter Kit FPS.

    python examples/oberon/fps/build_level.py   # writes level.ron
    cargo run -p oberon --release examples/oberon/fps/level.ron

Everything here can be done by hand in Oberon instead: drag the models from
the Assets tab onto the scene, click Fit Collider, attach the scripts and
add the attributes. Building it in code keeps the example reproducible.

The scenery is bare colliders without rigid bodies: static geometry that
scripts never have to think about. The drones are the same, so the player's
ray casts hit them while enemy.py moves them around.

The blasters and the drone face +Z in their files. Their import settings
(``models/<name>.glb.import.ron``, the Inspector's Import section) turn them
round to face -Z, the engine's forward, so scripts can aim them directly.
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

# Wall sizes (from the files' bounds): width, height, depth. Every model
# stands on its origin, so box colliders are raised by half their height.
# Platforms collide with their own triangles (mesh colliders), so their
# bevelled edges are exact.
SIZES = {
    "wall-high": (1.5, 1.7, 0.7),
    "wall-low": (2.2, 0.8, 0.7),
}

# Scenery: model, position, yaw in degrees (Kenney's main.tscn).
LEVEL = [
    ("platform-large-grass", (0.0, -0.5, 0.0), 0),
    ("platform-large-grass", (-2.0, 0.5, -6.0), 15),
    ("platform-large-grass", (-6.0, 1.0, 2.5), -15),
    ("platform-large-grass", (12.0, 2.5, -5.0), -30),
    ("platform-large-grass", (5.0, 0.5, 5.5), 15),
    ("platform", (-2.5, 0.0, 6.5), 0),
    ("platform", (-6.5, 2.5, -2.5), 0),
    ("platform", (2.5, 3.0, -3.5), 0),
    ("platform", (7.0, 1.0, -2.0), -45),
    ("wall-high", (-5.5, 1.5, 4.0), 0),
    ("wall-high", (11.5, 3.0, -5.5), -45),
    ("wall-low", (-1.92, 1.05, -6.9), 15),
    ("wall-low", (6.08, 1.05, 6.6), 180),
]

# Grass tufts on the big platforms (decoration, no colliders).
GRASS = [
    ("grass", (1.6, 0.0, -1.2), 20),
    ("grass-small", (-1.4, 0.0, 1.5), 70),
    ("grass", (-3.4, 1.0, -5.0), 0),
    ("grass-small", (-0.6, 1.0, -7.4), 40),
    ("grass", (-7.2, 1.5, 3.4), 10),
    ("grass", (4.1, 1.0, 6.8), 60),
    ("grass-small", (6.4, 1.0, 4.3), 0),
    ("grass", (13.2, 3.0, -3.9), 30),
]

# Flying drones: position and starting yaw.
ENEMIES = [
    ((-3.5, 2.5, -6.0), 0),
    ((-9.5, 2.5, 1.5), 90),
    ((5.5, 3.5, 9.0), 135),
    ((15.5, 4.0, -7.5), -45),
]

# Clouds around the arena: position and size.
CLOUDS = [
    ((-9.5, 8.5, 20.5), 4.0),
    ((25.5, 6.4, -12.1), 4.0),
    ((6.4, 6.4, -28.6), 4.0),
    ((-2.8, 2.4, 25.4), 3.0),
    ((27.5, 12.0, -5.4), 3.0),
    ((-28.6, 16.3, -4.9), 3.0),
    ((-25.1, 8.8, -24.3), 3.0),
    ((14.1, 10.1, 17.5), 2.0),
    ((-5.1, 2.4, -36.6), 2.0),
    ((-30.1, 2.4, -13.7), 2.0),
    ((-0.9, 13.0, -30.2), 3.0),
]

SPAWN = (0.0, 1.0, 0.0)  # the player's capsule centre


def model(name):
    return at.MeshRenderer.model(f"models/{name}.glb")


def yaw(degrees):
    return (0.0, math.radians(degrees), 0.0)


def build():
    at.set_asset_root(HERE)  # model paths are relative to this folder
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(
                direction=(-0.58, -0.77, 0.27), color=(1.0, 0.98, 0.94), ambient=0.5
            )
        )
    )
    base = at.Environment.preset("daylight")
    scene.add(
        at.Entity("Environment").add(
            at.Environment(
                sky_color=(0.66, 0.69, 0.77),
                ground_color=base.ground_color,
                background=(0.36, 0.39, 0.46),
                exposure=1.05,
                tonemapping="aces",
                fog=True,
                fog_color=(0.36, 0.39, 0.46),
                fog_density=0.015,
                fog_start=20.0,
                sky=True,
                zenith_color=(0.2, 0.26, 0.38),
                clouds=0.6,
            )
        )
    )

    counts = {}
    for name, position, degrees in LEVEL:
        counts[name] = counts.get(name, 0) + 1
        if name in SIZES:
            w, h, d = SIZES[name]
            collider = at.Collider3D.cuboid(w, h, d).offset(0.0, h / 2, 0.0)
        else:
            collider = at.Collider3D.mesh(f"models/{name}.glb")
        scene.add(
            at.Entity(f"{name.replace('-', ' ').title()} {counts[name]}")
            .add(at.Transform(position=position, rotation_euler=yaw(degrees)))
            .add(model(name))
            .add(collider)
        )
    for i, (name, position, degrees) in enumerate(GRASS, start=1):
        scene.add(
            at.Entity(f"Grass {i}")
            .add(at.Transform(position=position, rotation_euler=yaw(degrees)))
            .add(model(name))
        )

    player = (
        at.Entity("Player")
        .add(at.Transform(position=SPAWN))
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider3D.capsule(0.5, 0.3))
        .add(at.CharacterController(snap_to_ground=0.3, autostep=0.3))
        .add(at.CharacterVelocity())
        .add(at.Camera3D(position=(0.0, 1.6, 0.0), target=(0.0, 1.6, -1.0), fov=80.0))
        .add(at.AudioSource("sounds/walking.ogg", volume=0.35, looping=True))
    )
    player.script = "scripts/player.py"
    player.attributes["health"] = 100
    scene.add(player)

    # The held weapons: player.py moves the active one in front of the
    # camera and puts the other away. No colliders, so shots pass through.
    for name in ("blaster", "blaster-repeater"):
        label = "Blaster" if name == "blaster" else "Blaster Repeater"
        scene.add(
            at.Entity(label)
            .add(at.Transform(position=(0.0, -100.0, 0.0), scale=(0.4, 0.4, 0.4)))
            .add(model(name))
        )

    for i, (position, degrees) in enumerate(ENEMIES, start=1):
        enemy = (
            at.Entity(f"Enemy {i}")
            .add(at.Transform(position=position, rotation_euler=yaw(degrees)))
            .add(model("enemy-flying"))
            .add(at.Collider3D.ball(0.6).offset(0.0, 0.45, 0.0))
        )
        enemy.script = "scripts/enemy.py"
        enemy.attributes["health"] = 100
        scene.add(enemy)

    for i, (position, size) in enumerate(CLOUDS, start=1):
        cloud = at.Entity(f"Cloud {i}").add(
            at.Transform(
                position=position,
                rotation_euler=(0.35 * (i % 3), 0.7 * i, 0.25 * (i % 2)),
                scale=(size, size, size),
            )
        ).add(model("cloud"))
        cloud.script = "scripts/cloud.py"
        scene.add(cloud)
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
