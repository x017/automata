#!/usr/bin/env python3
"""Writes the Joint Yard scene: a swinging door, a rope bridge and a chain.

    python examples/oberon/joint_yard/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/joint_yard/level.ron

Everything that moves here is held by a Joint component:

- **The door** turns on a Hinge joint to its frame, limited to ±97°. Its
  motor pulls it to a target angle; ``scripts/door.py`` swaps the target
  every three seconds (or when you press E), so it swings open and shut.
- **The rope bridge** is ten planks, each hinged to the one before it, the
  first to the left post. The right post carries the last joint (an entity
  holds one joint, so the post holds the end plank). A crate drops on it.
- **The chain** is eight links on Ball joints hanging from a beam, with a
  heavy wrecking ball on the end. It starts out sideways, swings down and
  smashes a sign welded to its post by a Fixed joint with a break force:
  the sign snaps off, and ``scripts/sign.py`` hears about it.
- **The lantern** bobs on a Spring joint under the beam.
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

GRASS = (0.25, 0.36, 0.18, 1.0)
WOOD = (0.45, 0.3, 0.17, 1.0)
DARK_WOOD = (0.28, 0.18, 0.1, 1.0)
PLANK = (0.56, 0.4, 0.24, 1.0)
IRON = (0.32, 0.33, 0.36, 1.0)
PAINT = (0.85, 0.2, 0.15, 1.0)
CRATE = (0.62, 0.47, 0.25, 1.0)
GLOW = (1.0, 0.75, 0.35, 1.0)

#: The bridge spans x = -3 .. 3 between two posts 2.5 m tall, sagging 0.5 m.
BRIDGE_ENDS = (-3.0, 3.0)
BRIDGE_TOP = 2.5
BRIDGE_SAG = 0.5
PLANKS = 10

#: The chain hangs from the beam at x = 8; links are 0.4 m.
CHAIN_TOP = (8.0, 5.85, 0.0)
LINKS = 8
LINK = 0.4
BALL_RADIUS = 0.35


def r4(v):
    """Rounded, so the saved level is the same on every platform's maths
    library."""
    return round(v, 4)


def solid(name, position, size, color, rotation=(0.0, 0.0, 0.0), dynamic=False,
          density=1.0, **surface):
    """A box you can see and bump into (static unless ``dynamic``)."""
    e = (
        at.Entity(name)
        .add(at.Transform(position=position, rotation_euler=rotation, scale=size))
        .add(at.MeshRenderer.cube(size=1.0, color=color, **surface))
        .add(at.Collider3D.cuboid(*size).density(density))
    )
    if dynamic:
        e.add(at.RigidBody.dynamic())
    return e


def scenery(scene):
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(direction=(-0.5, -1.0, -0.35), color=(1.0, 0.95, 0.88),
                                ambient=0.3)
        )
    )
    scene.add(at.Entity("Sky").add(at.Environment.preset("daylight")))
    scene.add(solid("Ground", (0.0, -0.5, 0.0), (40.0, 1.0, 16.0), GRASS, roughness=0.95))


def door(scene):
    # A frame of two posts and a lintel; the left post carries the hinge.
    scene.add(solid("Door Frame", (-8.55, 1.15, 0.0), (0.15, 2.3, 0.2), DARK_WOOD))
    scene.add(solid("Door Post", (-7.25, 1.15, 0.0), (0.15, 2.3, 0.2), DARK_WOOD))
    scene.add(solid("Lintel", (-7.9, 2.375, 0.0), (1.45, 0.15, 0.2), DARK_WOOD))
    d = solid("Door", (-7.94, 1.05, 0.0), (1.08, 2.0, 0.06), WOOD, dynamic=True,
              roughness=0.8)
    d.add(
        at.Joint.hinge(
            axis=(0, 1, 0),
            connected="Door Frame",
            anchor=(-0.54, 0.0, 0.0),
            limits=(-1.7, 1.7),
            motor=at.JointMotor(target=0.0),
        )
    )
    d.script = "scripts/door.py"
    scene.add(d)


def bridge_points():
    """Where the planks' edges go: a parabola from post to post."""
    x0, x1 = BRIDGE_ENDS
    pts = []
    for i in range(PLANKS + 1):
        t = i / PLANKS
        x = x0 + (x1 - x0) * t
        y = BRIDGE_TOP - BRIDGE_SAG * 4.0 * t * (1.0 - t)
        pts.append((x, y))
    return pts


