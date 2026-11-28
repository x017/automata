import automata as at

WALK, GRAVITY, JUMP = 5.0, 20.0, 9.5

scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 3.5, 10), target=(0, 1, 0))))
    .add(
        at.Entity("floor")
        .add(at.Transform(scale=(30, 1, 8)))
        .add(at.MeshRenderer.cube(color=(0.35, 0.65, 0.30, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(30, 1, 8))
    )
    .add(
        at.Entity("player")
        .add(at.Transform(position=(-4, 1.4, 0)))
        .add(at.MeshRenderer.cube(size=0.9, color=(0.95, 0.45, 0.15, 1.0)))
        .add(at.RigidBody.kinematic_velocity())
        .add(at.Collider3D.capsule(0.5, 0.4))
        .add(at.CharacterController(snap_to_ground=0.5))
        .add(at.CharacterVelocity())
    )
)

controls = at.ActionMap()
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
controls.bind("jump", at.Binding.key(at.Key.SPACE))

vy = 0.0


def update(ctx):
    global vy
    player = ctx.scene["player"]
    dx = controls.axis("move_x", ctx.input)

    if ctx.physics.grounded(player):
        vy = -1.0  # keep contact so snap-to-ground works on slopes
        if controls.just_pressed("jump", ctx.input):
            vy = JUMP
    else:
        vy -= GRAVITY * ctx.time.delta

    player.character_velocity = (dx * WALK, vy, 0.0)


game = at.Game(title="module 6")
game.load_scene(scene)
game.run(update)
