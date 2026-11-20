"""Run an Oberon scene with its behaviour scripts attached.

Oberon lets you attach a Python script to any entity (Inspector → Script).
In the editor's Play mode those scripts run in a bridged Python process;
`run_scene` runs the very same files inside the real engine:

    import automata as at
    at.run_scene("level.ron")

A behaviour script is a module with optional `start(entity, ctx)`,
`update(entity, ctx)` and `stop(entity, ctx)` functions. `entity` offers
`name`, `transform`, `character_velocity`, `attributes`, `grounded` and a
per-entity `state` dict; `ctx` offers `input`, `time`, `physics`, `scene`,
`events`, `resources`, `log(...)` and `quit()`.
"""

import importlib.util
import os
import sys
from typing import Any, Callable, Dict, List, Optional, Tuple

from . import _core


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
    def attributes(self) -> Any:
        return self._view.attributes

    def despawn(self) -> None:
        self._view.despawn()

    def __repr__(self) -> str:
        return f"ScriptEntity({self.name!r})"


def _unwrap(entity: Any) -> Any:
    return entity._view if isinstance(entity, ScriptEntity) else entity


class _Physics:
    def __init__(self, physics: Any) -> None:
        self._p = physics

    def grounded(self, entity: Any) -> bool:
        if isinstance(entity, ScriptEntity):
            return entity.grounded
        return bool(self._p.grounded(entity))

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


class ScriptContext:
    """The `ctx` a behaviour script receives."""

    def __init__(self, ctx: Any, handles: Dict[str, ScriptEntity]) -> None:
        self.raw = ctx
        self.input = ctx.input
        self.time = ctx.time
        self.events = ctx.events
        self.resources = ctx.resources
        self.physics = _Physics(ctx.physics)
        self.scene = _Scene(ctx.scene, handles)

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
            scripted.append((entity.name, os.path.join(base, entity.script)))

    from . import Game  # local import: automata/__init__ imports this module

    game_options.setdefault("title", f"{os.path.basename(path)} — AUTOMATA")
    game = Game(**game_options)
    game.load_scene(scene)

    modules: Dict[str, Any] = {}
    handles: Dict[str, ScriptEntity] = {}
    started: set = set()

    def tick(ctx: Any) -> None:
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
