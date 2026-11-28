import math

import automata as at

game = at.Game(title="module 2")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0), fov=60.0)))
    .add(at.Entity("cube").add(at.Transform()).add(at.MeshRenderer.cube()))
)


def update(ctx):
    # Bob: the camera rises and falls in front of the cube.
    t = ctx.time.elapsed
    ctx.scene["cam"].camera_3d = at.Camera3D(position=(0.0, 2.0 + math.sin(t), 6.0), target=(0, 0, 0))


game.run(update)
