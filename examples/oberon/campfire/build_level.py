#!/usr/bin/env python3
"""Writes the Campfire scene: a fire that burns, smokes and throws embers.

    python examples/oberon/campfire/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/campfire/level.ron

The fire is three Particle Emitters on the log pile: the "fire" preset for
the flames, "smoke" rising above them (lit, so the firelight colours it), and
the "sparks" preset turned into a slow trickle of embers. A script on the
embers makes the fire crackle now and then with `ctx.particles.emit`. The
flames light nothing themselves; a Point Light in the fire does, so the
stones, logs and seats around it glow orange and cast shadows (High and up).
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

GROUND = (0.16, 0.2, 0.12, 1.0)
DIRT = (0.22, 0.17, 0.12, 1.0)
STONE = (0.42, 0.4, 0.38, 1.0)
WOOD = (0.33, 0.21, 0.12, 1.0)
CHARRED = (0.07, 0.05, 0.04, 1.0)
BARK = (0.24, 0.17, 0.11, 1.0)
LEAVES = (0.1, 0.22, 0.12, 1.0)

FIRE = (0.0, 0.0, 0.0)  # the middle of the fire pit, on the ground


def box(name, position, scale, color, rotation=(0.0, 0.0, 0.0), **surface):
    return (
        at.Entity(name)
        .add(at.Transform(position=position, scale=scale, rotation_euler=rotation))
        .add(at.MeshRenderer.cube(size=1.0, color=color, **surface))
    )


def ball(name, position, size, color, scale=(1.0, 1.0, 1.0), **surface):
    return (
        at.Entity(name)
        .add(at.Transform(position=position, scale=scale))
        .add(at.MeshRenderer.sphere(size=size, color=color, **surface))
    )


def build():
    scene = at.Scene()
    scene.add(
        at.Entity("Moon").add(
            at.DirectionalLight(
                direction=(-0.4, -1.0, 0.3), color=(0.16, 0.2, 0.34), ambient=0.08
            )
        )
    )
    scene.add(at.Entity("Environment").add(at.Environment.preset("night")))
    scene.add(box("Ground", (0, -0.1, 0), (40, 0.2, 40), GROUND, roughness=0.95))
    scene.add(
        at.Entity("Fire Pit")
        .add(at.Transform(position=(0, 0.005, 0), scale=(1.0, 0.01, 1.0)))
        .add(at.MeshRenderer.sphere(size=2.2, color=DIRT, roughness=1.0))
    )

    # A ring of stones around the pit.
    for i in range(10):
        a = i / 10 * math.tau
        r = 0.95
        scene.add(
            ball(f"Stone {i + 1}", (math.cos(a) * r, 0.08, math.sin(a) * r), 0.34, STONE,
                 scale=(1.0, 0.6, 0.85 + 0.15 * math.sin(i * 2.3)), roughness=0.9)
        )

    # The log pile: a teepee of four logs leaning in, charred at the core.
    for i in range(4):
        a = i / 4 * math.tau + 0.4
        x, z = math.cos(a) * 0.28, math.sin(a) * 0.28
        scene.add(
            box(f"Log {i + 1}", (x, 0.28, z), (0.12, 0.7, 0.12), WOOD,
                rotation=(math.sin(a) * -0.55, 0.0, math.cos(a) * 0.55), roughness=0.85)
        )
    scene.add(ball("Embers Bed", (0, 0.03, 0), 0.6, CHARRED, scale=(1.0, 0.25, 1.0),
                   emissive=(1.0, 0.3, 0.05), emissive_strength=1.5, glow_light=0.0))

    # The fire itself: flames, smoke above them, and a trickle of embers.
    fx, fy, fz = FIRE
    scene.add(
        at.Entity("Flames")
        .add(at.Transform(position=(fx, fy + 0.1, fz)))
        .add(at.ParticleEmitter.preset("fire"))
    )
    scene.add(
        at.Entity("Smoke")
        .add(at.Transform(position=(fx, fy + 1.1, fz)))
        # Darker than the preset: this close, the firelight brightens it.
        .add(at.ParticleEmitter.preset("smoke", color=(0.1, 0.095, 0.09, 0.55),
                                       color_end=(0.22, 0.22, 0.24, 0.0)))
    )
    embers = (
        at.Entity("Embers")
        .add(at.Transform(position=(fx, fy + 0.25, fz)))
        .add(
            at.ParticleEmitter.preset(
                "sparks",
                rate=5.0,
                burst=0,
                one_shot=False,
                cone_angle=25.0,
                speed=(1.0, 2.6),
                gravity=-0.6,
                drag=0.6,
                lifetime=(1.2, 2.4),
                size=(0.035, 0.015),
            )
        )
    )
    embers.script = "scripts/crackle.py"
    scene.add(embers)
    scene.add(
        at.Entity("Firelight")
        # Above the log pile, so the logs don't fan shadows across the ground.
        .add(at.Transform(position=(fx, fy + 0.95, fz)))
        .add(at.PointLight(color=(1.0, 0.55, 0.22), intensity=4.0, range=12.0, radius=0.25))
    )

    # Somewhere to sit, and trees at the edge of the light.
    scene.add(box("Bench Log", (-2.3, 0.22, 0.4), (0.45, 0.45, 2.2), BARK,
                  rotation=(0.0, 0.25, 0.0), roughness=0.9))
    scene.add(box("Stump", (1.9, 0.25, -1.4), (0.6, 0.5, 0.6), BARK, roughness=0.9))
    for i, (x, z, h) in enumerate([(-4.5, -4.0, 4.5), (3.8, -5.0, 5.5), (-6.0, 2.5, 4.0),
                                   (6.5, -2.0, 5.0), (0.5, -7.0, 6.0)], start=1):
        scene.add(box(f"Trunk {i}", (x, h * 0.3, z), (0.35, h * 0.6, 0.35), BARK))
        scene.add(ball(f"Crown {i}", (x, h * 0.75, z), h * 0.45, LEAVES, roughness=0.9))

    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(3.6, 1.9, 4.6)))
        .add(at.Camera3D(position=(3.6, 1.9, 4.6), target=(0.0, 0.7, 0.0)))
    )
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
