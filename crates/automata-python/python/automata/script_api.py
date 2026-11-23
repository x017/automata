"""The behaviour-script API, shared by Oberon's Play mode and `run_scene`.

A behaviour script is a Python file with optional `start(entity, ctx)`,
`update(entity, ctx)` and `stop(entity, ctx)` functions. This module is the
one implementation of the `entity` and `ctx` objects those functions receive,
plus script loading, hot reload and error reporting. Two thin drivers run it:

- Oberon's bridge (`crates/oberon/src/bridge.py`) feeds it the JSON frames
  the editor sends and replies with what the scripts changed. Oberon ships a
  copy of this file next to the bridge, so Play mode needs only a plain
  `python3`.
- `automata.scripting.run_scene` feeds it the real engine's frame context.

It is standard-library Python on purpose, for the bridge's sake.

The frame model
---------------

Every frame, the driver hands the host one *record* per entity: an object
with a ``get(key)`` method returning the start-of-frame value of:

- ``id``, ``name``, ``script`` (absolute path or None);
- ``p``, ``r``, ``s``: position, rotation (XYZ Euler radians), scale;
- ``v``: character velocity or None; ``g``: grounded;
- ``cam``: ``(x, y, z, tx, ty, tz, fov)`` or None;
- ``attrs``: attribute dict; ``snd``: the Audio Source (truthy) or None;
- ``anim``: an animated model's playback or None: a dict with ``clip``
  (name or None), ``time``, ``len`` (the clip's seconds), ``playing``,
  ``speed``, ``loop``, ``clips`` (names), ``lens`` (their seconds),
  ``springs``, ``rm`` (root motion on) and ``ev``: the latest animation
  events passed, ``[seq, clip, name]`` oldest first, ``seq`` counting up
  per entity (the host delivers each new one once);
- ``view``: the engine's own handle (run_scene only).

Scripts read those values and write new ones. **Writes are visible to every
script that reads later in the same frame, and apply to the simulation
after all scripts have run.** That's why four drones doing
``player.attributes["health"] -= 5`` in one frame take 20 health, not 5.
The host returns the writes (plus sounds, HUD items and cursor-lock
requests) as a :class:`FrameOutput` for the driver to apply.

Channels
--------

Scripts talk through named channels, Redis style: ``ctx.publish(channel,
data)`` delivers a :class:`Message` at once to every function subscribed
with ``ctx.subscribe(pattern, fn)``. Patterns match exactly, or with ``*``
and ``?`` wildcards (``"*:animation:hit"``). The engine publishes animation
events as ``<entity name>:animation:<event>`` at the start of the frame
after the playhead passes them, just before ``update`` runs.
"""

from __future__ import annotations

import difflib
import fnmatch
import importlib.util
import json
import math
import os
import sys
import time
import traceback
import types
from typing import TYPE_CHECKING, Any, Callable, Iterable

#: An (x, y, z) triple.
Vec3 = tuple[float, float, float]

__all__ = [
    "Attributes",
    "Camera",
    "Context",
    "Entity",
    "EntityAnimation",
    "EntityAudio",
    "FrameOutput",
    "Hit",
    "Hud",
    "Input",
    "ComponentRef",
    "FieldRef",
    "KeySource",
    "KeyStore",
    "Message",
    "ScriptHost",
    "Subscription",
    "Time",
    "Timer",
    "Tween",
    "EASINGS",
    "ease",
    "easing",
    "SaveData",
    "SaveSlot",
    "Settings",
    "KEY_NAMES",
    "GRAPHICS_PRESETS",
    "user_data_dir",
    "dev_save_dir",
]


def _vec(v, n):
    t = tuple(float(x) for x in v)
    if len(t) != n:
        raise ValueError(f"expected {n} numbers, got {len(t)}")
    return t


def _tuple(v):
    return tuple(v) if isinstance(v, list) else v


KEY_ALIASES = {
    "shift": ("shift_left", "shift_right"),
    "ctrl": ("ctrl_left", "ctrl_right"),
    "alt": ("alt_left", "alt_right"),
    "left": ("arrow_left",),
    "right": ("arrow_right",),
    "up": ("arrow_up",),
    "down": ("arrow_down",),
}


def keys_for(key):
    """The key names a script's key means (``"shift"`` is either Shift)."""
    k = str(getattr(key, "value", key)).lower()
    return KEY_ALIASES.get(k, (k,))


_MISSING = object()


# --------------------------------------------------------------------------
# entity
# --------------------------------------------------------------------------


class Transform:
    """`entity.transform`: position / rotation_euler (radians) / scale."""

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    @property
    def position(self) -> Vec3:
        return self._e._read("p", (0.0, 0.0, 0.0))

    @position.setter
    def position(self, v: Iterable[float]) -> None:
        self._e._write("p", _vec(v, 3))

    @property
    def rotation_euler(self) -> Vec3:
        return self._e._read("r", (0.0, 0.0, 0.0))

    @rotation_euler.setter
    def rotation_euler(self, v: Iterable[float]) -> None:
        self._e._write("r", _vec(v, 3))

    @property
    def scale(self) -> Vec3:
        return self._e._read("s", (1.0, 1.0, 1.0))

    @scale.setter
    def scale(self, v: Iterable[float]) -> None:
        self._e._write("s", _vec(v, 3))

    def translate(self, delta: Iterable[float]) -> None:
        """Moves by ``delta`` (x, y, z)."""
        d = _vec(delta, 3)
        self.position = tuple(a + b for a, b in zip(self.position, d))

    def rotate(self, delta: Iterable[float]) -> None:
        """Adds ``delta`` (radians, x, y, z) to the rotation."""
        d = _vec(delta, 3)
        self.rotation_euler = tuple(a + b for a, b in zip(self.rotation_euler, d))

    def __repr__(self):
        return f"Transform(position={self.position}, rotation_euler={self.rotation_euler})"


class Camera:
    """An entity's perspective camera (``entity.camera`` / ``ctx.camera``).

    Play renders through the scene's first camera. Move it by setting
    ``position`` / ``target`` or with ``look_at`` and ``follow``.
    """

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def _get(self, a, b):
        return tuple(self._e._read("cam")[a:b])

    def _set(self, a, values):
        cam = list(self._e._read("cam"))
        cam[a:a + len(values)] = values
        self._e._write("cam", tuple(cam))

    @property
    def position(self) -> Vec3:
        """The eye position (x, y, z)."""
        return self._get(0, 3)

    @position.setter
    def position(self, v: Iterable[float]) -> None:
        self._set(0, _vec(v, 3))

    @property
    def target(self) -> Vec3:
        """The point the camera looks at (x, y, z)."""
        return self._get(3, 6)

    @target.setter
    def target(self, v: Iterable[float]) -> None:
        self._set(3, _vec(v, 3))

    @property
    def fov(self) -> float:
        """Vertical field of view in degrees."""
        return self._e._read("cam")[6]

    @fov.setter
    def fov(self, degrees: float) -> None:
        self._set(6, (min(179.0, max(1.0, float(degrees))),))

    def look_at(self, point: Iterable[float]) -> None:
        """Aims the camera at a point or an entity."""
        if hasattr(point, "transform"):
            point = point.transform.position
        self.target = point

    def follow(
        self,
        entity: Entity,
        offset: Iterable[float] = (0.0, 3.0, 8.0),
        smooth: float = 8.0,
        look_height: float = 0.6,
    ) -> None:
        """Moves the camera toward ``entity + offset`` and looks at it.

        Call every frame from ``update``. ``smooth`` is how quickly it
        catches up (0 = snap).
        """
        p = entity.transform.position
        goal = tuple(p[i] + float(offset[i]) for i in range(3))
        k = 1.0
        if smooth > 0:
            k = 1.0 - math.exp(-float(smooth) * max(self._e._host.ctx.time.delta, 0.0))
        cur = self.position
        self.position = tuple(cur[i] + (goal[i] - cur[i]) * k for i in range(3))
        self.target = (p[0], p[1] + float(look_height), p[2])

    def __repr__(self):
        return f"Camera(position={self.position}, target={self.target}, fov={self.fov})"


class Attributes(dict):
    """`entity.attributes`: a dict whose edits are written back."""

    def __init__(self, entity, values):
        super().__init__(values)
        self._e = entity

    def _touch(self):
        self._e._dirty.add("attrs")

    def __setitem__(self, key, value):
        super().__setitem__(key, _tuple(value))
        self._touch()

    def __delitem__(self, key):
        super().__delitem__(key)
        self._touch()

    def update(self, *args, **kwargs):
        super().update(*args, **kwargs)
        self._touch()

    def pop(self, *args):
        self._touch()
        return super().pop(*args)

    def popitem(self):
        self._touch()
        return super().popitem()

    def setdefault(self, key, default=None):
        if key not in self:
            self[key] = default
        return self[key]

    def clear(self):
        super().clear()
        self._touch()


class EntityAudio:
    """``entity.audio``: plays the entity's Audio Source component."""

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def play(self) -> None:
        """Starts the sound (restarts it if it's already playing)."""
        self._e._audio = "play"
        self._e._dirty.add("audio")

    def stop(self) -> None:
        self._e._audio = "stop"
        self._e._dirty.add("audio")

    @property
    def playing(self) -> bool:
        """Whether it was playing at the start of the frame."""
        return bool(self._e._host.backend.source_playing(self._e))

    @property
    def source(self) -> Any:
        """The Audio Source (truthy), or None if the entity has none."""
        return self._e._record.get("snd") or None

    def __repr__(self):
        return f"Audio({self._e.name!r})"


class EntityAnimation:
    """``entity.animation``: plays the clips of the entity's animated model.

    ``play("Walking", fade=0.2)`` crossfades to a clip; calling it every
    frame with the same clip keeps it going. Reads see this frame's own
    requests (``play`` then ``clip`` gives the new clip).
    """

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def _st(self):
        return self._e._anim_state()

    def play(
        self,
        clip: str,
        loop: bool = True,
        fade: float = 0.0,
        speed: float | None = None,
        restart: bool = False,
    ) -> None:
        """Plays ``clip``, crossfading from the current pose over ``fade``
        seconds. Playing the clip that's already playing keeps it going
        unless ``restart``; ``speed`` (if given) sets the playback rate.
        ``loop=False`` plays it once and holds the last frame. Raises
        ``KeyError`` when the model has no such clip."""
        st = self._st()
        clip = str(clip)
        clips = list(st.get("clips") or ())
        if clip not in clips:
            raise KeyError(f"the model has no clip {clip!r}; it has: {', '.join(clips)}")
        speed = None if speed is None else float(speed)
        fade = max(0.0, float(fade))
        self._e._anim_cmd(["play", clip, bool(loop), fade, speed, bool(restart)])
        if speed is not None:
            st["speed"] = speed
        st["loop"] = bool(loop)
        if st.get("clip") == clip and st.get("playing") and not restart:
            return
        length = (st.get("lens") or [0.0] * len(clips))[clips.index(clip)]
        st.update(clip=clip, len=length, playing=True)
        st["time"] = length if float(st.get("speed") or 1.0) < 0 else 0.0

    def stop(self, fade: float = 0.0) -> None:
        """Stops, fading back to the rest pose over ``fade`` seconds."""
        self._e._anim_cmd(["stop", max(0.0, float(fade))])
        self._st().update(clip=None, time=0.0, len=0.0, playing=False)

    @property
    def clip(self) -> str | None:
        """The current clip's name, or None in the rest pose."""
        return self._st().get("clip")

    @property
    def playing(self) -> bool:
        """Whether time advances (False once a one-shot clip has ended)."""
        return bool(self._st().get("playing"))

    @property
    def time(self) -> float:
        """Seconds into the current clip; set it to jump."""
        return float(self._st().get("time") or 0.0)

    @time.setter
    def time(self, t: float) -> None:
        t = float(t)
        self._e._anim_cmd(["seek", t])
        st = self._st()
        length = float(st.get("len") or 0.0)
        if st.get("loop") and length > 0:
            t %= length
        st["time"] = min(max(t, 0.0), length)

    @property
    def duration(self) -> float:
        """The current clip's length in seconds (0 with no clip)."""
        return float(self._st().get("len") or 0.0)

    @property
    def speed(self) -> float:
        """Playback rate (1 is normal speed, negative plays backwards)."""
        return float(self._st().get("speed", 1.0))

    @speed.setter
    def speed(self, v: float) -> None:
        v = float(v)
        self._e._anim_cmd(["speed", v])
        self._st()["speed"] = v

    @property
    def looping(self) -> bool:
        """Whether the current clip loops."""
        return bool(self._st().get("loop"))

    @property
    def clips(self) -> tuple[str, ...]:
        """Every clip the model has."""
        return tuple(self._st().get("clips") or ())

    @property
    def springs(self) -> bool:
        """Whether the model has spring bones (set up in its import settings)."""
        return bool(self._st().get("springs"))

    def reset_springs(self) -> None:
        """Puts the spring bones back at rest, as if the entity had just
        appeared (after moving it somewhere new, say)."""
        self._e._anim_cmd(["reset_springs"])

    @property
    def root_motion(self) -> bool:
        """Whether the clip's own travel across the ground moves the entity
        (through its Character Controller, if it has one) instead of
        sliding the model away from it. Settable."""
        return bool(self._st().get("rm"))

    @root_motion.setter
    def root_motion(self, on: bool) -> None:
        self._e._anim_cmd(["root_motion", bool(on)])
        self._st()["rm"] = bool(on)

    def length(self, clip: str | None = None) -> float:
        """A clip's length in seconds (default: the current clip's)."""
        if clip is None:
            return self.duration
        st = self._st()
        clips = list(st.get("clips") or ())
        if clip not in clips:
            raise KeyError(f"the model has no clip {clip!r}")
        return float((st.get("lens") or [0.0] * len(clips))[clips.index(clip)])

    def __repr__(self):
        return f"Animation({self._e.name!r}, clip={self.clip!r})"


