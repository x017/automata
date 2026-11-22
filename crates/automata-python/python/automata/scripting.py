"""Run an Oberon scene with its behaviour scripts attached.

Oberon lets you attach a Python script to any entity (Inspector → Script).
In the editor's Play mode those scripts run in a bridged Python process;
`run_scene` runs the very same files inside the real engine:

    import automata as at
    at.run_scene("level.ron")

Both run the scripts through one implementation of `entity` and `ctx`,
`automata.script_api`, so a script behaves the same in the editor and in
the game. This module is the engine's driver for it: it turns the engine's
frame context into per-entity records, runs the scripts, and applies what
they changed. Like in Oberon, writes are visible to later scripts in the
same frame and reach the simulation after every script has run.

Under `run_scene`, `ctx` also has the engine's extras: `ctx.events`,
`ctx.resources`, `ctx.quit()`, `ctx.raw` (the engine's frame context) and
the full `ctx.physics` / `ctx.audio` / `ctx.scene` APIs, which accept
script entities wherever they take an entity.
"""

import json
import os
import sys
import time
from typing import Any, Callable, Dict, List, Optional

from . import _core
from . import script_api as api
from .script_api import Hit

__all__ = [
    "Hit",
    "ScriptCamera",
    "ScriptContext",
    "ScriptEntity",
    "SceneScripts",
    "load_script",
    "run_scene",
]

#: The script-facing classes, under their historical names.
ScriptEntity = api.Entity
ScriptContext = api.Context
ScriptCamera = api.Camera

#: Imports a behaviour script file as a fresh module.
load_script = api.load_module


def _unwrap(value: Any) -> Any:
    """Script entities → engine views (for the engine's own APIs)."""
    if isinstance(value, api.Entity):
        view = value.view
        return view if view is not None else value.id
    return value


def _unwrapping(attr: Any) -> Any:
    if not callable(attr):
        return attr

    def call(*args: Any, **kwargs: Any) -> Any:
        return attr(*(_unwrap(a) for a in args), **{k: _unwrap(v) for k, v in kwargs.items()})

    return call


class _EngineKeys:
    """The engine's input, as the shared API's key source."""

    def __init__(self) -> None:
        self.raw: Any = None
        self._screen: Any = None

    def begin(self, ctx: Any) -> None:
        self.raw = ctx.input
        self._screen = ctx.screen

    def key_down(self, k: str) -> bool:
        return bool(self.raw.pressed(k))

    def key_pressed(self, k: str) -> bool:
        return bool(self.raw.just_pressed(k))

    def key_released(self, k: str) -> bool:
        return bool(self.raw.just_released(k))

    def button_down(self, b: str) -> bool:
        return bool(self.raw.mouse_pressed(b))

    def button_pressed(self, b: str) -> bool:
        return bool(self.raw.mouse_just_pressed(b))

    def button_released(self, b: str) -> bool:
        return bool(self.raw.mouse_just_released(b))

    def mouse_delta(self) -> Any:
        return self.raw.mouse_delta

    def mouse_locked(self) -> bool:
        return bool(self._screen.cursor_locked)


class _ViewRecord:
    """One entity's start-of-frame values, read lazily from its view."""

    __slots__ = ("view", "_backend", "_t")

    def __init__(self, view: Any, backend: "_EngineBackend") -> None:
        self.view = view
        self._backend = backend
        self._t: Any = False

    def _transform(self) -> Any:
        if self._t is False:
            self._t = self.view.transform
        return self._t

    def get(self, key: str) -> Any:
        v = self.view
        if key == "id":
            return int(v.id)
        if key == "name":
            return v.name
        if key == "script":
            return self._backend.resolve(v.script)
        if key == "p":
            t = self._transform()
            return None if t is None else t.position
        if key == "r":
            t = self._transform()
            return None if t is None else t.rotation_euler
        if key == "s":
            t = self._transform()
            return None if t is None else t.scale
        if key == "v":
            return v.character_velocity
        if key == "g":
            return self._backend.grounded(v)
        if key == "cam":
            c = v.camera_3d
            return None if c is None else (*c.position, *c.target, c.fov)
        if key == "attrs":
            return dict(v.attributes.items())
        if key == "snd":
            return v.audio_source
        if key == "view":
            return v
        return None


