#!/usr/bin/env python3
"""Headless AUTOMATA demo: spins a cube entity for two seconds.

Run after building the bindings:

    maturin build --manifest-path crates/automata-python/Cargo.toml
    pip install target/wheels/automata_engine-*.whl
    python examples/python/headless_demo.py
"""

import automata as at

game = at.Game(title="headless demo", headless=True, target_fps=60)

scene = at.Scene()
cube = at.Entity("cube")
cube.add(at.Transform(position=(0.0, 0.0, 0.0)))
scene.add(cube)
game.load_scene(scene)


def update(ctx):
    cube_view = ctx.scene.find("cube")
    if cube_view is not None:
        _, ry, _ = cube_view.transform.rotation_euler
        cube_view.transform = at.Transform(rotation_euler=(0.0, ry + ctx.time.delta, 0.0))

    if ctx.time.frame % 60 == 0:
        print(f"frame {ctx.time.frame}: cube ry={ry:.2f} rad, entities={ctx.scene.entity_count}")


frames = game.run(update, max_frames=120)
print(f"demo finished after {frames} frames; state={game.state}")