class Entity:
    """A scene entity as seen by a script.

    One handle per entity lives for the whole run, so ``state`` survives
    between frames, and every script sees the same handle (and its writes).
    """

    def __init__(self, host, entity_id):
        self._host = host
        self.id = entity_id
        self.name = ""
        self.script = None
        self.state = {}
        self._transform = Transform(self)
        self._record = {}
        self._vals = {}
        self._attrs = None
        self._dirty = set()
        self._audio = None
        self._anim = None
        self._anim_st = None
        self._despawn = False
        self._ev_seen = 0  # the last animation event delivered

    def _begin(self, record):
        """Starts a frame from a fresh record."""
        self._record = record
        self._vals = {}
        self._attrs = None
        self._dirty = set()
        self._audio = None
        self._anim = None
        self._anim_st = None
        self.name = record.get("name") or ""
        self.script = record.get("script")

    def _read(self, key, default=None):
        v = self._vals.get(key, _MISSING)
        if v is _MISSING:
            v = self._record.get(key)
            v = default if v is None else _tuple(v)
            self._vals[key] = v
        return v

    def _write(self, key, value):
        self._vals[key] = value
        self._dirty.add(key)

    @property
    def transform(self) -> Transform:
        """Position, rotation (``rotation_euler``, radians) and scale."""
        return self._transform

    @transform.setter
    def transform(self, value: Any) -> None:
        # Any transform-like object: ours, the engine's Transform, ...
        self._transform.position = value.position
        if hasattr(value, "rotation_euler"):
            self._transform.rotation_euler = value.rotation_euler
        if hasattr(value, "scale"):
            self._transform.scale = value.scale

    @property
    def character_velocity(self) -> Vec3 | None:
        """The Character Controller's velocity, or None without one."""
        return self._read("v")

    @character_velocity.setter
    def character_velocity(self, v: Iterable[float]) -> None:
        self._write("v", _vec(v, 3))

    @property
    def grounded(self) -> bool:
        """True when a character is standing on something."""
        return bool(self._read("g", False))

    @property
    def attributes(self) -> Attributes:
        """Custom values from the Inspector; read and write like a dict."""
        if self._attrs is None:
            raw = self._record.get("attrs") or {}
            self._attrs = Attributes(self, {k: _tuple(v) for k, v in raw.items()})
        return self._attrs

    @attributes.setter
    def attributes(self, values: dict[str, Any]) -> None:
        self._attrs = Attributes(self, {k: _tuple(v) for k, v in dict(values).items()})
        self._dirty.add("attrs")

    @property
    def camera(self) -> Camera | None:
        """This entity's Camera, or None if it has no Camera component."""
        return Camera(self) if self._read("cam") is not None else None

    @property
    def camera_3d(self) -> Any:
        """The engine's ``Camera3D`` (``run_scene`` only; prefer ``camera``)."""
        return self._host.backend.camera_3d(self)

    @camera_3d.setter
    def camera_3d(self, value: Any) -> None:
        self._write("cam", (*_vec(value.position, 3), *_vec(value.target, 3), float(value.fov)))

    @property
    def audio(self) -> EntityAudio | None:
        """This entity's Audio Source: ``play()``, ``stop()``, ``playing``."""
        return EntityAudio(self)

    @property
    def animation(self) -> EntityAnimation | None:
        """This entity's animated model: ``play(clip, loop=True, fade=0,
        speed=None)``, ``stop(fade=0)``, ``clip``, ``time``, ``playing``,
        ``speed``, ``clips``. None when its mesh isn't animated."""
        return EntityAnimation(self) if self._anim_state() is not None else None

    def _anim_state(self):
        """This frame's playback (a dict our own requests update), or None."""
        if self._anim_st is None:
            rec = self._record.get("anim")
            self._anim_st = False if rec is None else dict(rec)
        return self._anim_st if self._anim_st is not False else None

    def _anim_cmd(self, cmd):
        if self._anim is None:
            self._anim = []
        self._anim.append(cmd)
        self._dirty.add("anim")

    @property
    def view(self) -> Any:
        """The engine's own handle for the entity (``run_scene`` only)."""
        return self._record.get("view")

    def despawn(self) -> None:
        """Removes the entity from the running scene (after this frame)."""
        self._despawn = True
        self._dirty.add("despawn")

    def _writes(self):
        if not self._dirty:
            return None
        w = {"id": self.id}
        d = self._dirty
        if "despawn" in d:
            w["despawn"] = True
        if self._audio is not None:
            w["audio"] = self._audio
        if self._anim:
            w["anim"] = list(self._anim)
        for key in ("p", "r", "s", "v", "cam"):
            if key in d and self._vals.get(key) is not None:
                w[key] = self._vals[key]
        if "attrs" in d and self._attrs is not None:
            w["attrs"] = dict(self._attrs)
        return w

    def __repr__(self):
        return f"Entity({self.name!r})"


# --------------------------------------------------------------------------
# ctx
# --------------------------------------------------------------------------


class KeySource:
    """Keyboard and mouse state from plain sets (what Oberon sends).

    Drivers with their own input (the engine) pass any object with the same
    methods to :class:`Input`.
    """

    def __init__(self):
        self.down = set()
        self.prev = set()
        self.buttons = set()
        self.prev_buttons = set()
        self.delta = (0.0, 0.0)
        self.locked = False
        self.raw = None

    def update(self, keys, buttons=(), delta=(0.0, 0.0), locked=False):
        self.prev, self.down = self.down, set(keys)
        self.prev_buttons, self.buttons = self.buttons, set(buttons)
        self.delta = (float(delta[0]), float(delta[1]))
        self.locked = bool(locked)

    def key_down(self, k):
        return k in self.down

    def key_pressed(self, k):
        return k in self.down and k not in self.prev

    def key_released(self, k):
        return k in self.prev and k not in self.down

    def button_down(self, b):
        return b in self.buttons

    def button_pressed(self, b):
        return b in self.buttons and b not in self.prev_buttons

    def button_released(self, b):
        return b in self.prev_buttons and b not in self.buttons

    def mouse_delta(self):
        return self.delta

    def mouse_locked(self):
        return self.locked


class Input:
    """``ctx.input``: keys, mouse buttons, mouse movement and capture."""

    def __init__(self, source):
        self._src = source
        self._lock_request = None
        self._settings: Settings | None = None  # ctx.settings, for action names

    def _any(self, key, on_key, on_button):
        settings = self._settings
        if settings is not None and isinstance(key, str) and settings.is_action(key):
            for k in settings.keys(key):
                button = MOUSE_BINDINGS.get(k)
                if button is not None:
                    if on_button(button):
                        return True
                elif any(on_key(n) for n in keys_for(k)):
                    return True
            return False
        return any(on_key(k) for k in keys_for(key))

    def pressed(self, key: str) -> bool:
        """The key is held down. Takes a key name (``"space"``) or an
        action bound with ``ctx.settings.bind`` (``"jump"``)."""
        return self._any(key, self._src.key_down, self._src.button_down)

    def just_pressed(self, key: str) -> bool:
        """The key (or action) went down this frame."""
        return self._any(key, self._src.key_pressed, self._src.button_pressed)

    def just_released(self, key: str) -> bool:
        """The key (or action) came up this frame."""
        return self._any(key, self._src.key_released, self._src.button_released)

    def axis(self, negative: str, positive: str) -> float:
        """-1, 0, or 1 from two keys (or actions)."""
        return float(self.pressed(positive)) - float(self.pressed(negative))

    def any_just_pressed(self) -> str | None:
        """The key that went down this frame (``"k"``, ``"space"``,
        ``"mouse_left"``...), or None: for "press a key" rebinding
        screens."""
        for k in KEY_NAMES:
            if self._src.key_pressed(k):
                return k
        for name, button in MOUSE_BINDINGS.items():
            if self._src.button_pressed(button):
                return name
        return None

    def mouse_pressed(self, button: str = "left") -> bool:
        return self._src.button_down(str(button).lower())

    def mouse_just_pressed(self, button: str = "left") -> bool:
        return self._src.button_pressed(str(button).lower())

    def mouse_just_released(self, button: str = "left") -> bool:
        return self._src.button_released(str(button).lower())

    @property
    def mouse_delta(self) -> tuple[float, float]:
        """Mouse movement since last frame (dx, dy) in pixels; +y is down."""
        dx, dy = self._src.mouse_delta()
        return (float(dx), float(dy))

    def lock_mouse(self, locked: bool = True) -> None:
        """Hides and captures the cursor for mouse look (False frees it).
        In Oberon, Esc gives the cursor back; click the view to recapture."""
        self._lock_request = bool(locked)

    @property
    def mouse_locked(self) -> bool:
        if self._lock_request is not None:
            return self._lock_request
        return bool(self._src.mouse_locked())

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            raw = getattr(self._src, "raw", None)
            if raw is None:
                raise AttributeError(f"ctx.input has no {name!r}")
            return getattr(raw, name)


class Time:
    """``ctx.time``: ``delta`` (seconds since last frame), ``elapsed``
    (seconds since Play started) and ``frame`` (starting at 1)."""

    def __init__(self):
        self.delta = 0.0
        self.elapsed = 0.0
        self.frame = 0
        self.raw = None

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            raw = self.__dict__.get("raw")
            if raw is None:
                raise AttributeError(f"ctx.time has no {name!r}")
            return getattr(raw, name)


class Hit:
    """A raycast hit: ``entity`` (or None), ``point``, ``normal``, ``distance``."""

    __slots__ = ("entity", "point", "normal", "distance")

    entity: Entity | None
    point: Vec3
    normal: Vec3
    distance: float

    def __init__(self, entity, point, normal, distance):
        self.entity = entity
        self.point = tuple(point)
        self.normal = tuple(normal)
        self.distance = float(distance)

    def __repr__(self):
        return f"Hit({getattr(self.entity, 'name', None)!r}, distance={self.distance:.2f})"


class Physics:
    """``ctx.physics``: ``grounded`` and ``raycast`` (plus the engine's full
    physics API under ``run_scene``)."""

    def __init__(self, host):
        self._host = host

    def grounded(self, entity: Entity) -> bool:
        if isinstance(entity, Entity):
            return entity.grounded
        return bool(self._host.backend.grounded(entity))

    def raycast(
        self,
        origin: Iterable[float],
        direction: Iterable[float],
        max_distance: float = 100.0,
        ignore: Entity | None = None,
    ) -> Hit | None:
        """The first collider along a ray, right now, or None.

        ``ignore`` skips one entity (usually the caster). Sensors are skipped.
        """
        o = _vec(tuple(origin)[:3], 3)
        d = _vec(tuple(direction)[:3], 3)
        length = math.sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
        if length < 1e-9:
            return None
        d = (d[0] / length, d[1] / length, d[2] / length)
        ignore_id = getattr(ignore, "id", ignore)
        hit = self._host.backend.raycast(o, d, float(max_distance), ignore_id)
        if hit is None:
            return None
        sid, point, normal, distance = hit
        return Hit(self._host.entities.get(sid), point, normal, distance)

    def water_height(self, x: float, z: float) -> float | None:
        """The height of the water surface at world ``(x, z)``, waves
        included (what floating bodies ride), or None where there's no
        water."""
        h = self._host.backend.water_height(float(x), float(z))
        return None if h is None else float(h)

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            return self._host.backend.extra("physics", name)


class Water:
    """``ctx.water``: the scene's water."""

    def __init__(self, host):
        self._host = host

    def height(self, x: float, z: float) -> float | None:
        """The height of the water surface at world ``(x, z)``, waves
        included, or None where there's no water (same as
        ``ctx.physics.water_height``)."""
        h = self._host.backend.water_height(float(x), float(z))
        return None if h is None else float(h)

    def splash(self, position: Iterable[float], strength: float = 1.0) -> None:
        """Makes a splash at ``position`` (a point or an entity; its X and
        Z): rings spread out from it and foam gathers. ``strength`` 1 is
        about a crate dropped from a couple of metres (0 to 10). Things
        that fall in or move through the water make their own ripples;
        this is for everything else (a cannonball, a jumping fish)."""
        if hasattr(position, "transform"):
            position = position.transform.position
        pos = _vec(tuple(position)[:3], 3)
        strength = min(max(float(strength), 0.0), 10.0)
        if strength > 0.0:
            self._host.backend.splash(pos, strength)

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            return self._host.backend.extra("water", name)


PARTICLE_PRESETS = ("fire", "smoke", "sparks", "dust", "splash", "magic")

PARTICLE_FIELDS = (
    "emitting", "rate", "burst", "duration", "one_shot", "max_particles",
    "lifetime", "shape", "shape_size", "cone_angle", "speed", "gravity",
    "drag", "size", "size_spread", "color", "color_end", "fade_in",
    "fade_out", "spin", "stretch", "blend", "lit", "soft", "sprite",
    "texture", "sheet", "world_space",
)


def _particle_value(v):
    if isinstance(v, (bool, str)) or v is None:
        return v
    if isinstance(v, (int, float)):
        return float(v)
    return [float(x) for x in v]


