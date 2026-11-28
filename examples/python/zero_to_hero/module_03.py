import math

import automata as at

game = at.Game(title="module 3")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 3, 7), target=(0, 0.5, 0))))
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0, 0.5, 0), scale=(1.0, 1.0, 1.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.4, 0.7, 0.9, 1.0)))
    )
)


def update(ctx):
    cube = ctx.scene["cube"]
    cube.transform.rotation_euler = (0.0, ctx.time.elapsed * 1.5, 0.0)  # spin
    cube.transform.scale = (1.0, 1.0 + 0.3 * abs(math.sin(ctx.time.elapsed)), 1.0)


game.run(update)
