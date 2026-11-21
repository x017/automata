import automata as at

# (height, restitution): higher numbers bounce more; 0.0 lands dead.
DROPS = [(3.0, 0.0), (4.0, 0.2), (5.0, 0.4), (6.0, 0.6), (7.0, 0.8)]

scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 4, 11), target=(0, 1.5, 0))))
    .add(
        at.Entity("floor")
        .add(at.Transform(position=(0, 0, 0), scale=(20, 1, 20)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.35, 0.65, 0.30, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(20.0, 1.0, 20.0))
    )
)
for i, (height, bounce) in enumerate(DROPS):
    scene.add(
        at.Entity(f"crate_{i}")
        .add(at.Transform(position=(-4.0 + 2.0 * i, height, 0)))
        .add(at.MeshRenderer.cube(size=0.8, color=(0.72, 0.52, 0.30, 1.0)))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.cuboid(0.8, 0.8, 0.8).restitution(bounce))
    )

game = at.Game(title="module 5")
game.load_scene(scene)
game.run(lambda ctx: None)
