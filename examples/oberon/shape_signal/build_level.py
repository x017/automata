#!/usr/bin/env python3
"""Writes the Shape Signal scene: a game whose screens come from Oberon's UI
editor.

    python examples/oberon/shape_signal/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/shape_signal/level.ron

Three shapes stand on pedestals. One at a time, one of them lifts, and the
player picks it with the HUD's buttons (or 1, 2, 3) before it drops. The
title screen, the HUD and the results screen are UI documents in ``ui/``,
made and themed in the UI editor; ``scripts/game.py`` only fills them in
and reacts to their buttons, by name.
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

FLOOR = (0.1, 0.11, 0.14, 1.0)
STONE = (0.3, 0.3, 0.33, 1.0)
SHAPES = [
    # name, position, size, colour, idle speed (rad/s)
    ("Cube", (-2.6, 1.6, 0.0), (1.0, 1.0, 1.0), (0.95, 0.42, 0.18, 1.0), 0.6),
    ("Tower", (0.0, 1.85, -0.4), (0.6, 1.5, 0.6), (0.3, 0.85, 0.75, 1.0), -0.5),
    ("Slab", (2.6, 1.45, 0.0), (1.5, 0.3, 0.9), (0.95, 0.35, 0.55, 1.0), 0.7),
]


def build():
    scene = at.Scene()
    scene.add(at.Entity("Environment").add(at.Environment.preset("night")))
    scene.add(
        at.Entity("Moon").add(
            at.DirectionalLight(direction=(-0.4, -1.0, -0.6), color=(0.75, 0.82, 1.0),
                                ambient=0.35)
        )
    )
    scene.add(
        at.Entity("Floor")
        .add(at.Transform(position=(0.0, -0.05, 0.0), scale=(30.0, 0.1, 30.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=FLOOR, roughness=0.85))
    )
    for name, (x, y, z), size, color, speed in SHAPES:
        scene.add(
            at.Entity(f"{name} Pedestal")
            .add(at.Transform(position=(x, 0.45, z), scale=(1.3, 0.9, 1.3)))
            .add(at.MeshRenderer.cube(size=1.0, color=STONE, roughness=0.9))
        )
        shape = (
            at.Entity(name)
            .add(at.Transform(position=(x, y, z), scale=size, rotation_euler=(0.0, 0.5, 0.0)))
            .add(at.MeshRenderer.cube(size=1.0, color=color, roughness=0.3, metallic=0.1))
        )
        shape.script = "scripts/shape.py"
        shape.attributes["speed"] = speed
        scene.add(shape)
        scene.add(
            at.Entity(f"{name} Glow")
            .add(at.Transform(position=(x, 2.6, z + 0.8)))
            .add(at.PointLight(color=color[:3], intensity=6.0, range=4.0))
        )

    game = at.Entity("Game")
    game.script = "scripts/game.py"
    game.attributes["seed"] = 7
    scene.add(game)
    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(0.0, 3.0, 8.0)))
        .add(at.Camera3D(position=(0.0, 3.0, 8.0), target=(0.0, 1.5, 0.0)))
    )
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
