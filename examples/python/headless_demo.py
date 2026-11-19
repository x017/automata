#!/usr/bin/env python3
"""Headless AUTOMATA demo: spins a cube entity for two seconds.

Shows the decorator/OO style; the same pieces work procedurally
(`game.run(tick)`) or functionally (`game.run(lambda ctx: ...)`).

Run after building the bindings:

    maturin build --manifest-path crates/automata-python/Cargo.toml
    pip install target/wheels/automata_engine-*.whl
    python examples/python/headless_demo.py
"""

import automata as at

game = at.Game(title="headless demo", headless=True, target_fps=60)

game.load_scene(
    at.Scene().add(
        at.Entity("cube").add(at.Transform(position=(0.0, 0.0, 0.0)))
    )
)


@game.update
def tick(ctx):
    cube = ctx.scene["cube"]  # KeyError if missing; ctx.scene.find is the soft variant
    _, ry, _ = cube.transform.rotation_euler
    cube.transform.rotation_euler = (0.0, ry + ctx.time.delta, 0.0)

    if ctx.time.frame % 60 == 0:
        print(f"frame {ctx.time.frame}: cube ry={ry:.2f} rad, entities={ctx.scene.entity_count}")


frames = game.run(max_frames=120)
print(f"demo finished after {frames} frames; state={game.state}")
