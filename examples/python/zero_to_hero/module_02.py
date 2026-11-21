import automata as at

game = at.Game(title="module 2")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0), fov=60.0)))
    .add(at.Entity("cube").add(at.Transform()).add(at.MeshRenderer.cube()))
)


def update(ctx):
    # Orbit: the camera circles the cube, always looking at it.
    import math

    t = ctx.time.elapsed
    ctx.scene["cam"].camera_3d = at.Camera3D(
        position=(6.0 * math.sin(t), 2.0, 6.0 * math.cos(t)),
        target=(0, 0, 0),
    )


game.run(update)
