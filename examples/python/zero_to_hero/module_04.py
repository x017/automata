import automata as at

game = at.Game(title="module 4")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 4, 8), target=(0, 0, 0))))
    .add(at.Entity("cube").add(at.Transform()).add(at.MeshRenderer.cube()))
)

controls = at.ActionMap()
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.ARROW_LEFT, at.Key.ARROW_RIGHT))
controls.bind_axis("move_x", at.AxisBinding.gamepad_axis(at.GamepadAxis.LEFT_STICK_X))
controls.bind("quit", at.Binding.key(at.Key.ESCAPE))

x = 0.0


def update(ctx):
    global x
    dx = controls.axis("move_x", ctx.input)  # -1.0 ..= 1.0
    x += dx * 4.0 * ctx.time.delta
    ctx.scene["cube"].transform.position = (x, 0.0, 0.0)
    if controls.just_pressed("quit", ctx.input):
        ctx.quit()


game.run(update)