def bridge(scene):
    x0, x1 = BRIDGE_ENDS
    scene.add(solid("Bridge Post L", (x0 - 0.2, BRIDGE_TOP / 2, 0.0), (0.4, BRIDGE_TOP, 1.4),
                    DARK_WOOD))
    pts = bridge_points()
    for i in range(PLANKS):
        (ax, ay), (bx, by) = pts[i], pts[i + 1]
        length = math.hypot(bx - ax, by - ay)
        angle = math.atan2(by - ay, bx - ax)
        plank = solid(
            f"Plank {i + 1}",
            (r4((ax + bx) / 2), r4((ay + by) / 2), 0.0),
            (r4(length - 0.02), 0.08, 1.2),
            PLANK,
            rotation=(0.0, 0.0, r4(angle)),
            dynamic=True,
            density=60.0,
            roughness=0.85,
        )
        previous = "Bridge Post L" if i == 0 else f"Plank {i}"
        plank.add(
            at.Joint.hinge(axis=(0, 0, 1), connected=previous, anchor=(r4(-length / 2), 0.0, 0.0))
        )
        scene.add(plank)
    # The right post holds the last plank's far edge.
    right = solid("Bridge Post R", (x1 + 0.2, BRIDGE_TOP / 2, 0.0), (0.4, BRIDGE_TOP, 1.4),
                  DARK_WOOD)
    right.add(
        at.Joint.hinge(axis=(0, 0, 1), connected=f"Plank {PLANKS}",
                       anchor=(-0.2, BRIDGE_TOP / 2, 0.0))
    )
    scene.add(right)
    scene.add(solid("Crate", (0.4, 4.6, 0.1), (0.5, 0.5, 0.5), CRATE, dynamic=True,
                    density=80.0, rotation=(0.0, 0.4, 0.0)))


def chain(scene):
    cx, cy, cz = CHAIN_TOP
    # The post stands behind the beam, out of the chain's way.
    scene.add(solid("Gallows Post", (9.5, 3.0, -0.8), (0.3, 6.0, 0.3), DARK_WOOD))
    scene.add(solid("Brace", (9.5, 6.0, -0.4), (0.3, 0.3, 0.5), DARK_WOOD))
    scene.add(solid("Beam", (8.0, 6.0, 0.0), (3.4, 0.3, 0.3), DARK_WOOD))
    # Laid out sideways (to the left), so it swings down when the game starts.
    for i in range(LINKS):
        link = solid(
            f"Link {i + 1}",
            (r4(cx - LINK * (i + 0.5)), cy, cz),
            (LINK - 0.04, 0.1, 0.1),
            IRON,
            dynamic=True,
            density=300.0,
            metallic=1.0,
            roughness=0.4,
        )
        if i == 0:
            link.add(at.Joint.ball(connected="Beam", anchor=(LINK / 2, 0.0, 0.0)))
        else:
            link.add(at.Joint.ball(connected=f"Link {i}", anchor=(LINK / 2, 0.0, 0.0)))
        scene.add(link)
    ball_x = cx - LINK * LINKS - BALL_RADIUS
    scene.add(
        at.Entity("Wrecking Ball")
        .add(at.Transform(position=(r4(ball_x), cy, cz)))
        .add(at.MeshRenderer.sphere(size=BALL_RADIUS * 2, color=IRON, metallic=1.0,
                                    roughness=0.35))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.ball(BALL_RADIUS).density(40.0))
        .add(at.Joint.ball(connected=f"Link {LINKS}", anchor=(BALL_RADIUS, 0.0, 0.0)))
    )
    # The sign it smashes: welded to a post, until it's hit hard enough.
    scene.add(solid("Sign Post", (9.7, 1.4, 0.0), (0.15, 2.8, 0.15), DARK_WOOD))
    sign = solid("Sign", (9.25, 2.45, 0.0), (0.75, 0.55, 0.06), PAINT, dynamic=True,
                 density=20.0)
    sign.add(at.Joint.fixed(connected="Sign Post", break_force=120.0))
    sign.script = "scripts/sign.py"
    scene.add(sign)


def lantern(scene):
    lamp = (
        at.Entity("Lantern")
        .add(at.Transform(position=(6.9, 3.6, 0.9)))
        .add(at.MeshRenderer.cube(size=0.3, color=GLOW, emissive=(1.0, 0.7, 0.3),
                                  emissive_strength=3.0))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.cuboid(0.3, 0.3, 0.3).density(5.0))
        .add(
            at.Joint.spring(1.2, 30.0, 1.0, connected="Beam", anchor=(0.0, 0.15, 0.0),
                            connected_anchor=(-1.1, -0.15, 0.9))
        )
    )
    scene.add(lamp)
    scene.add(
        at.Entity("Lantern Light")
        .add(at.Transform(position=(6.9, 3.2, 1.2)))
        .add(at.PointLight(color=(1.0, 0.7, 0.35), intensity=1.5, range=5.0))
    )


def build():
    scene = at.Scene()
    scenery(scene)
    door(scene)
    bridge(scene)
    chain(scene)
    lantern(scene)
    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(0.5, 3.8, 12.5)))
        .add(at.Camera3D(position=(0.5, 3.8, 12.5), target=(0.5, 2.3, 0.0)))
    )
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