class Particles:
    """``ctx.particles``: one-off particle effects."""

    def __init__(self, host):
        self._host = host

    def burst(
        self,
        effect: str,
        position: Iterable[float],
        direction: Iterable[float] | None = None,
        count: int | None = None,
        **fields: Any,
    ) -> None:
        """A one-off effect at ``position`` (a point or an entity) that
        cleans up after itself: sparks where a shot hits, a puff of dust
        where something lands.

        ``effect`` is a preset name (``"fire"``, ``"smoke"``, ``"sparks"``,
        ``"dust"``, ``"splash"``, ``"magic"``) or an ``at.ParticleEmitter``;
        ``fields`` change it (``color=(0.3, 1, 0.4, 1)``). ``direction``
        turns it to fly that way (a hit's normal) instead of up. ``count``
        is how many particles (default: the effect's own burst, or a second's
        worth of a continuous one like fire).
        """
        if hasattr(position, "transform"):
            position = position.transform.position
        pos = _vec(tuple(position)[:3], 3)
        d = None
        if direction is not None:
            d = _vec(tuple(direction)[:3], 3)
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 1e-12:
                d = None
        name, base = self._effect(effect)
        base.update({k: _particle_value(v) for k, v in fields.items()})
        for k in base:
            if k not in PARTICLE_FIELDS:
                raise KeyError(f"ParticleEmitter has no field {k!r}")
        n = None if count is None else max(0, int(count))
        if n == 0:
            return
        self._host.backend.burst(name, base, pos, d, n)

    def emit(self, entity: Entity | str, count: int) -> None:
        """``count`` extra particles from ``entity``'s own emitter (its
        Particle Emitter component), on top of its rate."""
        n = max(0, int(count))
        if n:
            self._host.backend.burst_emitter(getattr(entity, "id", entity), n)

    @staticmethod
    def _effect(effect):
        if isinstance(effect, str):
            name = effect.lower()
            if name not in PARTICLE_PRESETS:
                raise ValueError(
                    f"unknown particle preset {effect!r}; expected one of "
                    + ", ".join(PARTICLE_PRESETS)
                )
            return name, {}
        preset = getattr(effect, "preset_name", None)
        if preset:
            return preset, {}
        if hasattr(effect, "rate"):
            return None, {k: _particle_value(getattr(effect, k)) for k in PARTICLE_FIELDS}
        raise TypeError("burst() wants a preset name or an at.ParticleEmitter")

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            return self._host.backend.extra("particles", name)


class Audio:
    """``ctx.audio``: one-shot sounds by file name."""

    def __init__(self, host):
        self._host = host
        self._queue = []

    def play(
        self,
        path: str,
        volume: float = 1.0,
        pitch: float = 1.0,
        position: Iterable[float] | None = None,
    ) -> None:
        """Plays a sound file once (relative to the scene folder). With a
        ``position`` (a point or an entity) it fades with distance."""
        if hasattr(position, "transform"):
            position = position.transform.position
        pos = list(_vec(tuple(position)[:3], 3)) if position is not None else None
        self._queue.append([str(path), float(volume), float(pitch), pos])

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            return self._host.backend.extra("audio", name)


class Hud:
    """``ctx.hud``: draw images and rectangles over the game this frame.

    Coordinates are screen pixels from the top-left; shapes are placed by
    their centre. Call every frame you want them shown.
    """

    def __init__(self):
        self._items = []
        self.size = (1280, 720)

    @property
    def center(self) -> tuple[float, float]:
        return (self.size[0] / 2.0, self.size[1] / 2.0)

    def image(
        self,
        path: str,
        x: float,
        y: float,
        size: float | tuple[float, float] | None = None,
        color: tuple[float, float, float, float] = (1.0, 1.0, 1.0, 1.0),
        rotation: float = 0.0,
    ) -> None:
        item = {"k": "img", "path": str(path), "x": float(x), "y": float(y),
                "c": list(_vec(color, 4)), "rot": float(rotation)}
        if size is not None:
            item["w"], item["h"] = float(size[0]), float(size[1])
        self._items.append(item)

    def rect(
        self,
        x: float,
        y: float,
        width: float,
        height: float,
        color: tuple[float, float, float, float] = (1.0, 1.0, 1.0, 1.0),
    ) -> None:
        self._items.append({"k": "rect", "x": float(x), "y": float(y), "w": float(width),
                            "h": float(height), "c": list(_vec(color, 4))})

    def text(
        self,
        text: str,
        x: float,
        y: float,
        size: float = 24.0,
        color: tuple[float, float, float, float] = (1.0, 1.0, 1.0, 1.0),
        align: str = "left",
        shadow: Any = None,
        outline: Any = None,
    ) -> None:
        """Draws ``text`` with the built-in font this frame.

        ``size`` is the line height in pixels. ``align`` says where ``x``
        sits on each line: ``"left"`` (its left edge), ``"center"`` or
        ``"right"``. Lines (split on ``\\n``) are centred vertically on
        ``y``. ``shadow`` and ``outline`` take a colour, or True for
        translucent / solid black.
        """
        a = str(align).lower()
        if a == "centre":
            a = "center"
        if a not in ("left", "center", "right"):
            raise ValueError(f'align must be "left", "center" or "right", not {align!r}')
        size = float(size)
        if not (size > 0 and math.isfinite(size)):
            raise ValueError("text size must be a positive number")
        item = {"k": "text", "text": str(text), "x": float(x), "y": float(y), "size": size,
                "c": list(_vec(color, 4)), "align": a}
        for key, value, default in (("shadow", shadow, (0.0, 0.0, 0.0, 0.6)),
                                    ("outline", outline, (0.0, 0.0, 0.0, 1.0))):
            if value is True:
                value = default
            if value is not None and value is not False:
                item[key] = list(_vec(value, 4))
        self._items.append(item)


class SceneView:
    """``ctx.scene``: entities by name, by id, or all of them."""

    def __init__(self, host):
        self._host = host

    def find(self, name: str) -> Entity | None:
        """The entity with this name, or None."""
        return self._host._by_name.get(name)

    def __getitem__(self, name):
        e = self.find(name)
        if e is None:
            raise KeyError(f"no entity named {name!r}")
        return e

    def __iter__(self):
        return iter(list(self._host.entities.values()))

    def __len__(self):
        return len(self._host.entities)

    def __contains__(self, name):
        return name in self._host._by_name

    def names(self) -> list[str]:
        return [e.name for e in self._host.entities.values()]

    def by_id(self, entity_id: int) -> Entity | None:
        """The entity with this id (as in ``Hit.entity.id``), or None."""
        return self._host.entities.get(entity_id)

    @property
    def entity_count(self) -> int:
        return len(self._host.entities)

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            return self._host.backend.extra("scene", name)


class Message:
    """What a channel subscriber receives.

    ``channel`` is the full channel name, ``data`` what was published and
    ``ctx`` the usual ``ctx``. Animation events also set ``entity`` (whose
    animation it was), ``event`` (the event's name) and ``clip``.
    """

    __slots__ = ("channel", "data", "entity", "event", "clip", "ctx")

    channel: str
    data: Any
    entity: Entity | None
    event: str | None
    clip: str | None
    ctx: Context | None

    def __init__(self, channel, data=None, entity=None, event=None, clip=None):
        self.channel = channel
        self.data = data
        self.entity = entity
        self.event = event
        self.clip = clip
        self.ctx = None

    def __repr__(self):
        extra = f", data={self.data!r}" if self.data is not None else ""
        return f"Message({self.channel!r}{extra})"


def _fn_key(fn):
    """What makes two subscriptions the same function: plain functions by
    where they're written (so subscribing again after a hot reload, or from
    ``update`` every frame, doesn't add another), anything else by
    equality."""
    code = getattr(fn, "__code__", None)
    if code is not None and isinstance(fn, types.FunctionType) and fn.__closure__ is None:
        return (fn.__module__, fn.__qualname__, code.co_firstlineno)
    return fn


class Subscription:
    """A ``ctx.subscribe`` registration; ``cancel()`` ends it."""

    __slots__ = ("pattern", "fn", "key", "script", "owner", "active", "_host", "_wild")

    def __init__(self, host, pattern, fn, script, owner):
        self._host = host
        self.pattern = pattern
        self.fn = fn
        self.key = _fn_key(fn)
        self.script = script
        self.owner = owner
        self.active = True
        self._wild = any(c in pattern for c in "*?[")

    def matches(self, channel: str) -> bool:
        if self._wild:
            return fnmatch.fnmatchcase(channel, self.pattern)
        return channel == self.pattern

    def cancel(self) -> None:
        """Stops delivering to this subscriber."""
        if self.active:
            self.active = False
            self._host._subs = [s for s in self._host._subs if s is not self]

    def __repr__(self):
        name = getattr(self.fn, "__qualname__", repr(self.fn))
        return f"Subscription({self.pattern!r}, {name})"



# --------------------------------------------------------------------------
# Keys: ctx.get / set / scan / keys / ref
# --------------------------------------------------------------------------


def _wild(text):
    return any(c in text for c in "*?[")


def _did_you_mean(word, options):
    near = difflib.get_close_matches(word, sorted(set(options)), n=1, cutoff=0.6)
    return f"; did you mean {near[0]!r}?" if near else ""


def _play_clip(anim, clip):
    anim.play(clip)


def _set_attr(anim, name, value):
    setattr(anim, name, value)


# component -> (handle getter, what's missing, {field: (getter, setter or None)})
_ANIMATION_FIELDS = {
    "clip": (lambda a: a.clip, _play_clip),
    "time": (lambda a: a.time, lambda a, v: _set_attr(a, "time", v)),
    "speed": (lambda a: a.speed, lambda a, v: _set_attr(a, "speed", v)),
    "root_motion": (lambda a: a.root_motion, lambda a, v: _set_attr(a, "root_motion", v)),
    "playing": (lambda a: a.playing, None),
    "looping": (lambda a: a.looping, None),
    "duration": (lambda a: a.duration, None),
    "clips": (lambda a: a.clips, None),
    "springs": (lambda a: a.springs, None),
}
_TRANSFORM_FIELDS = {
    "position": (lambda t: t.position, lambda t, v: _set_attr(t, "position", v)),
    "rotation": (lambda t: t.rotation_euler, lambda t, v: _set_attr(t, "rotation_euler", v)),
    "scale": (lambda t: t.scale, lambda t, v: _set_attr(t, "scale", v)),
}
_FIELD_ALIASES = {("transform", "rotation_euler"): "rotation"}
COMPONENTS = {
    "animation": (lambda e: e.animation, "no Animation (or its model isn't animated)",
                  _ANIMATION_FIELDS),
    "transform": (lambda e: e.transform, "no Transform", _TRANSFORM_FIELDS),
    "attr": (lambda e: e.attributes, "no attributes", None),  # fields: its attributes
}


class FieldRef:
    """``ctx.ref("Player:animation:speed")``: one value, looked up again on
    every read. ``value`` (or ``get()`` / ``set(v)``) reads and writes it."""

    __slots__ = ("key", "_store", "_entity", "_component", "_field")

    def __init__(self, store, key, entity, component, field):
        self._store = store
        self.key = key
        self._entity = entity
        self._component = component
        self._field = field

    def get(self) -> Any:
        return self._store._read_field(self.key, self._entity, self._component, self._field)

    def set(self, value: Any) -> None:
        self._store._write_field(self.key, self._entity, self._component, self._field, value)

    @property
    def value(self) -> Any:
        return self.get()

    @value.setter
    def value(self, v: Any) -> None:
        self.set(v)

    def __repr__(self):
        return f"FieldRef({self.key!r})"


class ComponentRef:
    """``ctx.ref("Player:animation")``: the component's handle, looked up
    again on every use, so it's safe to keep from ``start``. Use it like
    the handle itself (``ref.play("Run")``, ``ref.position``)."""

    __slots__ = ("key", "_store", "_entity", "_component")

    def __init__(self, store, key, entity, component):
        object.__setattr__(self, "_store", store)
        object.__setattr__(self, "key", key)
        object.__setattr__(self, "_entity", entity)
        object.__setattr__(self, "_component", component)

    def resolve(self) -> Any:
        """The component's handle this frame."""
        return self._store._component_of(self.key, self._entity, self._component)

    def __getattr__(self, name):
        return getattr(self.resolve(), name)

    def __setattr__(self, name, value):
        setattr(self.resolve(), name, value)

    def __getitem__(self, name):
        return self.resolve()[name]

    def __setitem__(self, name, value):
        self.resolve()[name] = value

    def __repr__(self):
        return f"ComponentRef({self.key!r})"


