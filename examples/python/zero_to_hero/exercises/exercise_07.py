import automata as at


class MyGame(at.Game):
    title = "module 7: night"
    clear_color = (0.02, 0.03, 0.08, 1.0)      # dark blue night sky
    graphics = at.GraphicsQuality.HIGH          # shadow map resolution


scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 3.5, 9), target=(0, 1, 0))))
    .add(
        at.Entity("sun").add(
            at.DirectionalLight(
                direction=(-0.45, -1.0, -0.3),   # where the light travels
                color=(0.6, 0.7, 1.0),           # pale blue moonlight
                ambient=0.15,                    # deep shadows
                shadows=True,                    # shadow-mapped shadows
            )
        )
    )
    .add(
        at.Entity("floor")
        .add(at.Transform(scale=(30, 1, 8)))
        .add(at.MeshRenderer.cube(color=(0.35, 0.65, 0.30, 1.0)))
    )
    .add(
        at.Entity("tall")
        .add(at.Transform(position=(-1.5, 1.5, 0), scale=(1, 2, 1)))
        .add(at.MeshRenderer.cube(color=(0.95, 0.45, 0.15, 1.0)))
    )
    .add(
        at.Entity("short")
        .add(at.Transform(position=(1.5, 1.0, 0.5)))
        .add(at.MeshRenderer.cube(color=(0.4, 0.7, 0.9, 1.0)))
    )
)

game = MyGame()
game.load_scene(scene)
game.run(lambda ctx: None)
