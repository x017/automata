import automata as at

scene = (
    at.Scene()
    .add(at.Entity("camera").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0))))
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0, 0, 0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.95, 0.45, 0.15, 1.0)))
    )
)

game = at.Game(title="module 1")
game.load_scene(scene)
game.run(lambda ctx: None)