class KeyStore:
    """Engine state by key, Redis style: ``entity:component:field``.

    Components are ``animation``, ``transform`` and ``attr``::

        ctx.get("Player:animation:clip")           # "Run"
        ctx.set("Player:animation:speed", 1.5)
        ctx.get("Player:attr:health")
        ctx.get("Player:transform:position")
        for anim in ctx.scan("Enemy_*:animation"):
            anim.play("Dance", fade=0.3)
        speed = ctx.ref("Player:animation:speed")   # keep it from start()

    ``get`` / ``set`` / ``ref`` take one exact key; ``scan`` and ``keys``
    take ``*`` and ``?`` wildcards. A name two entities share is an error
    for ``get`` (``scan`` returns both), and a wrong key's error suggests
    the nearest right one.
    """

    def __init__(self, host):
        self._host = host
        self._parsed = {}

    # --- parsing and lookup -----------------------------------------------

    def _split(self, key, exact):
        parts = self._parsed.get(key)
        if parts is None:
            if not isinstance(key, str):
                raise TypeError(f"keys are strings like 'Player:animation:clip', not {key!r}")
            parts = tuple(key.split(":"))
            if len(parts) > 3 or not all(parts):
                raise KeyError(
                    f"{key!r} isn't a key: keys are 'entity', 'entity:component' or "
                    f"'entity:component:field' (components: {', '.join(COMPONENTS)})")
            if len(self._parsed) < 4096:
                self._parsed[key] = parts
        if exact and _wild(key):
            raise ValueError(f"{key!r} has a wildcard; get, set and ref take one exact key "
                             "(use ctx.scan or ctx.keys for patterns)")
        return parts

    def _entity(self, key, name):
        host = self._host
        count = host._name_count.get(name, 0)
        if count == 1:
            found = host._by_name.get(name)
            if found is None:  # the first of two was despawned
                found = next(e for e in host.entities.values() if e.name == name)
                host._by_name[name] = found
            return found
        if count > 1:
            raise KeyError(f"{key!r}: {count} entities are called {name!r}; give them "
                           "different names, or use ctx.scan to get them all")
        names = [n for n in host._name_count if n]
        raise KeyError(f"{key!r}: no entity is called {name!r}{_did_you_mean(name, names)}")

    def _component_info(self, key, component):
        info = COMPONENTS.get(component)
        if info is None:
            raise KeyError(f"{key!r}: there's no component {component!r}"
                           f"{_did_you_mean(component, COMPONENTS)} "
                           f"(keys know {', '.join(COMPONENTS)})")
        return info

    def _field_name(self, key, component, field, fields):
        field = _FIELD_ALIASES.get((component, field), field)
        if fields is not None and field not in fields:
            raise KeyError(f"{key!r}: {component} has no field {field!r}"
                           f"{_did_you_mean(field, fields)} (it has {', '.join(fields)})")
        return field

    def _component_of(self, key, entity, component):
        if self._host.entities.get(entity.id) is not entity:
            raise KeyError(f"{key!r}: {entity.name!r} is gone")
        getter, missing, _ = COMPONENTS[component]
        handle = getter(entity)
        if handle is None:
            raise KeyError(f"{key!r}: {entity.name!r} has {missing}")
        return handle

    def _read_field(self, key, entity, component, field):
        handle = self._component_of(key, entity, component)
        fields = COMPONENTS[component][2]
        if fields is None:
            if field not in handle:
                raise KeyError(f"{key!r}: {entity.name!r} has no attribute {field!r}"
                               f"{_did_you_mean(field, handle)}")
            return handle[field]
        return fields[field][0](handle)

    def _write_field(self, key, entity, component, field, value):
        handle = self._component_of(key, entity, component)
        fields = COMPONENTS[component][2]
        if fields is None:
            if field not in handle:
                raise KeyError(f"{key!r}: {entity.name!r} has no attribute {field!r}"
                               f"{_did_you_mean(field, handle)} (add new ones with "
                               "entity.attributes[...])")
            handle[field] = value
            return
        setter = fields[field][1]
        if setter is None:
            writable = [f for f, (_, s) in fields.items() if s is not None]
            raise AttributeError(f"{key!r} is read-only (you can set {', '.join(writable)})")
        setter(handle, value)

    def _resolve(self, key):
        """(entity, component or None, field or None) for an exact key."""
        parts = self._split(key, exact=True)
        entity = self._entity(key, parts[0])
        if len(parts) == 1:
            return entity, None, None
        component = parts[1]
        fields = self._component_info(key, component)[2]
        if len(parts) == 2:
            return entity, component, None
        return entity, component, self._field_name(key, component, parts[2], fields)

    # --- the API -----------------------------------------------------------

    def get(self, key: str) -> Any:
        entity, component, field = self._resolve(key)
        if component is None:
            return entity
        if field is None:
            return self._component_of(key, entity, component)
        return self._read_field(key, entity, component, field)

    def set(self, key: str, value: Any) -> None:
        entity, component, field = self._resolve(key)
        if field is None:
            raise KeyError(f"{key!r}: set needs a field, like '{key}:"
                           f"{'speed' if component == 'animation' else 'position'}'"
                           if component else f"{key!r}: set needs 'entity:component:field'")
        self._write_field(key, entity, component, field, value)

    def ref(self, key: str) -> Entity | ComponentRef | FieldRef:
        entity, component, field = self._resolve(key)
        if component is None:
            return entity  # entity handles already last the whole run
        self._component_of(key, entity, component)  # fail now, not later
        if field is None:
            return ComponentRef(self, key, entity, component)
        return FieldRef(self, key, entity, component, field)

    def _matches(self, pattern):
        """(key, entity, component, field) for every key ``pattern`` matches,
        in scene order. Exact segments that can't match anything raise."""
        parts = self._split(pattern, exact=False)
        name_pat = parts[0]
        if not _wild(name_pat) and name_pat not in self._host._name_count:
            self._entity(pattern, name_pat)  # raises with a suggestion
        comp_pat = parts[1] if len(parts) > 1 else None
        field_pat = parts[2] if len(parts) > 2 else None
        if comp_pat is not None and not _wild(comp_pat):
            fields = self._component_info(pattern, comp_pat)[2]
            if field_pat is not None and not _wild(field_pat):
                field_pat = self._field_name(pattern, comp_pat, field_pat, fields)
        out = []
        for e in self._host.entities.values():
            if not e.name or not fnmatch.fnmatchcase(e.name, name_pat):
                continue
            if comp_pat is None:
                out.append((e.name, e, None, None))
                continue
            for comp, (getter, _, fields) in COMPONENTS.items():
                if not fnmatch.fnmatchcase(comp, comp_pat):
                    continue
                handle = getter(e)
                if handle is None:
                    continue
                if field_pat is None:
                    out.append((f"{e.name}:{comp}", e, comp, None))
                    continue
                names = list(handle) if fields is None else list(fields)
                for f in names:
                    if fnmatch.fnmatchcase(f, field_pat):
                        out.append((f"{e.name}:{comp}:{f}", e, comp, f))
        if not out and field_pat is not None and not _wild(field_pat) and comp_pat == "attr":
            seen = set()
            for e in self._host.entities.values():
                if e.name and fnmatch.fnmatchcase(e.name, name_pat):
                    seen.update(e.attributes)
            hint = _did_you_mean(field_pat, seen)
            if hint:
                raise KeyError(f"{pattern!r}: no match has attribute {field_pat!r}{hint}")
        return out

    def keys(self, pattern: str = "*") -> list[str]:
        return [k for k, *_ in self._matches(pattern)]

    def scan(self, pattern: str) -> dict[str, Any]:
        out = []
        for key, entity, component, field in self._matches(pattern):
            if component is None:
                out.append(entity)
            elif field is None:
                out.append(self._component_of(key, entity, component))
            else:
                out.append(self._read_field(key, entity, component, field))
        return out


# --------------------------------------------------------------------------
# Easing, tweens and timers: ctx.tween / tween_value / after / every
# --------------------------------------------------------------------------


def _power(p):
    def ease_in(t):
        return t ** p

    def ease_out(t):
        return 1.0 - (1.0 - t) ** p

    def ease_in_out(t):
        return 2.0 ** (p - 1) * t ** p if t < 0.5 else 1.0 - (-2.0 * t + 2.0) ** p / 2.0

    return ease_in, ease_out, ease_in_out


def _expo_in(t):
    return 0.0 if t <= 0.0 else 2.0 ** (10.0 * t - 10.0)


def _expo_out(t):
    return 1.0 if t >= 1.0 else 1.0 - 2.0 ** (-10.0 * t)


def _expo_in_out(t):
    if t <= 0.0 or t >= 1.0:
        return min(max(t, 0.0), 1.0)
    return 2.0 ** (20.0 * t - 10.0) / 2.0 if t < 0.5 else (2.0 - 2.0 ** (-20.0 * t + 10.0)) / 2.0


def _circ_in(t):
    return 1.0 - math.sqrt(max(0.0, 1.0 - t * t))


def _circ_out(t):
    return math.sqrt(max(0.0, 1.0 - (t - 1.0) ** 2))


def _circ_in_out(t):
    if t < 0.5:
        return (1.0 - math.sqrt(max(0.0, 1.0 - (2.0 * t) ** 2))) / 2.0
    return (math.sqrt(max(0.0, 1.0 - (-2.0 * t + 2.0) ** 2)) + 1.0) / 2.0


_BACK = 1.70158


def _back_in(t):
    return (_BACK + 1.0) * t ** 3 - _BACK * t * t


def _back_out(t):
    return 1.0 + (_BACK + 1.0) * (t - 1.0) ** 3 + _BACK * (t - 1.0) ** 2


def _back_in_out(t):
    c = _BACK * 1.525
    if t < 0.5:
        return (2.0 * t) ** 2 * ((c + 1.0) * 2.0 * t - c) / 2.0
    return ((2.0 * t - 2.0) ** 2 * ((c + 1.0) * (2.0 * t - 2.0) + c) + 2.0) / 2.0


def _elastic_in(t):
    if t <= 0.0 or t >= 1.0:
        return min(max(t, 0.0), 1.0)
    return -(2.0 ** (10.0 * t - 10.0)) * math.sin((10.0 * t - 10.75) * math.tau / 3.0)


def _elastic_out(t):
    if t <= 0.0 or t >= 1.0:
        return min(max(t, 0.0), 1.0)
    return 2.0 ** (-10.0 * t) * math.sin((10.0 * t - 0.75) * math.tau / 3.0) + 1.0


def _elastic_in_out(t):
    if t <= 0.0 or t >= 1.0:
        return min(max(t, 0.0), 1.0)
    s = math.sin((20.0 * t - 11.125) * math.tau / 4.5)
    if t < 0.5:
        return -(2.0 ** (20.0 * t - 10.0)) * s / 2.0
    return 2.0 ** (-20.0 * t + 10.0) * s / 2.0 + 1.0


def _bounce_out(t):
    n, d = 7.5625, 2.75
    if t < 1.0 / d:
        return n * t * t
    if t < 2.0 / d:
        t -= 1.5 / d
        return n * t * t + 0.75
    if t < 2.5 / d:
        t -= 2.25 / d
        return n * t * t + 0.9375
    t -= 2.625 / d
    return n * t * t + 0.984375


def _bounce_in(t):
    return 1.0 - _bounce_out(1.0 - t)


def _bounce_in_out(t):
    return (1.0 - _bounce_out(1.0 - 2.0 * t)) / 2.0 if t < 0.5 else (1.0 + _bounce_out(2.0 * t - 1.0)) / 2.0


def _sine_in(t):
    return 1.0 - math.cos(t * math.pi / 2.0)


def _sine_out(t):
    return math.sin(t * math.pi / 2.0)


def _sine_in_out(t):
    return -(math.cos(math.pi * t) - 1.0) / 2.0


def _easing_table():
    table = {"linear": lambda t: t}
    families = {
        "sine": (_sine_in, _sine_out, _sine_in_out),
        "quad": _power(2),
        "cubic": _power(3),
        "quart": _power(4),
        "quint": _power(5),
        "expo": (_expo_in, _expo_out, _expo_in_out),
        "circ": (_circ_in, _circ_out, _circ_in_out),
        "back": (_back_in, _back_out, _back_in_out),
        "elastic": (_elastic_in, _elastic_out, _elastic_in_out),
        "bounce": (_bounce_in, _bounce_out, _bounce_in_out),
    }
    for family, (ease_in, ease_out, ease_in_out) in families.items():
        table[f"in_{family}"] = ease_in
        table[f"out_{family}"] = ease_out
        table[f"in_out_{family}"] = ease_in_out
    return table


#: The easing curves by name: ``"linear"``, and ``in_``, ``out_`` and
#: ``in_out_`` versions of ``sine``, ``quad``, ``cubic``, ``quart``,
#: ``quint``, ``expo``, ``circ``, ``back``, ``elastic`` and ``bounce``
#: (31 in all, the standard Penner set; see easings.net). Each maps 0 to 0
#: and 1 to 1; ``back`` and ``elastic`` overshoot on the way.
EASINGS: dict[str, Callable[[float], float]] = _easing_table()


def _ease_name(name):
    """``"easeOutCubic"``, ``"ease-out-cubic"`` and ``"OutCubic"`` all mean
    ``"out_cubic"``."""
    out = []
    for i, c in enumerate(name.strip()):
        if c.isupper() and i and out and out[-1] != "_":
            out.append("_")
        out.append("_" if c in "- " else c.lower())
    n = "".join(out)
    if n.startswith("ease_"):
        n = n[5:]
    elif n.startswith("ease") and n != "ease":
        n = n[4:]
    n = n.replace("inout", "in_out")
    return n


def easing(ease: str | Callable[[float], float]) -> Callable[[float], float]:
    """The curve called ``ease`` (see :data:`EASINGS`), or ``ease`` itself
    if it's already a function of ``t``. Raises ``ValueError`` with the
    nearest name for a misspelt one."""
    if callable(ease):
        return ease
    if not isinstance(ease, str):
        raise TypeError(f"ease is a curve name like 'out_cubic' or a function, not {ease!r}")
    fn = EASINGS.get(ease) or EASINGS.get(_ease_name(ease))
    if fn is None:
        raise ValueError(f"there's no easing curve {ease!r}{_did_you_mean(_ease_name(ease), EASINGS)} "
                         "(curves: linear, and in_/out_/in_out_ plus sine, quad, cubic, quart, "
                         "quint, expo, circ, back, elastic or bounce)")
    return fn


