#!/usr/bin/env python3
"""Writes the benchmark scenes: one small project per scene.

    python examples/bench/build_scenes.py [--sizes 100,1000,5000] [--out DIR]

Two kinds of scene, each at every size:

* ``scripts-N``: N scripted cubes that move by writing their transform.
* ``physics-N``: N scripted character controllers (capsules) that walk on
  the ground, so the physics step and the scripts both scale with N.

Every entity runs ``bench.py`` and casts a ray at a ground collider each
frame. Each project folder holds the scene (``bench.ron``), the script and
a ``build.ron``, so it opens in Oberon, runs with ``automata.run_scene`` and
exports with ``automata-export build``. ``run.py`` calls this first.
"""

import argparse
import math
import os
import shutil

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
SIZES = (100, 1000, 5000)
KINDS = ("scripts", "physics")
SPACING = 2.0

BUILD_RON = """// Export settings for the benchmark scene (examples/bench/run.py).
BuildConfig(
    name: "Bench {name}",
    version: "0.1.0",
    start_scene: Some("bench.ron"),
    main_script: None,
    resolution: (1280, 720),
    targets: [Linux, Windows],
    output: "build",
    archive: false,
    exclude: [],
)
"""


def build(kind, count):
    """The scene for ``kind`` ("scripts" or "physics") with ``count`` entities."""
    scene = at.Scene()
    side = math.ceil(math.sqrt(count))
    extent = side * SPACING
    scene.add(
        at.Entity("Sun").add(at.DirectionalLight(direction=(-0.5, -0.8, 0.3), ambient=0.4))
    )
    scene.add(
        at.Entity("Camera").add(
            at.Camera3D(
                position=(0.0, extent * 0.6, extent * 0.9),
                target=(0.0, 0.0, 0.0),
                fov=60.0,
            )
        )
    )
    scene.add(
        at.Entity("Ground")
        .add(at.Transform(position=(0.0, -0.5, 0.0)))
        .add(at.Collider3D.cuboid(extent + 8.0, 1.0, extent + 8.0))
    )
    for i in range(count):
        row, col = divmod(i, side)
        x = (col - side / 2) * SPACING
        z = (row - side / 2) * SPACING
        entity = at.Entity(f"Unit {i}")
        if kind == "physics":
            entity.add(at.Transform(position=(x, 0.6, z)))
            entity.add(at.MeshRenderer.cube(0.6, (0.35, 0.75, 1.0, 1.0)))
            entity.add(at.RigidBody.kinematic_position())
            entity.add(at.Collider3D.capsule(0.3, 0.3))
            entity.add(at.CharacterController(snap_to_ground=0.2))
            entity.add(at.CharacterVelocity())
            entity.attributes["walker"] = True
        else:
            entity.add(at.Transform(position=(x, 0.5, z)))
            entity.add(at.MeshRenderer.cube(0.6, (1.0, 0.75, 0.35, 1.0)))
        entity.attributes["speed"] = 1.0 + (i % 7) * 0.25
        entity.attributes["phase"] = (i % 13) * 0.5
        entity.attributes["height"] = 0.0
        entity.script = "bench.py"
        scene.add(entity)
    return scene


def write(out, kinds=KINDS, sizes=SIZES):
    """Writes every scene under ``out``; returns ``[(kind, size, folder)]``."""
    written = []
    for kind in kinds:
        for size in sizes:
            name = f"{kind}-{size}"
            folder = os.path.join(out, name)
            os.makedirs(folder, exist_ok=True)
            build(kind, size).save(os.path.join(folder, "bench.ron"))
            shutil.copyfile(os.path.join(HERE, "bench.py"), os.path.join(folder, "bench.py"))
            with open(os.path.join(folder, "build.ron"), "w", encoding="utf-8") as f:
                f.write(BUILD_RON.format(name=name))
            written.append((kind, size, folder))
    return written


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--sizes", default=",".join(map(str, SIZES)))
    parser.add_argument("--out", default=os.path.join(HERE, "scenes"))
    args = parser.parse_args()
    sizes = [int(s) for s in args.sizes.split(",") if s]
    for kind, size, folder in write(args.out, sizes=sizes):
        print(f"wrote {kind}-{size}: {folder}")


if __name__ == "__main__":
    main()
