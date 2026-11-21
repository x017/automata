"""Run an Oberon scene with its behaviour scripts attached.

Oberon lets you attach a Python script to any entity (Inspector → Script).
In the editor's Play mode those scripts run in a bridged Python process;
`run_scene` runs the very same files inside the real engine:

    import automata as at
    at.run_scene("level.ron")

A behaviour script is a module with optional `start(entity, ctx)`,
`update(entity, ctx)` and `stop(entity, ctx)` functions. `entity` offers
`name`, `transform`, `character_velocity`, `attributes`, `grounded`,
`camera`, `audio` and a per-entity `state` dict; `ctx` offers `input`,
`time`, `physics`, `scene`, `camera`, `audio`, `hud`, `events`,
`resources`, `log(...)` and `quit()`.
"""

import importlib.util
import math
import os
import sys
from typing import Any, Callable, Dict, List, Optional, Tuple

from . import _core


class ScriptCamera:
    """An entity's perspective camera (``entity.camera`` / ``ctx.camera``).

    The same API Oberon's Play mode offers: read or assign ``position``,
    ``target`` and ``fov``, or call ``look_at`` / ``follow`` every frame.
    Writes apply after the frame's callbacks, like every other write.
    """

    def __init__(self, view: Any, state: List[Any], delta: float) -> None:
        self._view = view
        self._state = state  # [x, y, z, tx, ty, tz, fov], shared per frame
        self._delta = delta

    def _set(self, start: int, values: Tuple[float, ...]) -> None:
        self._state[start : start + len(values)] = [float(v) for v in values]
        st = self._state
        self._view.camera_3d = _core.Camera3D(
            position=(st[0], st[1], st[2]), target=(st[3], st[4], st[5]), fov=st[6]
        )

    @property
    def position(self) -> Tuple[float, float, float]:
        """The eye position (x, y, z)."""
        return (self._state[0], self._state[1], self._state[2])

    @position.setter
    def position(self, value: Any) -> None:
        self._set(0, tuple(float(v) for v in value)[:3])

    @property
    def target(self) -> Tuple[float, float, float]:
        """The point the camera looks at (x, y, z)."""
        return (self._state[3], self._state[4], self._state[5])

    @target.setter
    def target(self, value: Any) -> None:
        self._set(3, tuple(float(v) for v in value)[:3])

    @property
    def fov(self) -> float:
        """Vertical field of view in degrees."""
        return self._state[6]

    @fov.setter
    def fov(self, degrees: float) -> None:
        self._set(6, (min(179.0, max(1.0, float(degrees))),))

    def look_at(self, point: Any) -> None:
        """Aims the camera at a point or an entity."""
        if hasattr(point, "transform"):
            point = point.transform.position
        self.target = point

    def follow(
        self,
        entity: Any,
        offset: Any = (0.0, 3.0, 8.0),
        smooth: float = 8.0,
        look_height: float = 0.6,
    ) -> None:
        """Moves toward ``entity + offset`` and looks at it (0 smooth = snap)."""
        p = entity.transform.position
        goal = [p[i] + float(offset[i]) for i in range(3)]
        k = 1.0 - math.exp(-float(smooth) * max(self._delta, 0.0)) if smooth > 0 else 1.0
        cur = self.position
        self._state[0:3] = [cur[i] + (goal[i] - cur[i]) * k for i in range(3)]
        self.target = (p[0], p[1] + float(look_height), p[2])

    def __repr__(self) -> str:
        return f"Camera(position={self.position}, target={self.target}, fov={self.fov})"


class _Cameras:
    """Per-frame camera state, so several writes in one frame accumulate."""

    def __init__(self) -> None:
        self.delta = 0.0
        self.states: Dict[int, List[Any]] = {}

    def begin(self, delta: float) -> None:
        self.delta = float(delta)
        self.states = {}

    def of(self, view: Any) -> Optional[ScriptCamera]:
        if view is None:
            return None
        cam = view.camera_3d
        if cam is None:
            return None
        key = int(view.id)
        state = self.states.get(key)
        if state is None:
            state = self.states[key] = [*cam.position, *cam.target, cam.fov]
        return ScriptCamera(view, state, self.delta)


_CAMERAS = _Cameras()