def ease(name: str | Callable[[float], float], t: float) -> float:
    """``t`` (0 to 1, clamped) through the easing curve ``name``:
    ``ease("out_cubic", 0.5)`` is 0.875."""
    return float(easing(name)(min(max(float(t), 0.0), 1.0)))


def _numbers(value, what):
    """``value`` as a float or a tuple of floats, for tweening."""
    if isinstance(value, bool) or isinstance(value, str):
        raise TypeError(f"can't tween {what} {value!r}: tweens move numbers, or tuples of numbers")
    if isinstance(value, (int, float)):
        return value
    try:
        items = tuple(value)
    except TypeError:
        raise TypeError(f"can't tween {what} {value!r}: tweens move numbers, or tuples of "
                        "numbers") from None
    for x in items:
        if isinstance(x, bool) or not isinstance(x, (int, float)):
            raise TypeError(f"can't tween {what} {value!r}: tweens move numbers, or tuples "
                            "of numbers")
    return items


def _lerp(a, b, t):
    if isinstance(a, tuple):
        return tuple(x + (y - x) * t for x, y in zip(a, b))
    v = a + (b - a) * t
    if isinstance(a, int) and isinstance(b, int):
        return int(round(v))
    return v


def _check_shapes(start, end, what):
    if isinstance(start, tuple) != isinstance(end, tuple) or (
            isinstance(start, tuple) and len(start) != len(end)):
        raise ValueError(f"can't tween {what} from {start!r} to {end!r}: they need the same shape")


def _live_fn(script, fn):
    """``fn``, or its latest version if it's a top-level function of
    ``script`` that a hot reload replaced."""
    if (script is not None and script.module is not None and isinstance(fn, types.FunctionType)
            and fn.__qualname__ == fn.__name__ and fn.__module__ == script.module.__name__):
        return getattr(script.module, fn.__name__, fn)
    return fn


_TRANSFORM_SHORTCUTS = {"position": "position", "rotation": "rotation",
                        "rotation_euler": "rotation", "scale": "scale"}


class _Scheduled:
    """What timers and tweens share: an owner, pausing and cancelling."""

    __slots__ = ("active", "paused", "script", "owner", "_host")

    def __init__(self, host):
        self._host = host
        self.script, self.owner = host._running or (None, None)
        self.active = True
        self.paused = False

    def cancel(self) -> None:
        """Stops it now, without calling anything."""
        self.active = False

    def pause(self) -> None:
        """Holds it where it is until :meth:`resume`."""
        self.paused = True

    def resume(self) -> None:
        self.paused = False

    @property
    def done(self) -> bool:
        """It finished or was cancelled."""
        return not self.active

    def _call(self, fn, args, during):
        """Runs a callback as its script, reporting errors like the script's."""
        script = self.script
        fn = _live_fn(script, fn)
        host = self._host
        outer = host._running
        if script is not None:
            host._running = (script, self.owner)
        try:
            fn(*args)
        except BaseException as exc:  # noqa: BLE001 - user code
            if script is None:
                raise
            script.broken = True
            who = self.owner.name if self.owner is not None else "?"
            host._failed(exc, script.path, f"{during} ({who})")
        finally:
            host._running = outer


class Timer(_Scheduled):
    """What ``ctx.after`` and ``ctx.every`` return.

    ``cancel()`` stops it; ``pause()`` and ``resume()`` hold it.
    ``remaining`` is the seconds until it next fires, ``count`` how many
    times it has, and ``done`` whether it's finished or cancelled.
    """

    __slots__ = ("interval", "remaining", "times", "count", "fn", "args", "repeat")

    def __init__(self, host, interval, fn, args, repeat, times):
        super().__init__(host)
        self.interval = interval
        self.remaining = interval
        self.repeat = repeat
        self.times = times
        self.count = 0
        self.fn = fn
        self.args = args

    def _tick(self, dt):
        self.remaining -= dt
        fired = 0
        while self.active and self.remaining <= 1e-9:
            self.count += 1
            if not self.repeat or (self.times is not None and self.count >= self.times):
                self.active = False
            self._call(self.fn, self.args, "every" if self.repeat else "after")
            if not self.active or (self.script is not None and self.script.broken):
                return
            self.remaining += self.interval
            fired += 1
            if fired >= 8:  # a long hitch: don't fire a backlog of hundreds
                self.remaining = max(self.remaining, self.interval)

    def __repr__(self):
        name = getattr(self.fn, "__qualname__", repr(self.fn))
        kind = "every" if self.repeat else "after"
        state = "done" if not self.active else f"in {self.remaining:.2f}s"
        return f"Timer({kind} {self.interval:g}s, {name}, {state})"


class Tween(_Scheduled):
    """What ``ctx.tween`` and ``ctx.tween_value`` return.

    ``value`` is where it is now, ``progress`` how far through it is (0 to
    1 over the whole run, loops included, or of the current play if it
    loops forever). ``cancel()`` stops it where it is, ``finish()`` jumps
    to the end (calling ``on_done``), ``pause()`` / ``resume()`` hold it,
    and ``done`` says whether it's over.
    """

    __slots__ = ("key", "start", "end", "duration", "curve", "delay", "plays", "yoyo",
                 "on_update", "on_done", "value", "elapsed", "_entity", "_component", "_field",
                 "_began", "_fixed_start")

    def __init__(self, host, entity, component, field, start, end, duration, curve, delay,
                 plays, yoyo, on_update, on_done, key):
        super().__init__(host)
        self.key = key
        self._entity = entity
        self._component = component
        self._field = field
        self._fixed_start = start is not None
        self.start = start
        self.end = end
        self.duration = duration
        self.curve = curve
        self.delay = delay
        self.plays = plays
        self.yoyo = yoyo
        self.on_update = on_update
        self.on_done = on_done
        self.elapsed = 0.0
        self.value = start
        self._began = False

    @property
    def progress(self) -> float:
        t = max(0.0, self.elapsed - self.delay)
        if self.duration <= 0.0:
            return 1.0 if self._began else 0.0
        if self.plays is None:
            return (t / self.duration) % 1.0
        return min(1.0, t / (self.duration * self.plays))

    def _target_alive(self):
        e = self._entity
        return e is None or self._host.entities.get(e.id) is e

    def _read(self):
        store = self._host.ctx._keys
        return _numbers(store._read_field(self.key, self._entity, self._component, self._field),
                        f"{self.key!r}, which is")

    def _apply(self, value):
        self.value = value
        if self._entity is not None:
            self._host.ctx._keys._write_field(self.key, self._entity, self._component,
                                              self._field, value)
        if self.on_update is not None:
            self._call(self.on_update, (value,), "tween on_update")

    def _tick(self, dt):
        if not self._target_alive():
            self.active = False
            return
        self.elapsed += dt
        t = self.elapsed - self.delay
        if t < 0.0:
            return
        if not self._began:
            self._began = True
            if not self._fixed_start:
                self.start = self._read()
                _check_shapes(self.start, self.end, self.key)
        if self.duration <= 0.0:
            leg, frac = (self.plays or 1) - 1, 1.0
        else:
            leg = int(t // self.duration)
            frac = t / self.duration - leg
        plays = self.plays
        finished = plays is not None and leg >= plays
        if plays is not None and finished:
            leg, frac = plays - 1, 1.0
        x = 1.0 - frac if self.yoyo and leg % 2 == 1 else frac
        self._apply(_lerp(self.start, self.end, self.curve(x)))
        if finished and self.active:
            self._end()

    def _end(self):
        self.active = False
        if self.on_done is not None:
            self._call(self.on_done, (), "tween on_done")

    def finish(self) -> None:
        """Jumps to where it ends and calls ``on_done``."""
        if not self.active:
            return
        if not self._began:
            self._began = True
            if not self._fixed_start and self._target_alive():
                self.start = self._read()
        backwards = self.yoyo and self.plays is not None and self.plays % 2 == 0
        if self._target_alive():
            self._apply(self.start if backwards else self.end)
        self._end()

    def __repr__(self):
        what = self.key or "value"
        state = "done" if not self.active else f"{self.progress:.0%}"
        return f"Tween({what} -> {self.end!r} over {self.duration:g}s, {state})"


# --------------------------------------------------------------------------
# Save games and player settings: ctx.save / ctx.settings
# --------------------------------------------------------------------------

#: The key names both runners know, in the order
#: ``ctx.input.any_just_pressed()`` checks them.
KEY_NAMES = (
    tuple("abcdefghijklmnopqrstuvwxyz")
    + tuple("0123456789")
    + ("space", "enter", "escape", "tab", "backspace",
       "arrow_left", "arrow_right", "arrow_up", "arrow_down",
       "shift_left", "shift_right", "ctrl_left", "ctrl_right", "alt_left", "alt_right")
    + tuple(f"f{i}" for i in range(1, 13))
)
#: Mouse buttons as bindings: ``"mouse_left"``, ``"mouse_right"``,
#: ``"mouse_middle"``.
MOUSE_BINDINGS = {"mouse_left": "left", "mouse_right": "right", "mouse_middle": "middle"}
#: The graphics presets, lowest first.
GRAPHICS_PRESETS = ("low", "medium", "high", "ultra", "beautiful")

_SAVE_FORMAT = 1


def user_data_dir(game: str) -> str:
    """Where an exported game keeps its saves and settings: a folder named
    after ``game`` in the platform's user-data folder (``%APPDATA%`` on
    Windows, ``~/Library/Application Support`` on macOS,
    ``$XDG_DATA_HOME`` or ``~/.local/share`` elsewhere).
    ``$AUTOMATA_SAVE_DIR`` overrides it."""
    override = os.environ.get("AUTOMATA_SAVE_DIR")
    if override:
        return os.path.abspath(override)
    name = "".join(c for c in str(game) if c not in '/\\:*?"<>|' and ord(c) >= 32)
    name = name.strip().rstrip(".") or "AUTOMATA game"
    if sys.platform == "win32":
        base = os.environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), "AppData",
                                                         "Roaming")
    elif sys.platform == "darwin":
        base = os.path.join(os.path.expanduser("~"), "Library", "Application Support")
    elif os.environ.get("ANDROID_PRIVATE"):
        base = os.environ["ANDROID_PRIVATE"]
    else:
        base = os.environ.get("XDG_DATA_HOME") or ""
        if not os.path.isabs(base):
            base = os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, name)


def dev_save_dir(project: str) -> str:
    """Where saves go while developing (Oberon's Play mode and
    ``run_scene``): ``<project>/.oberon/save``, kept out of exports.
    ``$AUTOMATA_SAVE_DIR`` overrides it."""
    override = os.environ.get("AUTOMATA_SAVE_DIR")
    if override:
        return os.path.abspath(override)
    return os.path.join(os.path.abspath(project), ".oberon", "save")


def _plain(value, where):
    """A JSON-safe copy of ``value`` (tuples become lists), or TypeError."""
    if value is None or isinstance(value, (bool, str)):
        return value
    if isinstance(value, int):
        return int(value)
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError(f"{where}: {value!r} can't be saved (only finite numbers)")
        return value
    if isinstance(value, (list, tuple)):
        return [_plain(v, f"{where}[{i}]") for i, v in enumerate(value)]
    if isinstance(value, dict):
        out = {}
        for k, v in value.items():
            if not isinstance(k, str):
                raise TypeError(f"{where}: dictionary keys must be strings to save, not {k!r}")
            out[k] = _plain(v, f"{where}[{k!r}]")
        return out
    raise TypeError(f"{where}: can't save {type(value).__name__} {value!r}; saves hold numbers, "
                    "strings, True/False/None, and lists and dicts of them")


class _Store:
    """JSON files in one folder, written atomically; or nothing at all
    (``folder=None``: kept in memory, for tests)."""

    def __init__(self, folder, log):
        self.folder = folder
        self._log = log

    def path(self, rel):
        return None if self.folder is None else os.path.join(self.folder, rel)

    def read(self, rel):
        path = self.path(rel)
        if path is None or not os.path.isfile(path):
            return None
        try:
            with open(path, encoding="utf-8") as f:
                return json.load(f)
        except (OSError, ValueError) as exc:
            broken = path + ".broken"
            try:
                os.replace(path, broken)
                where = f"; kept it as {os.path.basename(broken)}"
            except OSError:
                where = ""
            self._log("warn", f"couldn't read {path} ({exc}); starting fresh{where}")
            return None

    def write(self, rel, data):
        path = self.path(rel)
        if path is None:
            return
        os.makedirs(os.path.dirname(path), exist_ok=True)
        tmp = f"{path}.{os.getpid()}.tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=1, sort_keys=True, ensure_ascii=False)
            f.write("\n")
        os.replace(tmp, path)

    def remove(self, rel):
        path = self.path(rel)
        if path is not None and os.path.isfile(path):
            os.remove(path)

    def names(self, sub, suffix):
        path = self.path(sub)
        if path is None or not os.path.isdir(path):
            return []
        return sorted(n[: -len(suffix)] for n in os.listdir(path) if n.endswith(suffix))


def _slot_name(slot):
    if isinstance(slot, bool) or not isinstance(slot, (str, int)):
        raise TypeError(f"a save slot is a name or a number, not {slot!r}")
    name = str(slot).strip()
    if not name or len(name) > 64 or any(not (c.isalnum() or c in " _-") for c in name):
        raise ValueError(f"save slot {slot!r}: use up to 64 letters, digits, spaces, '_' or '-'")
    return name