class _EngineBackend(api.Backend):
    """Answers the shared API's questions from the engine's frame context."""

    def __init__(self, base_dir: str) -> None:
        self.base = base_dir
        self.ctx: Any = None
        self.keys = _EngineKeys()
        self._paths: Dict[str, str] = {}

    # --- frames -----------------------------------------------------------

    def begin(self, ctx: Any) -> List[_ViewRecord]:
        self.ctx = ctx
        self.keys.begin(ctx)
        return [_ViewRecord(view, self) for view in ctx.scene]

    def apply(self, out: api.FrameOutput, records: List[_ViewRecord]) -> None:
        ctx = self.ctx
        views = {int(r.view.id): r.view for r in records}
        for w in out.writes:
            view = views.get(w["id"])
            if view is None:
                continue
            if "p" in w or "r" in w or "s" in w:
                t = view.transform
                if t is None:
                    view.transform = _core.Transform(
                        position=w.get("p", (0.0, 0.0, 0.0)),
                        rotation_euler=w.get("r", (0.0, 0.0, 0.0)),
                        scale=w.get("s", (1.0, 1.0, 1.0)),
                    )
                else:
                    if "p" in w:
                        t.position = w["p"]
                    if "r" in w:
                        t.rotation_euler = w["r"]
                    if "s" in w:
                        t.scale = w["s"]
            if "v" in w:
                view.character_velocity = w["v"]
            if "cam" in w:
                c = w["cam"]
                view.camera_3d = _core.Camera3D(position=c[0:3], target=c[3:6], fov=c[6])
            if "attrs" in w:
                view.attributes = w["attrs"]
            audio = w.get("audio")
            if audio == "play":
                ctx.audio.play_source(view)
            elif audio == "stop":
                ctx.audio.stop_source(view)
            if w.get("despawn"):
                view.despawn()
        for path, volume, pitch, pos in out.sounds:
            ctx.audio.play_at(path, volume, pitch, tuple(pos) if pos is not None else None)
        for item in out.hud:
            color = tuple(item["c"])
            if item["k"] == "img":
                size = (item["w"], item["h"]) if "w" in item else None
                ctx.screen.image(item["path"], item["x"], item["y"], size, color, item["rot"])
            elif item["k"] == "text":
                shadow, outline = item.get("shadow"), item.get("outline")
                ctx.screen.text(
                    item["text"], item["x"], item["y"], item["size"], color, item["align"],
                    tuple(shadow) if shadow else None, tuple(outline) if outline else None,
                )
            else:
                ctx.screen.rect(item["x"], item["y"], item["w"], item["h"], color)
        if out.lock is not None:
            ctx.screen.lock_cursor(out.lock)

    def resolve(self, script: Optional[str]) -> Optional[str]:
        """A scene's script path (relative to the scene file) made absolute."""
        if not script:
            return None
        path = self._paths.get(script)
        if path is None:
            if os.path.isabs(script):
                path = script
            else:
                # Scenes store `/`-separated paths; join per component so they
                # also resolve under Windows `\\?\` verbatim base paths.
                path = os.path.join(self.base, *script.replace("\\", "/").split("/"))
            self._paths[script] = path
        return path

    # --- the shared API's questions ---------------------------------------

    def raycast(self, origin: Any, direction: Any, max_distance: float, ignore_id: Any) -> Any:
        return self.ctx.physics.raycast(origin, direction, max_distance, ignore_id)

    def water_height(self, x: float, z: float) -> Any:
        return self.ctx.physics.water_height(x, z)

    def splash(self, position: Any, strength: float) -> None:
        self.ctx.physics.splash(tuple(position), strength)

    def grounded(self, entity: Any) -> bool:
        try:
            return bool(self.ctx.physics.grounded(_unwrap(entity)))
        except Exception:  # noqa: BLE001 - not a character
            return False

    def source_playing(self, entity: Any) -> bool:
        return bool(self.ctx.audio.source_playing(_unwrap(entity)))

    def camera_3d(self, entity: Any) -> Any:
        c = entity._read("cam")
        if c is None:
            return None
        return _core.Camera3D(position=c[0:3], target=c[3:6], fov=c[6])

    def log(self, level: str, message: str) -> None:
        print(message, file=sys.stderr if level != "info" else sys.stdout)

    def extra(self, owner: str, name: str) -> Any:
        ctx = self.ctx
        if ctx is None:
            return super().extra(owner, name)
        if owner == "ctx":
            if name == "raw":
                return ctx
            return _unwrapping(getattr(ctx, name))
        return _unwrapping(getattr(getattr(ctx, owner), name))


