"""AUTOMATA — a Rust game engine with a Python gameplay interface.

Quick start (object-oriented, decorator style):

    import automata as at

    game = at.Game(title="Demo", resolution=(1280, 720))

    scene = (at.Scene()
        .add(at.Entity("cube").add(at.Transform(position=(0, 0, 0))))
        .add(at.Entity("camera").add(at.Camera3D(position=(0, 0.5, 4),
                                                 target=(0, 0, 0)))))
    game.load_scene(scene)

    @game.update
    def tick(ctx):
        if ctx.input.just_pressed("escape"):
            ctx.quit()
        cube = ctx.scene["cube"]            # KeyError if missing
        cube.transform.position = (0, 0, 0)  # buffered; applies after tick

    @game.on_event("hit")
    def handle_hit(payload):
        print("hit!", payload)

    game.run()

The same pieces support procedural style (`game.run(tick)` with a plain
function) and functional style (`game.run(lambda ctx: ctx.quit(),
max_frames=10)`).

Conventions:

- `ctx` reads observe start-of-frame state; writes (transforms, spawns,
  despawns, events) are buffered and applied right after the callback
  returns, visible from the next frame.
- Engine failures raise `automata.Error`; exceptions raised inside
  callbacks propagate unchanged out of `game.run`.
- Angles are radians in `Transform.rotation_euler`, degrees in
  `Camera3D(fov=...)`.
"""

from . import _core
from ._core import (
    Camera2D,
    Camera3D,
    CharacterController,
    CharacterVelocity,
    Collider2D,
    Collider3D,
    Entity,
    EntityView,
    Error,
    Events,
    FrameContext,
    FrameTime,
    Input,
    Modifiers,
    Physics,
    Physics2D,
    Resources,
    RigidBody,
    Scene,
    SceneView,
    Transform,
    TransformRef,
    __version__,
)

__all__ = [
    "Camera2D",
    "Camera3D",
    "CharacterController",
    "CharacterVelocity",
    "Collider2D",
    "Collider3D",
    "Entity",
    "EntityView",
    "Error",
    "Events",
    "FrameContext",
    "FrameTime",
    "Game",
    "Input",
    "Modifiers",
    "Physics",
    "Physics2D",
    "Resources",
    "RigidBody",
    "Scene",
    "SceneView",
    "Transform",
    "TransformRef",
    "__version__",
]


class Game(_core.Game):
    """The application, extended with decorator-based registration.

    `@game.update` registers the per-frame callback; `@game.on_event(name)`
    registers handlers that run after each update for every readable event
    of that name. `run()` with no arguments uses the registered callback.
    """

    def __init__(self, *args, **kwargs):
        super().__init__()
        self._update_fn = None
        self._event_handlers = {}

    def update(self, fn):
        """Decorator registering the per-frame update callback."""
        self._update_fn = fn
        return fn

    def on_event(self, name):
        """Decorator registering a handler for events named `name`.

        Handlers receive the event payload and run right after the update
        callback, once per readable event.
        """

        def decorator(fn):
            self._event_handlers.setdefault(name, []).append(fn)
            return fn

        return decorator

    def run(self, update=None, max_frames=None):
        """Runs the main loop; see `automata._core.Game.run`.

        `update` may be omitted when a callback was registered with
        `@game.update`.
        """
        fn = update if update is not None else self._update_fn
        if fn is None:
            raise TypeError(
                "no update callback: pass one to run() or register one with @game.update"
            )
        handlers = self._event_handlers

        def tick(ctx):
            fn(ctx)
            for name, callbacks in handlers.items():
                for payload in ctx.events.read(name):
                    for callback in callbacks:
                        callback(payload)

        return super().run(tick, max_frames)