class SaveSlot:
    """One save slot as ``ctx.save.slots()`` lists it: ``name``,
    ``saved_at`` (seconds since 1970, as ``time.time()``) and ``keys``
    (how many values it holds)."""

    __slots__ = ("name", "saved_at", "keys")

    def __init__(self, name, saved_at, keys):
        self.name = name
        self.saved_at = saved_at
        self.keys = keys

    def __repr__(self):
        return f"SaveSlot({self.name!r}, saved_at={self.saved_at!r}, keys={self.keys})"


class SaveData:
    """``ctx.save``: the current save slot, a dict that lasts between runs::

        ctx.save["coins"] = ctx.save.get("coins", 0) + 1
        if "checkpoint" in ctx.save:
            entity.transform.position = ctx.save["checkpoint"]

    Values are JSON: numbers, strings, True/False/None, and lists and
    dicts of them (tuples come back as lists). Changes are written at the
    end of the frame, nested ones too (``ctx.save["items"].append(...)``).
    Slots: ``use("2")`` switches, ``slots()`` lists them, ``delete(slot)``
    removes one and ``clear()`` empties the current one.
    """

    def __init__(self, store):
        self._store = store
        self._slot = "default"
        self._data = None
        self._written = None  # the JSON last written (or read)
        self._touched = False
        self._saved_at = None

    def _rel(self, slot=None):
        return os.path.join("saves", f"{slot or self._slot}.json")

    def _load(self):
        if self._data is None:
            raw = self._store.read(self._rel())
            data = raw.get("data") if isinstance(raw, dict) else None
            self._data = data if isinstance(data, dict) else {}
            self._saved_at = raw.get("saved_at") if isinstance(raw, dict) else None
            self._written = json.dumps(self._data, sort_keys=True)
        return self._data

    def _flush(self):
        """Writes the slot if anything in it changed."""
        if self._data is None or not self._touched:
            return
        text = json.dumps(self._data, sort_keys=True)
        if text != self._written:
            saved_at = time.time()
            self._store.write(self._rel(), {"format": _SAVE_FORMAT, "saved_at": saved_at,
                                            "data": self._data})
            self._written, self._saved_at = text, saved_at
        self._touched = False

    # --- the dict --------------------------------------------------------

    def __getitem__(self, key: str) -> Any:
        data = self._load()
        if key not in data:
            raise KeyError(f"{key!r} isn't saved in slot {self._slot!r}"
                           f"{_did_you_mean(str(key), data)} (use ctx.save.get(key, default))")
        value = data[key]
        if isinstance(value, (list, dict)):
            self._touched = True  # it may be changed in place
        return value

    def __setitem__(self, key: str, value: Any) -> None:
        if not isinstance(key, str):
            raise TypeError(f"save keys are strings, not {key!r}")
        self._load()[key] = _plain(value, f"ctx.save[{key!r}]")
        self._touched = True

    def __delitem__(self, key: str) -> None:
        del self._load()[key]
        self._touched = True

    def __contains__(self, key: object) -> bool:
        return key in self._load()

    def __iter__(self):
        return iter(list(self._load()))

    def __len__(self) -> int:
        return len(self._load())

    def get(self, key: str, default: Any = None) -> Any:
        """The saved value, or ``default`` when there's none."""
        return self[key] if key in self._load() else default

    def setdefault(self, key: str, default: Any = None) -> Any:
        if key not in self._load():
            self[key] = default
        return self[key]

    def pop(self, key: str, *default: Any) -> Any:
        data = self._load()
        if key in data:
            self._touched = True
        return data.pop(key, *default)

    def update(self, values: dict[str, Any] | None = None, **more: Any) -> None:
        for k, v in {**(values or {}), **more}.items():
            self[k] = v

    def keys(self) -> list[str]:
        return list(self._load())

    def items(self) -> list[tuple[str, Any]]:
        self._touched = True
        return list(self._load().items())

    def values(self) -> list[Any]:
        self._touched = True
        return list(self._load().values())

    def clear(self) -> None:
        """Empties the current slot."""
        self._load().clear()
        self._touched = True

    # --- slots -------------------------------------------------------------

    @property
    def slot(self) -> str:
        """The current slot's name (``"default"`` until :meth:`use`)."""
        return self._slot

    @property
    def saved_at(self) -> float | None:
        """When the current slot was last written (``time.time()``
        seconds), or None if it never was."""
        self._load()
        return self._saved_at

    @property
    def path(self) -> str | None:
        """The current slot's file (None when saves are kept in memory)."""
        return self._store.path(self._rel())

    def use(self, slot: str | int) -> None:
        """Switches to ``slot`` (a name or number), writing the current
        one first. A slot that was never saved starts empty."""
        name = _slot_name(slot)
        if name == self._slot:
            return
        self._flush()
        self._slot, self._data, self._written, self._saved_at = name, None, None, None

    def exists(self, slot: str | int | None = None) -> bool:
        """Whether ``slot`` (default: the current one) has been saved."""
        name = self._slot if slot is None else _slot_name(slot)
        if name == self._slot and self._data is not None and (self._data or self._touched):
            return True
        if self._store.folder is None:
            return False
        path = self._store.path(self._rel(name))
        return path is not None and os.path.isfile(path)

    def slots(self) -> list[SaveSlot]:
        """Every saved slot, by name."""
        self._flush()
        out = []
        for name in self._store.names("saves", ".json"):
            if name == self._slot and self._data is not None:
                out.append(SaveSlot(name, self._saved_at, len(self._data)))
                continue
            raw = self._store.read(self._rel(name))
            if isinstance(raw, dict) and isinstance(raw.get("data"), dict):
                out.append(SaveSlot(name, raw.get("saved_at"), len(raw["data"])))
        if self._store.folder is None and self._data:
            out.append(SaveSlot(self._slot, self._saved_at, len(self._data)))
        return out

    def delete(self, slot: str | int | None = None) -> None:
        """Deletes ``slot`` (default: the current one, which then starts
        empty)."""
        name = self._slot if slot is None else _slot_name(slot)
        self._store.remove(self._rel(name))
        if name == self._slot:
            self._data, self._touched, self._saved_at = {}, False, None
            self._written = json.dumps({})

    def flush(self) -> None:
        """Writes changes now instead of at the end of the frame."""
        self._touched = True
        self._flush()

    def __repr__(self):
        return f"SaveData(slot={self._slot!r}, {dict(self._load())!r})"


class Settings:
    """``ctx.settings``: the player's settings, kept between runs and
    shared by every save slot.

    - ``volume`` (0 to 1) sets the master volume.
    - ``graphics`` (``"low"`` … ``"beautiful"``) sets the graphics preset.
    - Key bindings: ``bind("jump", "space")`` gives an action its default
      keys, ``rebind("jump", "k")`` changes them for good, and
      ``ctx.input.pressed("jump")`` checks them.
    - Anything else: ``ctx.settings["sensitivity"] = 0.4``.

    Changes apply and are written at the end of the frame.
    """

    def __init__(self, store, current_graphics):
        self._store = store
        self._current_graphics = current_graphics
        self._loaded = False
        self._volume = None
        self._graphics = None
        self._rebound = {}
        self._values = {}
        self._defaults = {}
        self._dirty = False
        self._apply = {}
        self._initial_graphics = None

    def _load(self):
        if not self._loaded:
            self._loaded = True
            self._initial_graphics = self._current_graphics()
            raw = self._store.read("settings.json")
            raw = raw if isinstance(raw, dict) else {}
            v = raw.get("volume")
            if isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v):
                self._volume = min(max(float(v), 0.0), 1.0)
                self._apply["volume"] = self._volume
            g = raw.get("graphics")
            if g in GRAPHICS_PRESETS:
                self._graphics = g
                self._apply["graphics"] = g
            keys = raw.get("keys")
            if isinstance(keys, dict):
                self._rebound = {str(a): tuple(str(k) for k in ks)
                                 for a, ks in keys.items() if isinstance(ks, list)}
            values = raw.get("values")
            if isinstance(values, dict):
                self._values = values
        return self

    def _flush(self):
        if not self._dirty:
            return
        data: dict[str, Any] = {"format": _SAVE_FORMAT, "values": self._values,
                "keys": {a: list(ks) for a, ks in self._rebound.items()}}
        if self._volume is not None:
            data["volume"] = self._volume
        if self._graphics is not None:
            data["graphics"] = self._graphics
        self._store.write("settings.json", data)
        self._dirty = False

    def _take_apply(self):
        self._load()
        out, self._apply = self._apply, {}
        return out or None

    @property
    def volume(self) -> float:
        """The master volume, 0 (silent) to 1 (full, the default)."""
        v = self._load()._volume
        return 1.0 if v is None else v

    @volume.setter
    def volume(self, v: float) -> None:
        v = float(v)
        if not math.isfinite(v):
            raise ValueError(f"volume is 0 to 1, not {v!r}")
        self._load()._volume = min(max(v, 0.0), 1.0)
        self._apply["volume"] = self._volume
        self._dirty = True

    @property
    def graphics(self) -> str:
        """The graphics preset: ``"low"``, ``"medium"``, ``"high"``,
        ``"ultra"`` or ``"beautiful"``. Until the player picks one it's
        whatever the game started with."""
        g = self._load()._graphics
        if g is None:
            g = self._current_graphics()
        return g if g in GRAPHICS_PRESETS else "high"

    @graphics.setter
    def graphics(self, quality: str) -> None:
        q = str(quality).strip().lower()
        if q not in GRAPHICS_PRESETS:
            raise ValueError(f"there's no graphics preset {quality!r}"
                             f"{_did_you_mean(q, GRAPHICS_PRESETS)} "
                             f"(presets: {', '.join(GRAPHICS_PRESETS)})")
        self._load()._graphics = q
        self._apply["graphics"] = q
        self._dirty = True

    # --- key bindings -----------------------------------------------------

    @staticmethod
    def _check_keys(action, keys):
        if not isinstance(action, str) or not action or ":" in action:
            raise ValueError(f"an action is a name like 'jump', not {action!r}")
        out = []
        for k in keys:
            if isinstance(k, (list, tuple)):
                out.extend(Settings._check_keys(action, k))
                continue
            name = str(getattr(k, "value", k)).strip().lower()
            if name not in KEY_NAMES and name not in KEY_ALIASES and name not in MOUSE_BINDINGS:
                options = (*KEY_NAMES, *KEY_ALIASES, *MOUSE_BINDINGS)
                raise ValueError(f"{action!r}: there's no key {k!r}{_did_you_mean(name, options)} "
                                 "(keys are like 'space', 'w', 'arrow_up', 'shift' or "
                                 "'mouse_left')")
            out.append(name)
        return out

    def bind(self, action: str, *keys: str) -> None:
        """Gives ``action`` its default keys (``bind("jump", "space",
        "w")``). A player's :meth:`rebind` wins over it. Call it from
        ``start``; binding again replaces the defaults."""
        self._defaults[action] = tuple(self._check_keys(action, keys))

    def rebind(self, action: str, *keys: str) -> None:
        """Changes ``action``'s keys for good (saved). With no keys the
        action has none until :meth:`reset_keys`."""
        self._load()._rebound[action] = tuple(self._check_keys(action, keys))
        self._dirty = True

    def keys(self, action: str) -> tuple[str, ...]:
        """``action``'s keys now: the player's, else the defaults."""
        self._load()
        if action in self._rebound:
            return self._rebound[action]
        return self._defaults.get(action, ())

    def bindings(self) -> dict[str, tuple[str, ...]]:
        """Every action's keys, by action name."""
        self._load()
        return {a: self.keys(a) for a in sorted({*self._defaults, *self._rebound})}

    def is_action(self, name: str) -> bool:
        """Whether ``name`` is a bound action."""
        return name in self._defaults or name in self._load()._rebound

    def reset_keys(self, action: str | None = None) -> None:
        """Puts ``action``'s keys (or every action's) back to the defaults."""
        self._load()
        if action is None:
            self._rebound = {}
        else:
            self._rebound.pop(action, None)
        self._dirty = True

    # --- anything else ----------------------------------------------------

    def __getitem__(self, key: str) -> Any:
        values = self._load()._values
        if key not in values:
            raise KeyError(f"there's no setting {key!r}{_did_you_mean(str(key), values)} "
                           "(use ctx.settings.get(key, default))")
        value = values[key]
        if isinstance(value, (list, dict)):
            self._dirty = True
        return value

    def __setitem__(self, key: str, value: Any) -> None:
        if not isinstance(key, str):
            raise TypeError(f"setting names are strings, not {key!r}")
        self._load()._values[key] = _plain(value, f"ctx.settings[{key!r}]")
        self._dirty = True

    def __delitem__(self, key: str) -> None:
        del self._load()._values[key]
        self._dirty = True

    def __contains__(self, key: object) -> bool:
        return key in self._load()._values

    def get(self, key: str, default: Any = None) -> Any:
        return self[key] if key in self._load()._values else default

    def reset(self) -> None:
        """Every setting back to the game's defaults: full volume, the
        starting graphics, default keys, no other values."""
        self._load()
        if self._volume is not None:
            self._apply["volume"] = 1.0
        if self._graphics is not None and self._initial_graphics in GRAPHICS_PRESETS:
            self._apply["graphics"] = self._initial_graphics
        self._volume, self._graphics, self._rebound, self._values = None, None, {}, {}
        self._dirty = True

    @property
    def path(self) -> str | None:
        """The settings file (None when kept in memory)."""
        return self._store.path("settings.json")

    def __repr__(self):
        return (f"Settings(volume={self.volume:g}, graphics={self.graphics!r}, "
                f"keys={self.bindings()!r}, values={self._values!r})")


