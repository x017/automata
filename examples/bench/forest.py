#!/usr/bin/env python3
"""Times the renderer on a big world: a forest of identical trees.

    python examples/bench/forest.py                    # 10,000 trees, High
    python examples/bench/forest.py --trees 2000 --quality ultra --frames 30
    python examples/bench/forest.py --no-build --bin target/debug/examples/render_still
    python examples/bench/forest.py --draw-distance 80  # trees vanish past 80 m

Writes ``scenes/forest-N/forest.ron`` (``tree.glb`` from
``examples/models`` on a jittered grid, a sun with shadows, sky and fog,
and a camera standing among the trees at head height), then renders it
with Oberon's ``render_still`` example and prints the median frame time.
Unlike ``run.py`` (scripts and physics, no rendering) this measures only
drawing: culling, instancing and LOD (ROADMAP A11) are what move it.
Frame times include reading the image back, so compare runs on the same
machine and size.
"""

import argparse
import math
import os
import random
import re
import shutil
import subprocess
import sys

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
TREE = os.path.join(ROOT, "examples", "models", "tree.glb")
SPACING = 3.0
# Where the camera stands (the forest's middle) and what it looks at.
EYE = (0.0, 1.7, 0.0)
LOOK = (40.0, 4.0, 25.0)


def build(trees, draw_distance=0.0, seed=7):
    """The forest scene: ``trees`` trees around a clearing at the origin,
    each hidden past ``draw_distance`` metres (0 = never)."""
    rng = random.Random(seed)
    side = math.ceil(math.sqrt(trees))
    extent = side * SPACING
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(direction=(-0.45, -0.8, 0.35), ambient=0.35, shadows=True)
        )
    )
    scene.add(
        at.Entity("Environment").add(
            at.Environment(
                sky_color=(0.7, 0.82, 1.0),
                ground_color=(0.4, 0.45, 0.3),
                background=(0.66, 0.8, 0.95),
                fog=True,
                fog_color=(0.66, 0.8, 0.95),
                fog_density=0.004,
            )
        )
    )
    scene.add(
        at.Entity("Camera")
        .add(at.Transform(position=EYE))
        .add(at.Camera3D(position=EYE, target=LOOK, fov=60.0))
    )
    scene.add(
        at.Entity("Ground")
        .add(at.Transform(position=(0.0, -0.5, 0.0), scale=(extent + 10.0, 1.0, extent + 10.0)))
        .add(at.MeshRenderer.cube(1.0, (0.36, 0.5, 0.25, 1.0)))
    )
    placed = 0
    for i in range(side * side):
        if placed == trees:
            break
        row, col = divmod(i, side)
        x = (col - side / 2 + rng.uniform(-0.35, 0.35)) * SPACING
        z = (row - side / 2 + rng.uniform(-0.35, 0.35)) * SPACING
        if x * x + z * z < 16.0:
            continue  # the clearing the camera stands in
        scale = rng.uniform(0.8, 1.3)
        tree = at.Entity(f"Tree {placed}")
        tree.add(
            at.Transform(
                position=(x, 0.0, z),
                rotation_euler=(0.0, rng.uniform(0.0, math.tau), 0.0),
                scale=(scale, scale, scale),
            )
        )
        tree.add(at.MeshRenderer.model("models/tree.glb", draw_distance=draw_distance))
        scene.add(tree)
        placed += 1
    return scene


def write(trees, out, draw_distance=0.0):
    """Writes the forest project; returns the scene path."""
    name = f"forest-{trees}" + (f"-d{draw_distance:g}" if draw_distance > 0 else "")
    folder = os.path.join(out, name)
    os.makedirs(os.path.join(folder, "models"), exist_ok=True)
    shutil.copyfile(TREE, os.path.join(folder, "models", "tree.glb"))
    path = os.path.join(folder, "forest.ron")
    at.set_asset_root(folder)  # model paths are relative to the scene
    build(trees, draw_distance).save(path)
    return path


def render(binary, scene, quality, size, frames):
    """Renders ``scene`` with ``render_still``; returns (ms per frame, png)."""
    png = os.path.splitext(scene)[0] + f"-{quality}.png"
    env = dict(os.environ, PLAY="0.05", FRAMES=str(frames))
    done = subprocess.run(
        [binary, scene, png, quality, size],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    match = re.search(r"([\d.]+) ms/frame", done.stdout)
    if done.returncode != 0 or match is None:
        sys.exit(f"render_still failed:\n{done.stdout}\n{done.stderr}")
    return float(match.group(1)), png


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--trees", type=int, default=10_000)
    parser.add_argument("--quality", default="high")
    parser.add_argument("--size", default="1280x720")
    parser.add_argument("--frames", type=int, default=20)
    parser.add_argument("--draw-distance", type=float, default=0.0,
                        help="hide trees farther than this many metres (default: never)")
    parser.add_argument("--out", default=os.path.join(HERE, "scenes"))
    parser.add_argument("--bin", default=os.path.join(ROOT, "target", "release", "examples",
                                                      "render_still"))
    parser.add_argument("--no-build", action="store_true", help="don't run cargo first")
    args = parser.parse_args()
    if not args.no_build:
        subprocess.run(
            ["cargo", "build", "--release", "-p", "oberon", "--no-default-features",
             "--example", "render_still"],
            cwd=ROOT,
            check=True,
        )
    scene = write(args.trees, args.out, args.draw_distance)
    ms, png = render(args.bin, scene, args.quality, args.size, args.frames)
    name = os.path.basename(os.path.dirname(scene))
    print(f"{name} {args.quality} {args.size}: {ms:.2f} ms/frame "
          f"({1000.0 / ms:.1f} fps), image {png}")


if __name__ == "__main__":
    main()
