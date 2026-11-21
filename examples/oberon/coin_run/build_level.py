#!/usr/bin/env python3
"""Writes level.ron: the finished Coin Run scene from the tutorials.

    python examples/oberon/coin_run/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/coin_run/level.ron

Building the scene in code is optional — the tutorial builds the same thing
by hand in Oberon. Both produce an ordinary scene file.
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
GOLD = (1.0, 0.78, 0.2, 1.0)
COINS = [(3, 1.0, 0), (6, 1.0, -2), (9, 1.0, -5), (6, 3.2, -9), (0, 1.0, -8), (-4, 1.0, -4)]


def build():
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(direction=(-0.45, -1.0, -0.3), color=(1.0, 0.96, 0.88), ambient=0.45)
        )
    )
    scene.add(at.Entity("Environment").add(at.Environment.preset("daylight")))
    scene.add(
        at.Entity("Ground")
        .add(at.Transform(position=(0, -0.1, -4), scale=(30, 0.2, 30)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.36, 0.5, 0.32, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(30, 0.2, 30))  # full size: physics ignores scale
    )
    scene.add(
        at.Entity("Ledge")
        .add(at.Transform(position=(6, 1.0, -9), scale=(4, 2, 3)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.55, 0.5, 0.45, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(4, 2, 3))
    )
    player = (
        at.Entity("Player")
        .add(at.Transform(position=(0, 0.9, 0), scale=(0.8, 1.6, 0.8)))
        .add(at.MeshRenderer.cube(size=1.0, color=(1.0, 0.56, 0.24, 1.0)))
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider3D.cuboid(0.8, 1.6, 0.8))
        .add(at.CharacterController(snap_to_ground=0.3))
        .add(at.CharacterVelocity())
    )
    player.script = "scripts/player.py"
    scene.add(player)
    for i, pos in enumerate(COINS, start=1):
        coin = (
            at.Entity(f"Coin {i}")
            .add(at.Transform(position=pos, scale=(0.5, 0.5, 0.12)))
            .add(at.MeshRenderer.cube(size=1.0, color=GOLD))
        )
        coin.script = "scripts/coin.py"
        coin.attributes["value"] = 5 if pos[1] > 2 else 1
        scene.add(coin)
    camera = (
        at.Entity("Main Camera")
        .add(at.Transform(position=(0, 4.4, 8)))
        .add(at.Camera3D(position=(0, 4.4, 8), target=(0, 1.5, 0)))
    )
    camera.script = "scripts/follow_camera.py"
    scene.add(camera)
    rules = at.Entity("Rules").add(at.Transform(position=(-4, 0.5, 3)))
    rules.script = "scripts/rules.py"
    scene.add(rules)
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
