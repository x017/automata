import automata as at

game = at.Game(title="module 4")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 4, 8), target=(0, 0, 0))))
    .add(at.Entity("cube").add(at.Transform()).add(at.MeshRenderer.cube()))
)

controls = at.ActionMap()
controls.bind("jump", at.Binding.key(at.Key.SPACE))
controls.bind("jump", at.Binding.gamepad_button(at.GamepadButton.SOUTH))  # stacks with Space


def update(ctx):
    if controls.just_pressed("jump", ctx.input):
        print("jump!")


game.run(update)
