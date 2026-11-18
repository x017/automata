"""AUTOMATA — a Rust game engine with a Python gameplay interface.

Quick start:

    import automata as at

    game = at.Game(title="Demo", resolution=(1280, 720))

    scene = at.Scene()
    cube = at.Entity("cube")
    cube.add(at.Transform(position=(0, 0, 0)))
    scene.add(cube)
    game.load_scene(scene)

    def update(ctx):
        if ctx.input.just_pressed("escape"):
            ctx.quit()
        cube = ctx.scene.find("cube")
        if cube is not None:
            t = cube.transform
            t_spin = ctx.time.delta
            cube.transform = at.Transform(
                position=t.position,
                rotation_euler=(0, t_spin, 0),
                scale=t.scale,
            )

    game.run(update)

Conventions:

- `ctx` reads observe start-of-frame state; writes (transforms, spawns,
  despawns, events) are buffered and applied right after the callback
  returns, visible from the next frame.
- Engine failures raise `automata.Error`.
- Angles are radians in `Transform.rotation_euler`, degrees in
  `Camera3D(fov=...)`.
"""

from ._core import (
    Camera2D,
    Camera3D,
    Entity,
    EntityView,
    Error,
    Events,
    FrameContext,
    FrameTime,
    Game,
    Input,
    Resources,
    Scene,
    SceneView,
    Transform,
    __version__,
)

__all__ = [
    "Camera2D",
    "Camera3D",
    "Entity",
    "EntityView",
    "Error",
    "Events",
    "FrameContext",
    "FrameTime",
    "Game",
    "Input",
    "Resources",
    "Scene",
    "SceneView",
    "Transform",
    "__version__",
]
