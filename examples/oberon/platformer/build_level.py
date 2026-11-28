#!/usr/bin/env python3
"""Writes the platformer's scene from the ASCII map below.

    python examples/oberon/platformer/build_art.py    # the images (once)
    python examples/oberon/platformer/build_level.py  # writes level.ron
    cargo run -p oberon --release examples/oberon/platformer/level.ron

Everything here can be done by hand in Oberon: + Create → Tilemap, pick the
tileset in the Inspector and paint with the palette's brush (the Ground
auto-tile picks its edge pieces by itself), + Create → 2D Player, + Create
→ 2D Camera, and so on. Building it in code keeps the example reproducible
and makes the layout easy to change: edit ``MAP`` and run this again.

The map is listed top row first. One character per 1 x 1 cell:

    #  ground (the Ground auto-tile)      B  brick block    C  crate
    b  bush   f  flower   g  tall grass   >  sign post   (scenery, no collision)
    @  the player's start                 o  a coin
    s  a slime (patrols ``SLIME_RANGE`` cells either side)
    F  the goal flag
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

MAP = [
    "................................................................",
    "................................................................",
    "................................................................",
    "................................................................",
    "...........................................o.o..................",
    ".........................................BBBBBBB................",
    "..............................o...............................F.",
    "..................o.o.o...............C..........o.o........####",
    ".........o.o.....BBBBBBB.....###.....CC.....#######.........####",
    "........######.............o.......#####..................######",
    ".....................g...s.....#########....f..g.s...g....######",
    "..@.>gCb....b.f.###########...##########..b.##########..########",
    "###########################...########################..########",
    "###########################...########################..########",
]

SOLID = {"#": -1, "B": 17, "C": 18}
SCENERY = {"b": 19, "f": 20, "g": 21, ">": 22}
SLIME_RANGE = 2.0

# Sprite sheets (see build_art.py).
HERO_CLIPS = [
    at.SpriteClip("idle", 0, 1, fps=2),
    at.SpriteClip("run", 2, 5, fps=12),
    at.SpriteClip("jump", 6),
    at.SpriteClip("fall", 7),
]


def cells(char):
    """Cell coordinates (x, y from the bottom) of every ``char`` in MAP."""
    height = len(MAP)
    for row, line in enumerate(MAP):
        for x, c in enumerate(line):
            if c == char:
                yield x, height - 1 - row


def layer(legend, collide, depth):
    rows = ["".join(c if c in legend else "." for c in line) for line in MAP]
    return at.Tilemap.from_rows(
        rows,
        legend=legend,
        tileset="sprites/tiles.png",
        columns=8,
        rows=4,
        autotiles=[at.AutoTile("Ground", first=0)],
        collide=collide,
        layer=depth,
    )


def build():
    width, height = len(MAP[0]), len(MAP)
    assert all(len(line) == width for line in MAP), "every MAP row is as long"
    scene = at.Scene()

    # The backdrop: sky.py slides it a little behind the camera (parallax).
    sky = (
        at.Entity("Sky")
        .add(at.Transform(position=(width / 2, height / 2, 0.0)))
        .add(at.Sprite("sprites/sky.png", size=(32.0, 18.0), layer=-10))
    )
    sky.script = "scripts/sky.py"
    scene.add(sky)

    scene.add(at.Entity("Ground").add(at.Transform()).add(layer(SOLID, True, 0)))
    scene.add(at.Entity("Scenery").add(at.Transform()).add(layer(SCENERY, False, -1)))

    (sx, sy), = cells("@")
    player = (
        at.Entity("Player")
        .add(at.Transform(position=(sx + 0.5, sy + 0.5, 0.0)))
        .add(at.Sprite("sprites/hero.png", columns=4, rows=2, clips=HERO_CLIPS, play="idle", layer=2))
        .add(at.RigidBody.kinematic_position())
        # A little narrower than the art, so ledges don't catch on a pixel;
        # its bottom meets the sprite's feet.
        .add(at.Collider2D.cuboid(0.6, 0.9).offset(0.0, -0.05))
        .add(at.CharacterController(snap_to_ground=0.2))
        .add(at.CharacterVelocity())
    )
    player.script = "scripts/player.py"
    player.attributes["coins"] = 0
    scene.add(player)

    for i, (x, y) in enumerate(sorted(cells("o")), start=1):
        scene.add(
            at.Entity(f"Coin {i}")
            .add(at.Transform(position=(x + 0.5, y + 0.5, 0.0)))
            .add(
                at.Sprite(
                    "sprites/coin.png",
                    size=(0.75, 0.75),
                    columns=4,
                    clips=[at.SpriteClip("spin", 0, 3, fps=8)],
                    play="spin",
                    layer=1,
                )
            )
        )

    for i, (x, y) in enumerate(sorted(cells("s")), start=1):
        slime = (
            at.Entity(f"Slime {i}")
            .add(at.Transform(position=(x + 0.5, y + 0.5, 0.0)))
            .add(
                at.Sprite(
                    "sprites/slime.png",
                    columns=2,
                    clips=[at.SpriteClip("hop", 0, 1, fps=3)],
                    play="hop",
                    layer=1,
                )
            )
        )
        slime.script = "scripts/slime.py"
        slime.attributes["range"] = SLIME_RANGE
        scene.add(slime)

    (fx, fy), = cells("F")
    scene.add(
        at.Entity("Flag")
        .add(at.Transform(position=(fx + 0.5, fy + 1.0, 0.0)))
        .add(
            at.Sprite(
                "sprites/flag.png",
                size=(1.0, 2.0),
                columns=2,
                clips=[at.SpriteClip("wave", 0, 1, fps=4)],
                play="wave",
                layer=1,
            )
        )
    )

    # Ten cells tall; it trails the player and stays over the level.
    scene.add(
        at.Entity("Camera")
        .add(at.Transform(position=(sx + 0.5, sy + 0.5, 0.0)))
        .add(at.Camera2D(half_height=5.0))
        .add(
            at.CameraFollow2D(
                "Player",
                offset=(0.0, 1.0),
                smoothing=0.12,
                dead_zone=(1.0, 0.75),
                bounds=(0.0, 0.0, float(width), float(height)),
            )
        )
    )
    return scene


def main():
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