class SceneScripts:
    """The behaviour scripts of one running scene, advanced frame by frame.

    `run_scene` drives one of these from the game loop: call `tick(ctx)` in
    every update callback, and `stop()` once the game ends.
    """

    def __init__(self, base_dir: str) -> None:
        self.backend = _EngineBackend(base_dir)
        self.host = api.ScriptHost(self.backend, self.backend.keys, on_error="raise")

    def tick(self, ctx: Any) -> None:
        """Runs every script for this frame and queues what they changed."""
        records = self.backend.begin(ctx)
        self.host.ctx.time.raw = ctx.time
        out = self.host.frame(
            records,
            delta=ctx.time.delta,
            elapsed=ctx.time.elapsed,
            frame=ctx.time.frame,
            screen=ctx.screen.size,
        )
        self.backend.apply(out, records)

    def stop(self) -> None:
        """Calls every started script's `stop`. Their writes have nowhere to
        go, and one failing script doesn't skip the others."""
        self.host.on_error = "report"
        self.host.stop()


class _FrameStats:
    """Times frames for benchmarks (``examples/bench``).

    Enabled by the ``AUTOMATA_FRAME_STATS`` environment variable (a JSON
    file to write when the run ends), so exported builds can be measured
    too. ``AUTOMATA_FRAME_STATS_WARMUP`` frames (default 30) are skipped;
    after ``AUTOMATA_FRAME_STATS_SECONDS`` of measuring (default: no limit)
    the game quits. A frame runs from one script tick to the next, so it
    holds the scripts plus everything the engine does in between.
    """

    def __init__(self, path: str) -> None:
        self.path = path
        self.warmup = int(os.environ.get("AUTOMATA_FRAME_STATS_WARMUP", "30"))
        seconds = os.environ.get("AUTOMATA_FRAME_STATS_SECONDS")
        self.seconds = float(seconds) if seconds else None
        self.frame_ms: List[float] = []
        self.script_ms: List[float] = []
        self._frames = 0
        self._last: Optional[float] = None
        self._last_script = 0.0
        self._since: Optional[float] = None

    @classmethod
    def from_env(cls) -> "Optional[_FrameStats]":
        path = os.environ.get("AUTOMATA_FRAME_STATS")
        return cls(path) if path else None

    def begin(self) -> float:
        now = time.perf_counter()
        if self._last is not None and self._frames > self.warmup:
            self.frame_ms.append((now - self._last) * 1000.0)
            self.script_ms.append(self._last_script * 1000.0)
            if self._since is None:
                self._since = self._last
        self._last = now
        self._frames += 1
        return now

    def end(self, started: float, ctx: Any) -> None:
        self._last_script = time.perf_counter() - started
        if self.seconds is not None and self._since is not None:
            if time.perf_counter() - self._since > self.seconds:
                ctx.quit()

    def write(self) -> None:
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(
                {"runner": "run_scene", "frame_ms": self.frame_ms, "script_ms": self.script_ms},
                f,
            )


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

    from . import Game  # local import: automata/__init__ imports this module

    game_options.setdefault("title", f"{os.path.basename(path)} — AUTOMATA")
    game = Game(**game_options)
    game.load_scene(scene)
    scripts = SceneScripts(os.path.dirname(os.path.abspath(path)))
    stats = _FrameStats.from_env()

    def tick(ctx: Any) -> None:
        started = stats.begin() if stats is not None else 0.0
        scripts.tick(ctx)
        if stats is not None:
            stats.end(started, ctx)
        if update is not None:
            update(ctx)

    try:
        return game.run(tick, max_frames=max_frames)
    finally:
        scripts.stop()
        if stats is not None:
            stats.write()