class Context:
    """The ``ctx`` every script function receives."""

    def __init__(self, host, input_source):
        self._host = host
        self._keys = KeyStore(host)
        self.input = Input(input_source)
        self.time = Time()
        self.physics = Physics(host)
        self.water = Water(host)
        self.particles = Particles(host)
        self.scene = SceneView(host)
        self.audio = Audio(host)
        self.hud = Hud()
        store = _Store(host.save_dir, host.backend.log)
        #: The current save slot (see :class:`SaveData`).
        self.save = SaveData(store)
        #: The player's settings (see :class:`Settings`).
        self.settings = Settings(store, host.backend.graphics)
        self.input._settings = self.settings

    @property
    def camera(self) -> Camera | None:
        """The scene camera Play renders through (None if the scene has none)."""
        for e in self._host.entities.values():
            if e._read("cam") is not None:
                return Camera(e)
        return None

    def log(self, *parts: object) -> None:
        """Writes to the Console (``print()`` works too)."""
        self._host.backend.log("info", " ".join(str(p) for p in parts))

    def warn(self, *parts: object) -> None:
        self._host.backend.log("warn", " ".join(str(p) for p in parts))

    def subscribe(self, pattern: str, fn: Callable[[Message], Any]) -> Subscription:
        """Calls ``fn(message)`` for every message published on a channel
        matching ``pattern``: an exact name, or ``*`` and ``?`` wildcards.
        Animation events arrive as ``"<entity>:animation:<event>"``::

            ctx.subscribe("Player:animation:footstep", step_sound)
            ctx.subscribe("*:animation:hit", deal_damage)

        Subscribing the same function to the same pattern again changes
        nothing, so it's safe in ``update``. A subscription made by an
        entity's script ends when the entity goes. Returns a
        :class:`Subscription` (``cancel()`` it to stop)."""
        return self._host.subscribe(pattern, fn)

    def unsubscribe(self, pattern: str, fn: Callable[[Message], Any] | None = None) -> int:
        """Ends this script's subscriptions to ``pattern`` (only those to
        ``fn`` if given), or the :class:`Subscription` passed. Returns how
        many ended."""
        return self._host.unsubscribe(pattern, fn)

    def publish(self, channel: str, data: Any = None) -> int:
        """Sends ``data`` to every subscriber of ``channel`` now (they run
        before this returns). Returns how many received it."""
        return self._host.publish(Message(str(channel), data))

    # Keys, Redis style (see KeyStore).

    def get(self, key: str) -> Any:
        """The value at ``key``: ``"Player:animation:clip"``,
        ``"Player:attr:health"``, ``"Player:transform:position"``.
        ``"Player:animation"`` gives the component, ``"Player"`` the entity.
        Raises ``KeyError`` (suggesting the nearest key) for a wrong key or a
        name two entities share."""
        return self._keys.get(key)

    def set(self, key: str, value: Any) -> None:
        """Sets the field at ``key`` (``"Player:animation:speed"``). Setting
        ``animation:clip`` plays that clip. Applies like any other write."""
        self._keys.set(key, value)

    def scan(self, pattern: str) -> dict[str, Any]:
        """Everything matching ``pattern`` (``*`` and ``?`` wildcards), in
        scene order: ``ctx.scan("Enemy_*:animation")`` gives each enemy's
        animation, ``ctx.scan("Enemy_*:attr:health")`` their health.
        Entities without that component or field are skipped."""
        return self._keys.scan(pattern)

    def keys(self, pattern: str = "*") -> list[str]:
        """The keys matching ``pattern``: ``ctx.keys("Player:attr:*")``."""
        return self._keys.keys(pattern)

    def ref(self, key: str) -> Entity | ComponentRef | FieldRef:
        """A handle to keep (from ``start``) that looks ``key`` up again on
        each use: a component (``ctx.ref("Player:animation").play("Run")``)
        or a field (``ctx.ref("Player:attr:health").value``)."""
        return self._keys.ref(key)

    # Tweens and timers.

    def tween(
        self,
        entity: Entity | str,
        field: str,
        to: Any,
        duration: float,
        ease: str | Callable[[float], float] = "out_cubic",
        *,
        start: Any = None,
        delay: float = 0.0,
        loop: bool | int = False,
        yoyo: bool = False,
        on_update: Callable[[Any], Any] | None = None,
        on_done: Callable[[], Any] | None = None,
    ) -> Tween:
        """Moves one of ``entity``'s values to ``to`` over ``duration``
        seconds along an easing curve::

            ctx.tween(door, "position", (0, 3, 0), 1.5, ease="in_out_sine")
            ctx.tween("Coin_3", "scale", (0, 0, 0), 0.3, ease="in_back",
                      on_done=coin.despawn)
            ctx.tween(entity, "attr:score", 100, 2.0, ease="linear")

        ``entity`` is an entity or its name. ``field`` is ``"position"``,
        ``"rotation"`` (Euler radians) or ``"scale"``, an attribute's name,
        or a key's ``component:field`` (``"animation:speed"``,
        ``"attr:glow"``). Numbers and tuples of numbers tween; whole numbers
        stay whole.

        ``ease`` is a curve name (see :data:`EASINGS`) or a function of
        ``t``. ``start`` overrides where it starts from (otherwise the value
        when it begins, after ``delay`` seconds). ``loop=True`` repeats it
        forever and ``loop=3`` plays it three times; ``yoyo=True`` makes
        every other play run backwards, so ``yoyo=True, loop=True`` swings
        back and forth. ``on_update(value)`` runs each frame it moves and
        ``on_done()`` when it ends.

        A new tween on the same entity and field replaces the old one.
        Tweens move values once a frame, before ``update`` runs, and stop
        when the entity goes (or the one whose script started them).
        Returns a :class:`Tween` to ``cancel()``, ``finish()`` or read.
        """
        return self._host.tween(entity, field, to, duration, ease, start, delay, loop, yoyo,
                                on_update, on_done)

    def tween_value(
        self,
        start: Any,
        end: Any,
        duration: float,
        on_update: Callable[[Any], Any] | None = None,
        ease: str | Callable[[float], float] = "out_cubic",
        *,
        delay: float = 0.0,
        loop: bool | int = False,
        yoyo: bool = False,
        on_done: Callable[[], Any] | None = None,
    ) -> Tween:
        """A tween that moves a value of your own from ``start`` to ``end``:
        read it as ``.value``, or have ``on_update(value)`` called with it
        each frame. Good for fading HUD text::

            entity.state["fade"] = ctx.tween_value(1.0, 0.0, 0.8, ease="in_quad")
            ...
            alpha = entity.state["fade"].value
            ctx.hud.text("Checkpoint!", *ctx.hud.center, color=(1, 1, 1, alpha))

        The other arguments are as for :meth:`tween`.
        """
        return self._host.tween_value(start, end, duration, on_update, ease, delay, loop, yoyo,
                                      on_done)

    def after(self, seconds: float, fn: Callable[..., Any], *args: Any) -> Timer:
        """Calls ``fn(*args)`` once, ``seconds`` from now::

            ctx.after(2.0, door.despawn)
            ctx.after(0.5, ctx.audio.play, "sounds/ding.ogg")

        Returns a :class:`Timer` (``cancel()`` it to call it off). Timers
        count ``ctx.time.delta`` and fire before ``update`` runs, and
        stop when the entity whose script made them goes.
        """
        return self._host.timer(seconds, fn, args, repeat=False, times=None)

    def every(
        self, seconds: float, fn: Callable[..., Any], *args: Any, times: int | None = None
    ) -> Timer:
        """Calls ``fn(*args)`` every ``seconds``, first ``seconds`` from now,
        forever or ``times`` times::

            ctx.every(1.0, spawn_enemy)
            ctx.every(0.1, blink, entity, times=6)

        Returns a :class:`Timer`: ``cancel()`` stops it. Call ``every`` from
        ``start`` (or once on an event), not from ``update``, or you'll
        start a new timer each frame.
        """
        return self._host.timer(seconds, fn, args, repeat=True, times=times)

    def ease(self, name: str | Callable[[float], float], t: float) -> float:
        """``t`` (0 to 1) through an easing curve: ``ctx.ease("out_back",
        0.5)``. See :data:`EASINGS` for the names."""
        return ease(name, t)

    if not TYPE_CHECKING:  # type checkers flag misspelt names instead
        def __getattr__(self, name):
            return self._host.backend.extra("ctx", name)


# --------------------------------------------------------------------------
# The host: scripts, frames, errors
# --------------------------------------------------------------------------


class FrameOutput:
    """What a frame's scripts changed, for the driver to apply.

    ``writes``: one dict per changed entity (``id`` plus any of ``p``,
    ``r``, ``s``, ``v``, ``cam`` (tuples), ``attrs`` (the whole new dict),
    ``audio`` ("play"/"stop"), ``despawn``). ``sounds``: ``[path, volume, pitch, position or None]``.
    ``hud``: draw items. ``lock``: True/False to capture/free the cursor,
    or None. ``settings``: player settings to apply now, ``{"volume": 0.5,
    "graphics": "low"}`` (either may be missing), or None.
    """

    __slots__ = ("writes", "sounds", "hud", "lock", "settings")

    def __init__(self, writes, sounds, hud, lock, settings=None):
        self.writes = writes
        self.sounds = sounds
        self.hud = hud
        self.lock = lock
        self.settings = settings


class Backend:
    """What a driver provides. Override what applies; the defaults suit a
    driver without physics, audio or engine extras."""

    def raycast(self, origin, direction, max_distance, ignore_id):
        """``(entity id, point, normal, distance)`` or None. ``direction``
        is normalized."""
        return None

    def grounded(self, entity):
        return False

    def water_height(self, x, z):
        """The water's surface height at ``(x, z)``, or None."""
        return None

    def splash(self, position, strength):
        """A splash in the water at ``position`` (x, y, z); ``strength``
        is 0..10. Drivers without water ignore it."""

    def burst(self, preset, fields, position, direction, count):
        """A one-off particle effect: ``preset`` (a name or None for
        defaults) with ``fields`` changed, at ``position``, flying along
        ``direction`` (or None: the effect's own up), ``count`` particles
        (or None). Drivers without particles ignore it."""

    def burst_emitter(self, entity_id, count):
        """``count`` extra particles from an entity's own emitter."""

    def source_playing(self, entity):
        return False

    def camera_3d(self, entity):
        return None

    def graphics(self):
        """The graphics preset the game runs at (``"high"``...), or None."""
        return None

    def log(self, level, message):
        print(message)

    def error(self, report):
        """A script error (a dict: ``msg``, ``file``, ``line``, ``traceback``)."""
        print(report["traceback"], file=sys.stderr)

    def extra(self, owner, name):
        """Engine-only members (``ctx.events``, ``ctx.physics.apply_impulse``…)."""
        where = "ctx" if owner == "ctx" else f"ctx.{owner}"
        raise AttributeError(
            f"{where}.{name} isn't available here (engine extras need run_scene)"
        )


class Script:
    """A loaded behaviour module, reloaded when its file changes."""

    def __init__(self, path, prefix):
        self.path = path
        self.prefix = prefix
        self.module = None
        self.mtime = None
        self.broken = False

    def load(self):
        self.mtime = _mtime(self.path)
        self.module = load_module(self.path, self.prefix)
        self.broken = False


