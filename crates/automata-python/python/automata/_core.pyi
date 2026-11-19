"""Type stubs for the native automata._core extension module."""

from typing import Any, Callable, Optional

__version__: str

Vec3 = tuple[float, float, float]
Vec2 = tuple[float, float]

class Error(Exception):
    """An error raised by the AUTOMATA engine runtime."""

class Transform:
    """Position, rotation, and scale of an entity, relative to its parent."""

    def __init__(
        self,
        position: Vec3 = (0.0, 0.0, 0.0),
        rotation_euler: Vec3 = (0.0, 0.0, 0.0),
        scale: Vec3 = (1.0, 1.0, 1.0),
    ) -> None: ...
    @property
    def position(self) -> Vec3: ...
    @property
    def rotation_euler(self) -> Vec3: ...
    @property
    def scale(self) -> Vec3: ...

class Camera3D:
    """A perspective 3D camera."""

    def __init__(
        self,
        position: Vec3 = (0.0, 0.0, 0.0),
        target: Vec3 = (0.0, 0.0, -1.0),
        fov: float = 60.0,
    ) -> None: ...
    @property
    def position(self) -> Vec3: ...
    @property
    def target(self) -> Vec3: ...
    @property
    def fov(self) -> float: ...

class Camera2D:
    """An orthographic 2D camera."""

    def __init__(
        self, center: Vec2 = (0.0, 0.0), half_height: float = 100.0
    ) -> None: ...
    @property
    def center(self) -> Vec2: ...
    @property
    def half_height(self) -> float: ...

class Entity:
    """A game object builder: attach components, then add it to a Scene."""

    def __init__(self, name: Optional[str] = None) -> None: ...
    @property
    def name(self) -> Optional[str]: ...
    def add(self, component: object) -> None: ...

class Scene:
    """A collection of entities, ready to load into a game."""

    def __init__(self) -> None: ...
    def add(self, entity: Entity) -> None: ...
    @property
    def entity_count(self) -> int: ...
    def __len__(self) -> int: ...

class FrameTime:
    """Frame timing information."""

    delta: float
    elapsed: float
    frame: int

class Modifiers:
    """Modifier key state at the start of the frame."""

    shift: bool
    ctrl: bool
    alt: bool
    super_key: bool

class Input:
    """Keyboard and mouse state at the start of the frame."""

    cursor: Optional[tuple[float, float]]
    scroll: tuple[float, float]
    modifiers: Modifiers
    def pressed(self, key: str) -> bool: ...
    def just_pressed(self, key: str) -> bool: ...
    def just_released(self, key: str) -> bool: ...
    def mouse_pressed(self, button: str) -> bool: ...
    def mouse_just_pressed(self, button: str) -> bool: ...
    def mouse_just_released(self, button: str) -> bool: ...

class EntityView:
    """A handle to one entity inside a SceneView."""

    @property
    def id(self) -> int: ...
    @property
    def name(self) -> Optional[str]: ...
    @property
    def transform(self) -> Optional[Transform]: ...
    @transform.setter
    def transform(self, value: Transform) -> None: ...
    def despawn(self) -> None: ...

class SceneView:
    """Read access to the scene during the update callback."""

    @property
    def entity_count(self) -> int: ...
    def find(self, name: str) -> Optional[EntityView]: ...
    def names(self) -> list[str]: ...
    def spawn(self, entity: Entity) -> None: ...
    def despawn(self, id: int) -> None: ...

class Events:
    """Named events with arbitrary payloads, readable for two frames."""

    def send(self, name: str, payload: Any = None) -> None: ...
    def read(self, name: str) -> list[Any]: ...

class Resources:
    """Named, game-owned storage for arbitrary Python objects."""

    def set(self, name: str, value: Any) -> None: ...
    def get(self, name: str) -> Optional[Any]: ...
    def remove(self, name: str) -> Optional[Any]: ...
    def clear(self) -> None: ...
    def names(self) -> list[str]: ...
    def __contains__(self, name: str) -> bool: ...
    def __len__(self) -> int: ...

class FrameContext:
    """The per-frame update context passed to the update callback."""

    time: FrameTime
    input: Input
    scene: SceneView
    events: Events
    resources: Resources
    def quit(self) -> None: ...

class Game:
    """The application: owns the engine, the loaded scene, and the main loop."""

    def __init__(
        self,
        title: str = "AUTOMATA",
        resolution: tuple[int, int] = (1280, 720),
        headless: bool = False,
        resizable: bool = True,
        vsync: bool = True,
        log_filter: Optional[str] = None,
        target_fps: Optional[int] = None,
    ) -> None: ...
    @property
    def state(self) -> str: ...
    def load_scene(self, scene: Scene) -> None: ...
    def run(
        self,
        update: Callable[[FrameContext], None],
        max_frames: Optional[int] = None,
    ) -> int: ...
    def quit(self) -> None: ...
