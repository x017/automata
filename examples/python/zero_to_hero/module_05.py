import automata as at

scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 4, 9), target=(0, 1, 0))))
    .add(  # the floor: fixed body + big flat collider
        at.Entity("floor")
        .add(at.Transform(position=(0, 0, 0), scale=(20, 1, 20)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.35, 0.65, 0.30, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(20.0, 1.0, 20.0))
    )
    .add(  # a crate: dynamic body falls, bounces a little, rests
        at.Entity("crate")
        .add(at.Transform(position=(0, 5, 0)))
        .add(at.MeshRenderer.cube(size=0.8, color=(0.72, 0.52, 0.30, 1.0)))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.cuboid(0.8, 0.8, 0.8).restitution(0.3))
    )
)

game = at.Game(title="module 5")
game.load_scene(scene)
game.run(lambda ctx: None)
