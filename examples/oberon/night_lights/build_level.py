#!/usr/bin/env python3
"""Writes the Night Lights scene: a torch on a wall and a car's headlights.

    python examples/oberon/night_lights/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/night_lights/level.ron

Neither light comes from a glowing mesh. The torch is a Point Light and the
headlights are two Spot Lights. The flame and the lenses glow only for the
look, with `glow_light=0`. The torch light sits inside the flame (a light
leaves small meshes around it, like a bulb or a flame, out of its own
shadow). All three lights cast shadows: the crate's shadow
falls on the wall behind it, and the post standing in the headlights throws
a long shadow up the wall. Shadows from placed lights need the High preset
or above.
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

STONE = (0.52, 0.47, 0.42, 1.0)
ASPHALT = (0.2, 0.2, 0.22, 1.0)
CAR_RED = (0.55, 0.06, 0.05, 1.0)
TYRE = (0.05, 0.05, 0.05, 1.0)

TORCH = (-2.2, 2.5, -3.45)  # the flame, just in front of the wall
CAR = (2.6, 0.0, 2.6)       # the car's centre on the ground; it faces -Z
HEADLIGHT_X = 0.62          # each headlight's distance from the car's middle


def box(name, position, scale, color, **surface):
    return (
        at.Entity(name)
        .add(at.Transform(position=position, scale=scale))
        .add(at.MeshRenderer.cube(size=1.0, color=color, **surface))
    )


def build():
    scene = at.Scene()
    # Moonlight: dim and blue, so the placed lights carry the scene.
    scene.add(
        at.Entity("Moon").add(
            at.DirectionalLight(
                direction=(0.35, -1.0, 0.55), color=(0.2, 0.25, 0.42), ambient=0.1
            )
        )
    )
    scene.add(at.Entity("Environment").add(at.Environment.preset("night")))
    scene.add(box("Ground", (0, -0.1, 0), (30, 0.2, 30), ASPHALT, roughness=0.85))
    scene.add(box("Wall", (0, 2.0, -4.2), (12, 4.0, 0.5), STONE, roughness=0.9))

    # The torch: an iron bracket, a flame that only glows for the look, and
    # the Point Light that actually lights the wall.
    bx, by, bz = TORCH
    scene.add(box("Torch Bracket", (bx, by - 0.3, bz - 0.2), (0.08, 0.5, 0.08), TYRE,
                  metallic=1.0, roughness=0.5))
    scene.add(
        at.Entity("Torch Flame")
        .add(at.Transform(position=(bx, by, bz)))
        .add(at.MeshRenderer.sphere(size=0.16, color=(1.0, 0.6, 0.2, 1.0),
                                    emissive=(1.0, 0.55, 0.2), emissive_strength=6.0,
                                    glow_light=0.0))
    )
    scene.add(
        at.Entity("Torch")
        .add(at.Transform(position=(bx, by, bz)))
        .add(at.PointLight(color=(1.0, 0.58, 0.28), intensity=3.5, range=10.0, radius=0.08))
    )
    scene.add(box("Crate", (bx + 0.9, 0.4, -3.3), (0.8, 0.8, 0.8), (0.5, 0.36, 0.2, 1.0),
                  roughness=0.8))

    # The car, facing the wall, with a headlight lens and a Spot Light on
    # each side. The spots point along the entity's -Z (forward) and tip
    # down a little toward the road.
    cx, _, cz = CAR
    scene.add(box("Car Body", (cx, 0.62, cz), (1.8, 0.62, 4.0), CAR_RED,
                  roughness=0.35, metallic=0.4))
    scene.add(box("Car Cabin", (cx, 1.18, cz + 0.35), (1.6, 0.5, 2.0), (0.1, 0.12, 0.15, 1.0),
                  roughness=0.15))
    for i, (dx, dz) in enumerate([(-0.9, -1.3), (0.9, -1.3), (-0.9, 1.3), (0.9, 1.3)], start=1):
        scene.add(
            at.Entity(f"Wheel {i}")
            .add(at.Transform(position=(cx + dx, 0.35, cz + dz)))
            .add(at.MeshRenderer.sphere(size=0.7, color=TYRE, roughness=0.9))
        )
    front = cz - 2.0
    for side, dx in (("Left", -HEADLIGHT_X), ("Right", HEADLIGHT_X)):
        scene.add(
            at.Entity(f"Headlight Lens {side}")
            .add(at.Transform(position=(cx + dx, 0.72, front), scale=(1.0, 0.7, 0.4)))
            .add(at.MeshRenderer.sphere(size=0.26, color=(1.0, 1.0, 1.0, 1.0),
                                        emissive=(1.0, 0.95, 0.85), emissive_strength=4.0,
                                        glow_light=0.0))
        )
        scene.add(
            at.Entity(f"Headlight {side}")
            .add(at.Transform(position=(cx + dx, 0.72, front - 0.05),
                              rotation_euler=(math.radians(-9.0), 0.0, 0.0)))
            .add(at.SpotLight(color=(1.0, 0.95, 0.85), intensity=18.0, outer_angle=26.0,
                              inner_angle=12.0, range=22.0))
        )
    # Something standing in the beams, to throw a shadow up the wall.
    scene.add(box("Post", (cx - 0.2, 0.9, -1.6), (0.25, 1.8, 0.25), (0.6, 0.6, 0.6, 1.0)))

    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(-6.5, 3.2, 7.5)))
        .add(at.Camera3D(position=(-6.5, 3.2, 7.5), target=(0.4, 1.0, -2.0)))
    )
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