class ScriptEntity:
    """The entity handle a behaviour script receives.

    Wraps the frame's `EntityView` and keeps a `state` dict that survives
    across frames (EntityViews are rebuilt every frame).
    """

    def __init__(self, name: str, script: Optional[str] = None) -> None:
        self.name = name
        self.script = script
        self.state: Dict[str, Any] = {}
        self.grounded = False
        self._view: Any = None

    @property
    def id(self) -> int:
        return int(self._view.id)

    @property
    def view(self) -> Any:
        """The underlying `EntityView` for this frame."""
        return self._view

    @property
    def transform(self) -> Any:
        return self._view.transform

    @transform.setter
    def transform(self, value: Any) -> None:
        self._view.transform = value

    @property
    def character_velocity(self) -> Any:
        return self._view.character_velocity

    @character_velocity.setter
    def character_velocity(self, value: Any) -> None:
        self._view.character_velocity = value

    @property
    def camera_3d(self) -> Any:
        return self._view.camera_3d

    @camera_3d.setter
    def camera_3d(self, value: Any) -> None:
        self._view.camera_3d = value

    @property
    def camera(self) -> Optional[ScriptCamera]:
        """This entity's camera, or None if it has no Camera3D."""
        return _CAMERAS.of(self._view)

    @property
    def attributes(self) -> Any:
        return self._view.attributes

    @property
    def audio(self) -> "ScriptAudio":
        """This entity's Audio Source: ``play()``, ``stop()``, ``playing``."""
        return ScriptAudio(self)

    def despawn(self) -> None:
        self._view.despawn()

    def __repr__(self) -> str:
        return f"ScriptEntity({self.name!r})"


class ScriptAudio:
    """``entity.audio``: plays the entity's Audio Source component."""

    def __init__(self, entity: "ScriptEntity") -> None:
        self._e = entity

    def _raw(self) -> Any:
        return _FRAME["ctx"].audio

    @property
    def source(self) -> Any:
        """The ``AudioSource`` component, or None."""
        return self._e._view.audio_source

    def play(self) -> None:
        """Starts the sound (restarts it if it's already playing)."""
        self._raw().play_source(self._e._view)

    def stop(self) -> None:
        self._raw().stop_source(self._e._view)

    @property
    def playing(self) -> bool:
        """Whether it was playing at the start of the frame."""
        return bool(self._raw().source_playing(self._e._view))

    def __repr__(self) -> str:
        return f"Audio({self._e.name!r})"


# The raw frame context of the frame being run (for ScriptAudio).
_FRAME: Dict[str, Any] = {}


def _unwrap(entity: Any) -> Any:
    return entity._view if isinstance(entity, ScriptEntity) else entity


class Hit:
    """A raycast hit: ``entity`` (or None), ``point``, ``normal``, ``distance``."""

    __slots__ = ("entity", "point", "normal", "distance")

    def __init__(self, entity: Any, point: Any, normal: Any, distance: float) -> None:
        self.entity = entity
        self.point = tuple(point)
        self.normal = tuple(normal)
        self.distance = float(distance)

    def __repr__(self) -> str:
        name = getattr(self.entity, "name", None)
        return f"Hit({name!r}, distance={self.distance:.2f})"


class _Physics:
    def __init__(self, physics: Any, scene: "_Scene") -> None:
        self._p = physics
        self._scene = scene

    def grounded(self, entity: Any) -> bool:
        if isinstance(entity, ScriptEntity):
            return entity.grounded
        return bool(self._p.grounded(entity))

    def raycast(
        self,
        origin: Any,
        direction: Any,
        max_distance: float = 100.0,
        ignore: Any = None,
    ) -> Optional[Hit]:
        """The first collider along a ray, right now, or None.

        ``ignore`` skips one entity (usually the caster). Sensors are skipped.
        """
        o = tuple(float(v) for v in origin)[:3]
        d = tuple(float(v) for v in direction)[:3]
        exclude = _unwrap(ignore) if ignore is not None else None
        hit = self._p.raycast(o, d, float(max_distance), exclude)
        if hit is None:
            return None
        bits, point, normal, distance = hit
        return Hit(self._scene.by_id(bits), point, normal, distance)

    def __getattr__(self, name: str) -> Any:
        attr = getattr(self._p, name)
        if not callable(attr):
            return attr

        def call(*args: Any, **kwargs: Any) -> Any:
            return attr(*(_unwrap(a) for a in args), **kwargs)

        return call