def load_module(path, prefix="automata_script_"):
    """Imports a Python file as a fresh module."""
    name = prefix + str(abs(hash(os.path.abspath(path))))
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load script {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _mtime(path):
    try:
        return os.stat(path).st_mtime
    except OSError:
        return None


class ScriptHost:
    """Runs behaviour scripts frame by frame.

    ``on_error="report"`` sends a script's exception to
    ``backend.error`` once and pauses that script until its file changes
    (Oberon). ``on_error="raise"`` lets it propagate (``run_scene``).
    ``hot_reload`` reloads changed script files about twice a second.
    """

    def __init__(self, backend, input_source=None, on_error="raise", hot_reload=False,
                 module_prefix="automata_script_", internal_files=(), save_dir=None):
        self.backend = backend
        #: Where ``ctx.save`` and ``ctx.settings`` live (None: in memory).
        self.save_dir = None if save_dir is None else os.path.abspath(save_dir)
        self._save_error = None
        self.on_error = on_error
        self.hot_reload = hot_reload
        self.module_prefix = module_prefix
        self.internal = {os.path.abspath(__file__)} | {os.path.abspath(f) for f in internal_files}
        self.entities = {}
        self._by_name = {}
        self._name_count = {}
        self.scripts = {}
        self.started = set()
        self.ctx = Context(self, input_source if input_source is not None else KeySource())
        self._last_check = 0.0
        self._subs = []
        self._timers = []  # Timers and Tweens, oldest first
        self._running = None  # (script, entity) whose code runs now

    # --- frames -----------------------------------------------------------

    def frame(self, records, delta=0.0, elapsed=0.0, frame=0, screen=None):
        """Runs one frame over ``records`` and returns a :class:`FrameOutput`."""
        t = self.ctx.time
        t.delta, t.elapsed, t.frame = float(delta), float(elapsed), int(frame)
        if screen:
            self.ctx.hud.size = (int(screen[0]), int(screen[1]))
        self._sync(records)
        if self.hot_reload and (elapsed - self._last_check > 0.4 or elapsed < self._last_check):
            self._last_check = elapsed
            self.reload_changed()
        self._deliver_animation_events()
        if self._timers:
            self._tick_timers(t.delta)
        for e in list(self.entities.values()):
            if not e.script:
                continue
            s = self.script(e.script)
            if s is None:
                continue
            if e.id not in self.started:
                self.started.add(e.id)
                self.call(s, "start", e)
                if s.broken:
                    continue
            self.call(s, "update", e)
        return self._collect()

    def _sync(self, records):
        old = self.entities
        self.entities = {}
        self._by_name = {}
        counts = self._name_count = {}
        for record in records:
            eid = record.get("id")
            e = old.get(eid)
            if e is None:
                e = Entity(self, eid)
            e._begin(record)
            self.entities[eid] = e
            counts[e.name] = counts.get(e.name, 0) + 1
            if e.name and e.name not in self._by_name:
                self._by_name[e.name] = e
        if self._subs or self._timers:
            self._drop_orphans()

    def _drop_orphans(self):
        """Ends subscriptions, timers and tweens whose entity is gone."""
        keep = []
        for s in self._subs:
            if s.owner is None or s.owner.id in self.entities:
                keep.append(s)
            else:
                s.active = False
        self._subs = keep
        for t in self._timers:
            if t.owner is not None and self.entities.get(t.owner.id) is not t.owner:
                t.active = False
        self._timers = [t for t in self._timers if t.active]

    # --- tweens and timers ------------------------------------------------

    def _tick_timers(self, dt):
        """Advances every timer and tween by ``dt``: the ones made during
        this frame start next frame."""
        dt = max(0.0, float(dt))
        for item in list(self._timers):
            if not item.active or item.paused:
                continue
            script = item.script
            if script is not None and (script.broken or script.module is None):
                continue  # paused with its script until the file's fixed
            item._tick(dt)
        self._timers = [t for t in self._timers if t.active]

    def timer(self, seconds, fn, args, repeat, times):
        """See :meth:`Context.after` and :meth:`Context.every`."""
        if not callable(fn):
            raise TypeError(f"{'every' if repeat else 'after'} needs a function to call, "
                            f"not {fn!r}")
        seconds = float(seconds)
        if not math.isfinite(seconds) or seconds < 0.0 or (repeat and seconds <= 0.0):
            raise ValueError(f"{'every' if repeat else 'after'} needs a number of seconds "
                             f"{'above' if repeat else 'from'} 0, not {seconds!r}")
        if times is not None:
            times = int(times)
            if times < 1:
                raise ValueError(f"times must be 1 or more, not {times!r}")
        item = Timer(self, seconds, fn, tuple(args), repeat, times)
        self._timers.append(item)
        return item

    def _tween_target(self, entity, field):
        """(entity, component, field, key) for ``ctx.tween``'s target."""
        store = self.ctx._keys
        if isinstance(entity, str):
            entity = store._entity(entity, entity)
        elif not isinstance(entity, Entity):
            raise TypeError(f"tween moves an entity (or its name), not {entity!r}; use "
                            "ctx.tween_value for a value of your own")
        if self.entities.get(entity.id) is not entity:
            raise KeyError(f"can't tween {entity.name!r}: it's gone")
        if not isinstance(field, str) or not field:
            raise TypeError(f"field is a name like 'position' or 'attr:health', not {field!r}")
        if ":" in field:
            parts = field.split(":")
            if len(parts) != 2 or not all(parts):
                raise KeyError(f"{field!r} isn't a field: use 'position', 'rotation', 'scale', "
                               "an attribute's name, or 'component:field' like 'attr:health'")
            component, name = parts
        elif field in _TRANSFORM_SHORTCUTS:
            component, name = "transform", field
        elif field in entity.attributes:
            component, name = "attr", field
        else:
            options = list(_TRANSFORM_SHORTCUTS) + list(entity.attributes)
            raise KeyError(f"can't tween {entity.name!r}'s {field!r}: it's not 'position', "
                           f"'rotation', 'scale' or one of its attributes{_did_you_mean(field, options)}")
        key = f"{entity.name}:{component}:{name}"
        fields = store._component_info(key, component)[2]
        name = store._field_name(key, component, name, fields)
        key = f"{entity.name}:{component}:{name}"
        info = fields.get(name) if fields is not None and name is not None else None
        if fields is not None and info is not None and info[1] is None:
            writable = [f for f, (_, s) in fields.items() if s is not None]
            raise AttributeError(f"{key!r} is read-only (you can tween {', '.join(writable)})")
        return entity, component, name, key

    @staticmethod
    def _tween_args(duration, ease, delay, loop):
        duration, delay = float(duration), float(delay)
        if not math.isfinite(duration) or duration < 0.0:
            raise ValueError(f"a tween's duration is seconds from 0 up, not {duration!r}")
        if not math.isfinite(delay) or delay < 0.0:
            raise ValueError(f"delay is seconds from 0 up, not {delay!r}")
        if loop is True:
            plays = None
        elif loop is False or loop is None:
            plays = 1
        else:
            plays = int(loop)
            if plays < 1:
                raise ValueError(f"loop is True (forever), False, or how many times to play "
                                 f"(1 or more), not {loop!r}")
        if plays is None and duration <= 0.0:
            raise ValueError("a tween that loops forever needs a duration above 0")
        return duration, easing(ease), delay, plays

    def tween(self, entity, field, to, duration, ease, start, delay, loop, yoyo, on_update,
              on_done):
        """See :meth:`Context.tween`."""
        entity, component, name, key = self._tween_target(entity, field)
        duration, curve, delay, plays = self._tween_args(duration, ease, delay, loop)
        end = _numbers(to, "to")
        what = f"{key!r}, which is"
        now = _numbers(self.ctx._keys._read_field(key, entity, component, name), what)
        if start is not None:
            start = _numbers(start, "start")
            _check_shapes(start, end, key)
        _check_shapes(now, end, key)
        for old in self._timers:
            if (isinstance(old, Tween) and old.active and old._entity is entity
                    and old._component == component and old._field == name):
                old.active = False
        item = Tween(self, entity, component, name, start, end, duration, curve, delay, plays,
                     bool(yoyo), on_update, on_done, key)
        if start is None:
            item.value = now
        self._timers.append(item)
        return item

    def tween_value(self, start, end, duration, on_update, ease, delay, loop, yoyo, on_done):
        """See :meth:`Context.tween_value`."""
        duration, curve, delay, plays = self._tween_args(duration, ease, delay, loop)
        start, end = _numbers(start, "start"), _numbers(end, "end")
        _check_shapes(start, end, "the value")
        if on_update is not None and not callable(on_update):
            raise TypeError(f"on_update is a function to call with the value, not {on_update!r}")
        item = Tween(self, None, None, None, start, end, duration, curve, delay, plays,
                     bool(yoyo), on_update, on_done, None)
        self._timers.append(item)
        return item

    # --- channels ---------------------------------------------------------

    def subscribe(self, pattern, fn):
        """See :meth:`Context.subscribe`."""
        if not callable(fn):
            raise TypeError(f"subscribe needs a function to call, not {fn!r}")
        pattern = str(pattern)
        script, owner = self._running or (None, None)
        key = _fn_key(fn)
        for s in self._subs:
            if s.pattern == pattern and s.owner is owner and s.key == key:
                s.fn = fn  # the latest version (after a hot reload)
                return s
        sub = Subscription(self, pattern, fn, script, owner)
        self._subs.append(sub)
        return sub

    def unsubscribe(self, pattern, fn=None):
        """See :meth:`Context.unsubscribe`."""
        if isinstance(pattern, Subscription):
            was = pattern.active
            pattern.cancel()
            return int(was)
        _, owner = self._running or (None, None)
        key = None if fn is None else _fn_key(fn)
        gone = [s for s in self._subs
                if s.pattern == pattern and s.owner is owner and (key is None or s.key == key)]
        for s in gone:
            s.cancel()
        return len(gone)

    def publish(self, message):
        """Delivers ``message`` to every matching subscriber; how many got it."""
        message.ctx = self.ctx
        delivered = 0
        for sub in list(self._subs):
            if not sub.active or not sub.matches(message.channel):
                continue
            script = sub.script
            fn = sub.fn
            if script is not None:
                if script.broken or script.module is None:
                    continue
                fn = _live_fn(script, fn)  # follows its script's hot reloads
            delivered += 1
            outer = self._running
            if script is not None:
                self._running = (script, sub.owner)
            try:
                fn(message)
            except BaseException as exc:  # noqa: BLE001 - user code
                if script is None:
                    raise
                script.broken = True
                who = sub.owner.name if sub.owner is not None else "?"
                self._failed(exc, script.path, f"subscriber of {sub.pattern!r} ({who})")
            finally:
                self._running = outer
        return delivered

    def _deliver_animation_events(self):
        """Hands each entity's new animation events to its script's
        ``on_animation_event(entity, ctx, name)`` and to the channel
        ``<name>:animation:<event>``."""
        for e in list(self.entities.values()):
            rec = e._record.get("anim")
            events = rec.get("ev") if rec else None
            if not events:
                continue
            if int(events[-1][0]) < e._ev_seen:
                e._ev_seen = 0  # a new animator counts from 1 again
            fresh = [ev for ev in events if int(ev[0]) > e._ev_seen]
            if not fresh:
                continue
            e._ev_seen = int(fresh[-1][0])
            for _seq, clip, name in fresh:
                if e.script and e.id in self.started:
                    s = self.script(e.script)
                    if s is not None:
                        self.call(s, "on_animation_event", e, name)
                if e.id not in self.entities:
                    break  # despawned by a handler
                channel = f"{e.name or e.id}:animation:{name}"
                self.publish(Message(channel, None, e, name, clip))

    def _collect(self):
        writes = [w for w in (e._writes() for e in self.entities.values()) if w]
        gone = [e for e in self.entities.values() if e._despawn]
        for e in gone:
            del self.entities[e.id]
            if self._by_name.get(e.name) is e:
                del self._by_name[e.name]
            left = self._name_count.get(e.name, 1) - 1
            if left > 0:
                self._name_count[e.name] = left
            else:
                self._name_count.pop(e.name, None)
        if gone and self._subs:
            self._drop_orphans()
        self._persist()
        out = FrameOutput(writes, self.ctx.audio._queue, self.ctx.hud._items,
                          self.ctx.input._lock_request, self.ctx.settings._take_apply())
        self.ctx.audio._queue = []
        self.ctx.hud._items = []
        self.ctx.input._lock_request = None
        return out

    def stop(self):
        """Calls ``stop(entity, ctx)`` on every started entity, ends every
        timer and tween, and writes any unsaved save data and settings."""
        for e in list(self.entities.values()):
            if e.script and e.id in self.started:
                s = self.scripts.get(e.script)
                if s and s.module and not s.broken:
                    self.call(s, "stop", e)
        for t in self._timers:
            t.active = False
        self._timers = []
        self._persist()

    def _persist(self):
        """Writes changed save data and settings; a failure is logged
        (once per different error), not raised."""
        try:
            self.ctx.save._flush()
            self.ctx.settings._flush()
            self._save_error = None
        except OSError as exc:
            message = f"couldn't save to {self.save_dir}: {exc}"
            if message != self._save_error:
                self._save_error = message
                self.backend.log("warn", message)

    # --- scripts ----------------------------------------------------------

    def script(self, path):
        """The loaded script for ``path``, or None while it's broken."""
        s = self.scripts.get(path)
        if s is None:
            s = self.scripts[path] = Script(path, self.module_prefix)
            try:
                s.load()
            except BaseException as exc:  # noqa: BLE001 - user code
                s.broken = True
                s.mtime = _mtime(path)
                self._failed(exc, path, "import")
        if s.broken or s.module is None:
            return None
        return s

    def reload_changed(self):
        """Reloads every script whose file changed on disk."""
        for s in self.scripts.values():
            m = _mtime(s.path)
            if m is not None and m != s.mtime:
                try:
                    s.load()
                    self.backend.log("info", f"Reloaded {os.path.basename(s.path)}")
                    reloaded = getattr(self.backend, "reloaded", None)
                    if reloaded is not None:
                        reloaded(s.path)
                except BaseException as exc:  # noqa: BLE001 - user code
                    s.broken = True
                    s.mtime = m
                    self._failed(exc, s.path, "reload")

    def call(self, script, fn_name, entity, *args):
        fn = getattr(script.module, fn_name, None)
        if fn is None:
            return
        outer = self._running
        self._running = (script, entity)
        try:
            fn(entity, self.ctx, *args)
        except BaseException as exc:  # noqa: BLE001 - user code
            # One report, then the script pauses until its file changes.
            script.broken = True
            self._failed(exc, script.path, f"{fn_name}({entity.name})")
        finally:
            self._running = outer

    def _failed(self, exc, path, during):
        if self.on_error == "raise" or isinstance(exc, (KeyboardInterrupt, SystemExit)):
            raise exc
        self.backend.error(self.describe(exc, path, during))

    def describe(self, exc, path, during):
        """A script error as ``{msg, file, line, traceback}``, with the
        runner's own frames left out of the traceback."""
        tb = traceback.extract_tb(exc.__traceback__)
        line = None
        for frame in tb:
            if os.path.abspath(frame.filename) == os.path.abspath(path):
                line = frame.lineno
        if isinstance(exc, SyntaxError) and exc.lineno:
            line = exc.lineno
        user = [f for f in tb if os.path.abspath(f.filename) not in self.internal]
        text = "Traceback (most recent call last):\n" if user else ""
        text += "".join(traceback.format_list(user))
        text += "".join(traceback.format_exception_only(type(exc), exc))
        return {
            "msg": f"{os.path.basename(path)}: {type(exc).__name__}: {exc} (in {during})",
            "file": path,
            "line": line or 0,
            "traceback": text.rstrip(),
        }
