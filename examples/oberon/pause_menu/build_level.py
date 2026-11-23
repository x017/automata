#!/usr/bin/env python3
"""Writes the Pause Menu scene: game UI drawn by scripts with ``ctx.ui``.

    python examples/oberon/pause_menu/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/pause_menu/level.ron

Three shapes turn on pedestals under a dusk sky. The Menu entity's script
(``scripts/menu.py``) draws everything else: a pause menu, a settings
screen wired to ``ctx.settings``, and the in-game HUD with a spin meter
and a Menu button. Clicking in the game kicks the shapes
(``scripts/spin.py``), but clicks on the UI don't.
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

FLOOR = (0.13, 0.13, 0.15, 1.0)
STONE = (0.32, 0.31, 0.3, 1.0)
SHAPES = [
    # name, position, size, colour, speed (rad/s)
    ("Cube", (-2.4, 1.6, 0.0), (1.0, 1.0, 1.0), (0.95, 0.42, 0.18, 1.0), 1.2),
    ("Tower", (0.0, 1.85, -0.6), (0.6, 1.5, 0.6), (0.92, 0.9, 0.86, 1.0), -0.8),
    ("Slab", (2.4, 1.45, 0.0), (1.5, 0.3, 0.9), (0.2, 0.55, 0.95, 1.0), 1.7),
]


def build():
    scene = at.Scene()
    scene.add(at.Entity("Environment").add(at.Environment.preset("dusk")))
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(direction=(-0.6, -1.0, -0.4), color=(1.0, 0.82, 0.62),
                                ambient=0.25)
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
            .add(at.Transform(position=(x, 0.45, z), scale=(1.2, 0.9, 1.2)))
            .add(at.MeshRenderer.cube(size=1.0, color=STONE, roughness=0.9))
        )
        shape = (
            at.Entity(name)
            .add(at.Transform(position=(x, y, z), scale=size, rotation_euler=(0.0, 0.5, 0.0)))
            .add(at.MeshRenderer.cube(size=1.0, color=color, roughness=0.35, metallic=0.1))
        )
        shape.script = "scripts/spin.py"
        shape.attributes["speed"] = speed
        scene.add(shape)

    menu = at.Entity("Menu")
    menu.script = "scripts/menu.py"
    scene.add(menu)
    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(0.0, 3.2, 7.5)))
        .add(at.Camera3D(position=(0.0, 3.2, 7.5), target=(0.0, 1.3, 0.0)))
    )
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