class _Scene:
    def __init__(self, scene: Any, handles: Dict[str, ScriptEntity]) -> None:
        self._s = scene
        self._h = handles

    def by_id(self, bits: int) -> Any:
        """The entity with this id, as a script handle (None if gone)."""
        for handle in self._h.values():
            if handle._view is not None and int(handle._view.id) == bits:
                return handle
        for view in self._s:
            if int(view.id) == bits:
                temp = ScriptEntity(view.name or "")
                temp._view = view
                return temp
        return None

    def find(self, name: str) -> Any:
        handle = self._h.get(name)
        if handle is not None and handle._view is not None:
            return handle
        return self._s.find(name)

    def __getitem__(self, name: str) -> Any:
        found = self.find(name)
        if found is None:
            raise KeyError(f"no entity named {name!r}")
        return found

    def __iter__(self) -> Any:
        return iter(self._s)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._s, name)


_KEY_ALIASES = {
    "shift": ("shift_left", "shift_right"),
    "ctrl": ("ctrl_left", "ctrl_right"),
    "alt": ("alt_left", "alt_right"),
    "left": ("arrow_left",),
    "right": ("arrow_right",),
    "up": ("arrow_up",),
    "down": ("arrow_down",),
}


def _keys(key: Any) -> Tuple[Any, ...]:
    if isinstance(key, str):
        return _KEY_ALIASES.get(key.lower(), (key.lower(),))
    return (key,)


class _Input:
    """`ctx.input` for scripts: Oberon's key aliases (``"shift"`` means
    either Shift key, ``"left"`` the left arrow) plus ``axis``."""

    def __init__(self, raw: Any) -> None:
        self._raw = raw

    def pressed(self, key: Any) -> bool:
        return any(self._raw.pressed(k) for k in _keys(key))

    def just_pressed(self, key: Any) -> bool:
        return any(self._raw.just_pressed(k) for k in _keys(key))

    def just_released(self, key: Any) -> bool:
        return any(self._raw.just_released(k) for k in _keys(key))

    def axis(self, negative: Any, positive: Any) -> float:
        """-1, 0, or 1 from two keys."""
        return float(self.pressed(positive)) - float(self.pressed(negative))

    @property
    def mouse_delta(self) -> Tuple[float, float]:
        """Mouse movement since last frame (dx, dy); +y is down."""
        dx, dy = self._raw.mouse_delta
        return (float(dx), float(dy))

    def lock_mouse(self, locked: bool = True) -> None:
        """Hides and captures the cursor for mouse-look (False releases it)."""
        _FRAME["ctx"].screen.lock_cursor(bool(locked))

    @property
    def mouse_locked(self) -> bool:
        return bool(_FRAME["ctx"].screen.cursor_locked)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._raw, name)


class _Audio:
    """``ctx.audio`` for scripts: one-shot sounds by file name."""

    def __init__(self, raw: Any) -> None:
        self._raw = raw

    def play(
        self,
        path: str,
        volume: float = 1.0,
        pitch: float = 1.0,
        position: Any = None,
    ) -> None:
        """Plays a sound file once (relative to the scene folder). With a
        ``position`` (a point or an entity) it fades with distance."""
        if hasattr(position, "transform"):
            position = position.transform.position
        pos = tuple(float(v) for v in position)[:3] if position is not None else None
        self._raw.play_at(str(path), float(volume), float(pitch), pos)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._raw, name)


class _Hud:
    """``ctx.hud``: draw images and rectangles over the game this frame.

    Coordinates are screen pixels from the top-left; shapes are placed by
    their centre. Call every frame you want them shown.
    """

    def __init__(self, screen: Any) -> None:
        self._s = screen

    @property
    def size(self) -> Tuple[int, int]:
        w, h = self._s.size
        return (int(w), int(h))

    @property
    def center(self) -> Tuple[float, float]:
        w, h = self.size
        return (w / 2.0, h / 2.0)

    def image(
        self,
        path: str,
        x: float,
        y: float,
        size: Any = None,
        color: Any = (1.0, 1.0, 1.0, 1.0),
        rotation: float = 0.0,
    ) -> None:
        sz = None if size is None else (float(size[0]), float(size[1]))
        self._s.image(str(path), float(x), float(y), sz, tuple(float(c) for c in color), float(rotation))

    def rect(
        self,
        x: float,
        y: float,
        width: float,
        height: float,
        color: Any = (1.0, 1.0, 1.0, 1.0),
    ) -> None:
        self._s.rect(float(x), float(y), float(width), float(height), tuple(float(c) for c in color))


