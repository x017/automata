#!/usr/bin/env python3
"""Writes the finished Coin Run scenes from the Oberon tutorials.

    python examples/oberon/coin_run/build_level.py   # then open one in Oberon
    cargo run -p oberon --release examples/oberon/coin_run/level.ron

It writes three files, one per look from "Lighting & mood":

    level.ron         bright afternoon (the main level)
    level_dusk.ron    golden dusk
    level_night.ron   moonlit night

Building the scene in code is optional. The tutorials build the same thing
by hand in Oberon, and both produce an ordinary scene file.
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
GOLD = (1.0, 0.78, 0.2, 1.0)
COINS = [(3, 1.0, 0), (6, 1.0, -2), (9, 1.0, -5), (6, 3.2, -9), (0, 1.0, -8), (-4, 1.0, -4)]

# Camera rail & intro shot: where the camera flies on Play, and what it looks
# at. The last shot matches the follow camera's starting framing, so the
# hand-off is seamless.
SHOTS = [
    ((-10.0, 7.0, 10.0), (2.0, 1.0, -4.0)),
    ((14.0, 4.5, -1.0), (6.0, 2.5, -8.0)),
    ((0.0, 4.4, 8.0), (0.0, 1.5, 0.0)),
]


def sun_direction(azimuth, elevation):
    """The direction light travels, from the Graphics tab's angles (degrees)."""
    az, el = math.radians(azimuth), math.radians(elevation)
    return (-math.cos(el) * math.sin(az), -math.sin(el), -math.cos(el) * math.cos(az))


def hex_color(code):
    code = code.lstrip("#")
    return tuple(int(code[i : i + 2], 16) / 255.0 for i in (0, 2, 4))


def environment(preset, **changes):
    """A preset with some fields changed (Environment values are read-only)."""
    base = at.Environment.preset(preset)
    fields = (
        "sky_color", "ground_color", "background", "exposure", "tonemapping",
        "fog", "fog_color", "fog_density", "fog_start",
    )
    return at.Environment(**{**{f: getattr(base, f) for f in fields}, **changes})


# The three looks from "Lighting & mood", exactly as set in the Graphics tab.
MOODS = {
    "afternoon": {
        "sun": dict(azimuth=35, elevation=55, color="#FFF5E0", ambient=0.45),
        "environment": lambda: environment(
            "daylight", tonemapping="aces", exposure=1.1, fog_density=0.01, fog_start=15.0
        ),
    },
    "dusk": {
        "sun": dict(azimuth=200, elevation=8, color="#FFB36B", ambient=0.3),
        "environment": lambda: environment("dusk", tonemapping="agx", fog_density=0.03, fog_start=8.0),
    },
    "night": {
        "sun": dict(azimuth=320, elevation=40, color="#9DB4FF", ambient=0.2),
        "environment": lambda: environment("night", exposure=0.8, fog_start=2.0),
    },
}
FILES = {"afternoon": "level.ron", "dusk": "level_dusk.ron", "night": "level_night.ron"}


def build(mood="afternoon"):
    look = MOODS[mood]
    sun = look["sun"]
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(
                direction=sun_direction(sun["azimuth"], sun["elevation"]),
                color=hex_color(sun["color"]),
                ambient=sun["ambient"],
            )
        )
    )
    scene.add(at.Entity("Environment").add(look["environment"]()))
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
    for i, (pos, look_at) in enumerate(SHOTS, start=1):
        shot = at.Entity(f"Shot {i}").add(at.Transform(position=pos))
        shot.attributes["look"] = look_at
        scene.add(shot)
    camera = (
        at.Entity("Main Camera")
        .add(at.Transform(position=SHOTS[0][0]))
        .add(at.Camera3D(position=SHOTS[0][0], target=SHOTS[0][1]))
    )
    # camera_rail.py flies the intro, then becomes follow_camera.py's
    # third-person camera. Attach scripts/follow_camera.py instead to skip it.
    camera.script = "scripts/camera_rail.py"
    scene.add(camera)
    rules = at.Entity("Rules").add(at.Transform(position=(-4, 0.5, 3)))
    rules.script = "scripts/rules.py"
    scene.add(rules)
    return scene


if __name__ == "__main__":
    for mood, name in FILES.items():
        path = os.path.join(HERE, name)
        build(mood).save(path)
        print(f"wrote {path}")
