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

Subclassing (OOP) style:

    class MyGame(automata.Game):
        title = "Demo"

        def on_start(self):        # once, before the first frame
            self.load_scene(...)

        def on_update(self, ctx):  # every frame
            if ctx.input.just_pressed("escape"):
                ctx.quit()

        def on_hit(self, payload):  # auto-registered "hit" event handler
            ...

    MyGame().run()

    `on_<event>` methods fire once per matching event; events are readable
    on the frame after they're sent.

`Entity` and `Scene` are subclassable too — build reusable archetypes and
levels as classes (see docs/api.md and examples/python/oop_platformer.py).
"""

from typing import Any, Callable, Dict, List, Optional, Tuple, Union, cast

from . import _core
from .enums import GamepadAxis, GamepadButton, GraphicsQuality, Key, MouseButton
from ._core import (
    ActionMap,
    Attributes,
    Audio,
    AxisBinding,
    Audio,
    AudioSource,
    Binding,
    Camera2D,
    Camera3D,
    CharacterController,
    CharacterVelocity,
    Collider2D,
    Collider3D,
    DirectionalLight,
    EntityView,
    Environment,
    Error,
    Events,
    FrameContext,
    FrameTime,
    Gamepad,
    GamepadEvent,
    Gamepads,
    Input,
    MeshRenderer,
    Modifiers,
    ParticleEmitter,
    Physics,
    Physics2D,
    PointLight,
    Prefab,
    Resources,
    RigidBody,
    SceneView,
    Screen,
    SpotLight,
    Transform,
    TransformRef,
    Water,
    __version__,
    asset_root,
    set_asset_root,
)


#: Anything `Entity.add` accepts. Grows as the engine gains components.
Component = Union[
    Transform,
    Camera2D,
    Camera3D,
    DirectionalLight,
    PointLight,
    SpotLight,
    Environment,
    MeshRenderer,
    RigidBody,
    Collider2D,
    Collider3D,
    CharacterController,
    CharacterVelocity,
    AudioSource,
    ParticleEmitter,
    Water,
]

#: A per-frame update callback.
UpdateFn = Callable[[FrameContext], None]

#: An event handler receiving one payload.
EventFn = Callable[[Any], None]


class Entity(_core.Entity):
    """A game object builder, subclassable.

    Attach components with `.add(component)` and place the entity in a
    scene with `Scene.add(entity)`. Subclass to bundle a reusable
    archetype::

        class Player(at.Entity):
            def __init__(self):
                super().__init__("player")
                self.add(at.Transform(position=(0.0, 1.15, 0.0)))
                self.add(at.RigidBody.kinematic_velocity())

    The native constructor runs in `__new__`, so the `name` argument given
    to the subclass constructor reaches it automatically; the `__init__`
    here exists so `super().__init__(name)` chains cleanly.
    """

    def __new__(cls, name: Optional[str] = None, *args: Any, **kwargs: Any) -> "Entity":
        # Subclass constructors may declare their own parameters (e.g.
        # `Crate(position=...)`); those are not for the native constructor.
        # Direct construction stays strict so typos don't pass silently.
        if cls is Entity and (args or kwargs):
            raise TypeError(f"unexpected Entity arguments: {args}{sorted(kwargs)}")
        return cast("Entity", super().__new__(cls, name))

    def __init__(self, name: Optional[str] = None) -> None:
        # Constructed by _core.Entity.__new__; apply the name here too so
        # subclass constructors can pass it through super().__init__(name)
        # even when their own constructor takes no positional arguments.
        if name is not None:
            self.name = name

    def add(self, component: Component) -> "Entity":
        """Attaches a component; returns the entity so calls can chain."""
        return cast("Entity", super().add(component))


class Scene(_core.Scene):
    """A collection of staged entities, subclassable.

    Subclass and build the level in `__init__`::

        class Level(at.Scene):
            def __init__(self):
                super().__init__()
                self.add(Floor())
                self.add(Player())
    """

    def __new__(cls, *args: Any, **kwargs: Any) -> "Scene":
        # See Entity.__new__: subclass constructors may take their own
        # parameters; direct construction takes none.
        if cls is Scene and (args or kwargs):
            raise TypeError("Scene takes no arguments")
        return cast("Scene", super().__new__(cls))

    def __init__(self) -> None:
        pass

    def add(self, entity: Union[Entity, Prefab]) -> "Scene":
        """Stages an entity (or a prefab's entities); returns the scene."""
        return cast("Scene", super().add(entity))

__all__ = [
    "ScriptContext",
    "ScriptEntity",
    "run_scene",
    "ActionMap",
    "Attributes",
    "Audio",
    "AxisBinding",
    "Binding",
    "Camera2D",
    "Audio",
    "AudioSource",
    "Camera3D",
    "CharacterController",
    "CharacterVelocity",
    "Collider2D",
    "Collider3D",
    "DirectionalLight",
    "Entity",
    "Component",
    "EntityView",
    "Environment",
    "Error",
    "Events",
    "FrameContext",
    "FrameTime",
    "Game",
    "GamepadAxis",
    "Gamepad",
    "GamepadButton",
    "GamepadEvent",
    "Gamepads",
    "GraphicsQuality",
    "Input",
    "Key",
    "MeshRenderer",
    "MouseButton",
    "Modifiers",
    "ParticleEmitter",
    "Physics",
    "Physics2D",
    "PointLight",
    "Prefab",
    "Resources",
    "RigidBody",
    "Scene",
    "SceneView",
    "Screen",
    "SpotLight",
    "Transform",
    "TransformRef",
    "Water",
    "__version__",
    "asset_root",
    "set_asset_root",
]


class Game(_core.Game):
    """The application. Three ways to use it, mixing freely:

    **Object-oriented** — subclass and override the hooks. Engine options
    are class attributes; methods named `on_<event>` are auto-registered as
    event handlers::

        class Platformer(Game):
            title = "Platformer"
            headless = True

            def on_start(self):        # once, before the first frame
                self.load_scene(...)

            def on_update(self, ctx):  # every frame
                if ctx.input.just_pressed("escape"):
                    ctx.quit()

            def on_hit(self, payload):  # once per "hit" event
                ...

        Platformer().run()

    **Decorator** — register plain functions on an instance::

        game = Game(title="Demo")

        @game.update
        def tick(ctx): ...

        @game.on_event("hit")
        def handle(payload): ...

        game.run()

    **Procedural** — pass the callback directly: `game.run(tick)`.

    Note: engine options flow through `__new__` (the native constructor),
    not `__init__`.
    """

    # Engine options as overridable class attributes. `None` means "use the
    # engine default"; keyword arguments to the constructor win over the
    # class attributes.
    title: Optional[str] = None
    resolution: Optional[Tuple[int, int]] = None
    headless: Optional[bool] = None
    resizable: Optional[bool] = None
    vsync: Optional[bool] = None
    log_filter: Optional[str] = None
    target_fps: Optional[int] = None
    clear_color: Optional[Tuple[float, float, float, float]] = None
    graphics: Optional[GraphicsQuality] = None
    fixed_delta: Optional[float] = None

    _CONFIG_KEYS = (
        "title",
        "resolution",
        "headless",
        "resizable",
        "vsync",
        "log_filter",
        "target_fps",
        "clear_color",
        "graphics",
        "fixed_delta",
    )

    def __new__(cls, **kwargs: Any) -> "Game":
        config: Dict[str, Any] = {}
        for key in cls._CONFIG_KEYS:
            value = kwargs.pop(key, getattr(cls, key, None))
            if value is not None:
                config[key] = value
        # Subclass constructors may declare their own parameters; those are
        # not engine options. Direct construction stays strict.
        if kwargs and cls is Game:
            unknown = ", ".join(sorted(kwargs))
            raise TypeError(f"unknown Game options: {unknown}")
        return cast("Game", super().__new__(cls, **config))

    def __init__(self, **_options: Any) -> None:
        # Construction happened in __new__; the options arrive here too and
        # are intentionally ignored.
        self._update_fn: Optional[UpdateFn] = None
        self._event_handlers: Dict[str, List[EventFn]] = {}
        self._started = False
        self._frame_scene: Optional[SceneView] = None

    def load_scene(self, scene: Scene) -> None:
        """Replaces the world with the entities of `scene`.

        Before `run` (and in `on_start`) the scene loads at once. While the
        game is running, from `on_update` or an `on_<event>` handler, the
        switch happens when the frame's callback returns, so the next frame
        sees the new scene.
        """
        if self._frame_scene is not None:
            self._frame_scene.load(scene)
        else:
            super().load_scene(scene)

    # -- OOP hooks ------------------------------------------------------

    def on_start(self) -> None:
        """Hook called once before the first frame. Override to set up
        state and load scenes."""

    def on_update(self, ctx: FrameContext) -> None:
        """Hook called every frame. Override it, register a function with
        `@game.update`, or pass one to `run()`."""

    # -- Decorator registration -----------------------------------------

    def update(self, fn: UpdateFn) -> UpdateFn:
        """Decorator registering the per-frame update callback."""
        self._update_fn = fn
        return fn

    def on_event(self, name: str) -> Callable[[EventFn], EventFn]:
        """Decorator registering a handler for events named `name`.

        Handlers receive the event payload and run right after the update
        callback, once per matching event. Events are readable on the frame
        after they're sent, then expire.
        """

        def decorator(fn: EventFn) -> EventFn:
            self._event_handlers.setdefault(name, []).append(fn)
            return fn

        return decorator

    # -- Running ----------------------------------------------------------

    def _resolve_update(self, update: Optional[UpdateFn]) -> UpdateFn:
        fn = update if update is not None else self._update_fn
        if fn is None and type(self).on_update is not Game.on_update:
            fn = self.on_update
        if fn is None:
            raise TypeError(
                "no update callback: pass one to run(), register one with "
                "@game.update, or override on_update in a Game subclass"
            )
        return fn

    def _all_event_handlers(self) -> Dict[str, List[EventFn]]:
        """Decorator-registered handlers plus auto-discovered `on_<event>`
        methods from subclasses."""
        handlers = {name: list(cbs) for name, cbs in self._event_handlers.items()}
        for attr in dir(self):
            if not attr.startswith("on_") or attr in ("on_start", "on_update"):
                continue
            method = getattr(self, attr)
            if callable(method):
                handlers.setdefault(attr[3:], []).append(method)
        return handlers

    def run(self, update: Optional[UpdateFn] = None, max_frames: Optional[int] = None) -> int:
        """Runs the main loop; see `automata._core.Game.run`.

        The callback comes from `update`, `@game.update`, or an overridden
        `on_update` — in that order. `on_start()` runs once before the
        first frame; `on_<event>` methods handle matching events.
        """
        fn = self._resolve_update(update)
        handlers = self._all_event_handlers()
        if not self._started:
            self._started = True
            self.on_start()

        def tick(ctx: FrameContext) -> None:
            self._frame_scene = ctx.scene
            try:
                fn(ctx)
                for name, callbacks in handlers.items():
                    for payload in ctx.events.read(name):
                        for callback in callbacks:
                            callback(payload)
            finally:
                self._frame_scene = None

        return super().run(tick, max_frames)


# Behaviour scripts (Oberon's Inspector → Script) in the real runtime.
from .scripting import ScriptContext, ScriptEntity, run_scene  # noqa: E402