class ScriptContext:
    """The `ctx` a behaviour script receives."""

    def __init__(self, ctx: Any, handles: Dict[str, ScriptEntity]) -> None:
        self.raw = ctx
        _FRAME["ctx"] = ctx
        self.input = _Input(ctx.input)
        self.time = ctx.time
        self.events = ctx.events
        self.resources = ctx.resources
        self.scene = _Scene(ctx.scene, handles)
        self.physics = _Physics(ctx.physics, self.scene)
        self.audio = _Audio(ctx.audio)
        self.hud = _Hud(ctx.screen)

    @property
    def camera(self) -> Optional[ScriptCamera]:
        """The scene camera the frame renders through (None without one)."""
        for view in self.raw.scene:
            if view.camera_3d is not None:
                return _CAMERAS.of(view)
        return None

    def log(self, *parts: Any) -> None:
        print(*parts)

    warn = log

    def quit(self) -> None:
        self.raw.quit()


def load_script(path: str) -> Any:
    """Imports a behaviour script file as a fresh module."""
    name = "automata_script_" + str(abs(hash(os.path.abspath(path))))
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load script {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def run_scene(
    path: str,
    *,
    max_frames: Optional[int] = None,
    update: Optional[Callable[[Any], None]] = None,
    **game_options: Any,
) -> int:
    """Loads an Oberon scene file and runs it with its behaviour scripts.

    Script paths are resolved relative to the scene file. `update`, if
    given, runs after the scripts every frame (with the raw frame context).
    Extra keyword arguments go to `Game` (`headless=True`, `title=...`).
    Exceptions raised by scripts propagate out of `run_scene`, like any
    update callback. Returns the number of frames run.
    """
    scene = _core.Scene.load(path)
    base = os.path.dirname(os.path.abspath(path))
    scripted: List[Tuple[str, str]] = []
    for entity in scene.entities():
        if entity.script and entity.name:
            # Scenes store `/`-separated paths; join per component so they
            # also resolve under Windows `\\?\` verbatim base paths.
            parts = entity.script.replace("\\", "/").split("/")
            scripted.append((entity.name, os.path.join(base, *parts)))

    from . import Game  # local import: automata/__init__ imports this module

    game_options.setdefault("title", f"{os.path.basename(path)} — AUTOMATA")
    game = Game(**game_options)
    game.load_scene(scene)

    modules: Dict[str, Any] = {}
    handles: Dict[str, ScriptEntity] = {}
    started: set = set()

    def tick(ctx: Any) -> None:
        _CAMERAS.begin(ctx.time.delta)
        sctx = ScriptContext(ctx, handles)
        live = []
        for name, script_path in scripted:
            view = ctx.scene.find(name)
            handle = handles.setdefault(name, ScriptEntity(name, script_path))
            handle._view = view
            if view is None:
                continue
            try:
                handle.grounded = bool(ctx.physics.grounded(view))
            except Exception:  # noqa: BLE001 - not a character
                handle.grounded = False
            live.append(handle)
        for handle in live:
            assert handle.script is not None
            module = modules.get(handle.script)
            if module is None:
                module = modules[handle.script] = load_script(handle.script)
            if handle.name not in started:
                started.add(handle.name)
                start = getattr(module, "start", None)
                if start is not None:
                    start(handle, sctx)
            step = getattr(module, "update", None)
            if step is not None:
                step(handle, sctx)
        if update is not None:
            update(ctx)

    try:
        return game.run(tick, max_frames=max_frames)
    finally:
        for name, script_path in scripted:
            handle = handles.get(name)
            module = modules.get(script_path)
            stop = getattr(module, "stop", None) if module else None
            if handle is not None and handle._view is not None and stop is not None:
                try:
                    stop(handle, None)
                except Exception:  # noqa: BLE001 - shutting down
                    pass
