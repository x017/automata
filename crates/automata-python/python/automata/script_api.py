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
- ``joint``: the entity's Joint or None: a dict with ``kind``, ``to`` (the
  connected entity's name or None), ``on`` (enabled), ``value`` (hinge
  angle, slider position or anchor distance), ``attached``, ``broken``,
  ``force`` (newtons), ``motor`` (``[speed, target or None]`` or None),
  ``limits`` (``[min, max]`` or None) and ``bf`` (break force, 0 = never);
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
after the playhead passes them, just before ``update`` runs, and
``<entity name>:joint:broken`` the frame after a joint snaps.
"""

from __future__ import annotations

import csv
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
import unicodedata
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
    "EntityJoint",
    "EntityNav",
    "EntityRagdoll",
    "Nav",
    "NavGraph",
    "AI",
    "StateMachine",
    "Node",
    "Action",
    "Condition",
    "Sequence",
    "Selector",
    "Parallel",
    "Invert",
    "Succeed",
    "Repeat",
    "Wait",
    "Cooldown",
    "Noise",
    "SUCCESS",
    "FAILURE",
    "RUNNING",
    "FrameOutput",
    "Hit",
    "Hud",
    "Input",
    "Touch",
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


def _unit(x) -> float:
    x = float(x)
    if x != x:
        raise ValueError("weight must be a number")
    return min(max(x, 0.0), 1.0)


def _ik_target(target):
    """An IK target on the wire: None, an entity's name, or a point."""
    if target is None or isinstance(target, str):
        return target
    name = getattr(target, "name", None)
    if isinstance(name, str) and hasattr(target, "id"):
        if not name:
            raise ValueError("only named entities can be IK targets; give it a name")
        return name
    try:
        return list(_vec(target, 3))
    except (TypeError, ValueError):
        raise TypeError(
            "the target must be an entity, an entity's name, a point (x, y, z), or None"
        ) from None


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

    With an animation graph (``graph``), scripts set its parameters
    instead (``set("speed", 3.0)``, ``trigger("jump")``) and the graph
    picks the clips; ``state`` says where it is.
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
        ``KeyError`` when the model has no such clip.

        With an animation graph, ``clip`` names a state of its first layer
        and the graph goes straight there (``play_state``)."""
        st = self._st()
        clip = str(clip)
        if st.get("graph"):
            self.play_state(clip, fade=fade)
            return
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
        """Stops, fading back to the rest pose over ``fade`` seconds. An
        animation graph stops running too."""
        self._e._anim_cmd(["stop", max(0.0, float(fade))])
        self._st().update(clip=None, time=0.0, len=0.0, playing=False, graph=None, gl=[], gp=[])

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

    # --- Inverse kinematics ----------------------------------------------

    def look_at(self, target=None, weight: float = 1.0) -> None:
        """Turns the head (and neck) towards ``target``: an entity, an
        entity's name, or a point ``(x, y, z)``; None stops. ``weight``
        (0 to 1) says how far. It eases in and out, and stops turning
        past a natural limit. An animated entity is looked in the eye."""
        self._e._anim_cmd(["look_at", _ik_target(target), _unit(weight)])
        st = self._st()
        if st is not None:
            st["look"] = target is not None

    def reach(self, bone: str, target=None, weight: float = 1.0) -> None:
        """Bends the limb ending at ``bone`` (a hand, a foot) so it
        reaches ``target``: an entity, an entity's name, or a point; None
        stops. ``weight`` (0 to 1) says how far. Eases in and out."""
        bone = str(bone)
        self._e._anim_cmd(["reach", bone, _ik_target(target), _unit(weight)])
        st = self._st()
        if st is not None:
            bones = [b for b in st.get("reach") or () if b != bone]
            st["reach"] = bones + [bone] if target is not None else bones

    @property
    def foot_ik(self) -> bool:
        """Whether the feet follow the ground: on slopes and stairs each
        foot stands on what's under it and the hips drop for the lower
        one. Settable (also the Animation component's Feet On Ground)."""
        return bool(self._st().get("fik"))

    @foot_ik.setter
    def foot_ik(self, on: bool) -> None:
        self._e._anim_cmd(["foot_ik", bool(on)])
        self._st()["fik"] = bool(on)

    @property
    def looking(self) -> bool:
        """Whether the head is set to look at something."""
        return bool(self._st().get("look"))

    @property
    def reaching(self) -> tuple:
        """The bones set to reach for something."""
        return tuple(self._st().get("reach") or ())

    def length(self, clip: str | None = None) -> float:
        """A clip's length in seconds (default: the current clip's)."""
        if clip is None:
            return self.duration
        st = self._st()
        clips = list(st.get("clips") or ())
        if clip not in clips:
            raise KeyError(f"the model has no clip {clip!r}")
        return float((st.get("lens") or [0.0] * len(clips))[clips.index(clip)])

    # --- Animation graphs ------------------------------------------------

    @property
    def graph(self) -> str | None:
        """The animation graph (a ``.anim.ron`` path) picking the clips, or
        None. Set a path to run one, None to go back to clips. A new
        graph's parameters can be set at once; they're checked from the
        next frame."""
        return self._st().get("graph") or None

    @graph.setter
    def graph(self, path: str | None) -> None:
        path = "" if path is None else str(path)
        self._e._anim_cmd(["graph", path])
        # Its states and parameters arrive next frame; None means unknown.
        self._st().update(graph=path or None, gl=None if path else [], gp=None if path else [])

    def _graph_st(self, what: str):
        st = self._st()
        if not st.get("graph"):
            raise RuntimeError(
                f"{what} needs an animation graph; set entity.animation.graph "
                "(or the Animation's Graph in the Inspector) first. If one is "
                "set, it didn't load: the log says why"
            )
        return st

    def _param(self, name: str, what: str):
        """The parameter's row ``[name, kind, value]``, or None while a new
        graph's parameters aren't known yet."""
        st = self._graph_st(what)
        rows = st.get("gp")
        if rows is None:
            return None
        for row in rows:
            if row[0] == name:
                return row
        names = ", ".join(r[0] for r in rows) or "none"
        raise KeyError(f"the graph has no parameter {name!r}; it has: {names}")

    def _layer(self, layer: str | None, what: str):
        """The layer's row ``[name, state, time, from, fade, weight,
        states]`` (None: the first), or None while unknown."""
        st = self._graph_st(what)
        rows = st.get("gl")
        if rows is None:
            return None
        if layer is None:
            return rows[0] if rows else None
        for row in rows:
            if row[0] == layer:
                return row
        names = ", ".join(r[0] for r in rows) or "none"
        raise KeyError(f"the graph has no layer {layer!r}; it has: {names}")

    def set(self, name: str, value) -> None:
        """Sets a graph parameter: a number for a float or an int (rounded),
        True/False for a bool. Raises ``KeyError`` for an unknown name and
        ``TypeError`` for a trigger (use ``trigger``)."""
        name = str(name)
        row = self._param(name, "set()")
        kind = row[1] if row is not None else None
        if kind == "trigger":
            raise TypeError(f"{name!r} is a trigger; use trigger({name!r})")
        if kind == "bool" or isinstance(value, bool):
            value = bool(value)
            stored = 1.0 if value else 0.0
        else:
            value = float(value)
            if value != value or value in (float("inf"), float("-inf")):
                raise ValueError(f"{name!r} must be a finite number")
            if kind == "int":
                value = float(round(value))
            stored = value
        self._e._anim_cmd(["set", name, value])
        if row is not None:
            row[2] = stored

    def get(self, name: str):
        """A graph parameter's value: a float, an int, or a bool (bools and
        triggers). Raises ``KeyError`` for an unknown name."""
        name = str(name)
        row = self._param(name, "get()")
        if row is None:
            raise KeyError(f"the graph's parameters arrive next frame; {name!r} isn't known yet")
        kind, value = row[1], float(row[2])
        if kind in ("bool", "trigger"):
            return value >= 0.5
        if kind == "int":
            return int(round(value))
        return value

    def trigger(self, name: str) -> None:
        """Sets a trigger: the next transition that tests it fires (and
        clears it). Raises ``KeyError`` for an unknown name and
        ``TypeError`` for a number."""
        name = str(name)
        row = self._param(name, "trigger()")
        if row is not None and row[1] not in ("trigger", "bool"):
            raise TypeError(f"{name!r} is a {row[1]}; use set({name!r}, value)")
        self._e._anim_cmd(["trigger", name])
        if row is not None:
            row[2] = 1.0

    def reset_trigger(self, name: str) -> None:
        """Clears a trigger no transition has used yet."""
        name = str(name)
        row = self._param(name, "reset_trigger()")
        if row is not None and row[1] != "trigger":
            raise TypeError(f"{name!r} is a {row[1]}, not a trigger")
        self._e._anim_cmd(["reset_trigger", name])
        if row is not None:
            row[2] = 0.0

    @property
    def parameters(self) -> dict:
        """Every graph parameter and its value (empty without a graph)."""
        st = self._st()
        if not st.get("graph"):
            return {}
        return {row[0]: self.get(row[0]) for row in st.get("gp") or ()}

    @property
    def state(self) -> str | None:
        """The state the graph's first layer is in (or fading to), or None
        without a graph."""
        st = self._st()
        if not st.get("graph"):
            return None
        rows = st.get("gl") or ()
        return rows[0][1] if rows else None

    @property
    def state_time(self) -> float:
        """How far through its motion the first layer's state is, 0 to 1."""
        st = self._st()
        rows = st.get("gl") or () if st.get("graph") else ()
        return float(rows[0][2]) if rows else 0.0

    @property
    def in_transition(self) -> bool:
        """Whether the first layer is crossfading between states."""
        st = self._st()
        rows = st.get("gl") or () if st.get("graph") else ()
        return bool(rows) and rows[0][3] is not None

    @property
    def layers(self) -> tuple[str, ...]:
        """The graph's layer names, bottom first (empty without a graph)."""
        st = self._st()
        return tuple(r[0] for r in st.get("gl") or ()) if st.get("graph") else ()

    def layer_state(self, layer: str) -> str:
        """The state a graph layer is in. Raises ``KeyError`` for an unknown
        layer."""
        row = self._layer(str(layer), "layer_state()")
        if row is None:
            raise KeyError(f"the graph's layers arrive next frame; {layer!r} isn't known yet")
        return row[1]

    def layer_weight(self, layer: str) -> float:
        """How strongly a graph layer shows over the ones below, 0 to 1."""
        row = self._layer(str(layer), "layer_weight()")
        return 1.0 if row is None else float(row[5])

    def set_layer_weight(self, layer: str, weight: float) -> None:
        """Fades a graph layer in or out (0 hides it, 1 shows it fully).
        The first layer always shows fully."""
        layer = str(layer)
        weight = min(max(float(weight), 0.0), 1.0)
        row = self._layer(layer, "set_layer_weight()")
        self._e._anim_cmd(["layer_weight", layer, weight])
        if row is not None:
            row[5] = weight

    def play_state(self, state: str, layer: str | None = None, fade: float = 0.0) -> None:
        """Sends a graph layer (default: the first) straight to ``state``,
        crossfading over ``fade`` seconds, whatever its transitions say.
        Raises ``KeyError`` for an unknown layer or state."""
        state = str(state)
        fade = max(0.0, float(fade))
        row = self._layer(None if layer is None else str(layer), "play_state()")
        if row is not None:
            if state not in row[6]:
                raise KeyError(
                    f"the layer {row[0]!r} has no state {state!r}; it has: {', '.join(row[6])}"
                )
            row[3] = row[1] if fade > 0 else None  # fading from
            row[4] = 0.0 if fade > 0 else 1.0
            row[1], row[2] = state, 0.0
        self._e._anim_cmd(["play_state", "" if layer is None else str(layer), state, fade])

    def __repr__(self):
        if self.graph:
            return f"Animation({self._e.name!r}, graph={self.graph!r}, state={self.state!r})"
        return f"Animation({self._e.name!r}, clip={self.clip!r})"


class EntityJoint:
    """``entity.joint``: the entity's Joint and how it's holding up.

    Reads come from the start of the frame (plus this frame's own
    writes); writes apply after every script has run::

        door.joint.motor_target = 1.6       # swing open
        if door.joint.angle > 1.5: ...
        bridge_plank.joint.enabled = False  # let go
    """

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def _st(self):
        return self._e._joint_state() or {}

    @property
    def kind(self) -> str:
        """``"hinge"``, ``"ball"``, ``"slider"``, ``"fixed"``, ``"rope"`` or ``"spring"``."""
        return str(self._st().get("kind", "hinge"))

    @property
    def connected(self) -> str | None:
        """The name of the entity it holds on to (None = the world)."""
        return self._st().get("to")

    @connected.setter
    def connected(self, name: str | None) -> None:
        self._e._joint_set("to", None if name is None else str(name))

    @property
    def value(self) -> float:
        """A hinge's angle (radians), a slider's position (metres) or, for
        the others, how far apart its two anchors are."""
        return float(self._st().get("value", 0.0))

    @property
    def angle(self) -> float:
        """A hinge's angle in radians from where it started (same as ``value``)."""
        return self.value

    @property
    def attached(self) -> bool:
        """Whether it's holding two things together right now."""
        return bool(self._st().get("attached", False))

    @property
    def broken(self) -> bool:
        """Whether it snapped past its ``break_force``."""
        return bool(self._st().get("broken", False))

    @property
    def force(self) -> float:
        """The force it held with last step, in newtons."""
        return float(self._st().get("force", 0.0))

    @property
    def enabled(self) -> bool:
        """Whether it holds at all; set False to let go."""
        return bool(self._st().get("on", True))

    @enabled.setter
    def enabled(self, on: bool) -> None:
        self._e._joint_set("on", bool(on))

    @property
    def motor_speed(self) -> float | None:
        """The motor's speed (rad/s or m/s), or None without a motor.
        Setting it drives at that speed; None removes the motor."""
        m = self._st().get("motor")
        return None if m is None else float(m[0])

    @motor_speed.setter
    def motor_speed(self, speed: float | None) -> None:
        self._e._joint_set("motor", None if speed is None else [float(speed), None])

    @property
    def motor_target(self) -> float | None:
        """The angle / position the motor pulls to, or None. Setting it
        pulls there; None removes the motor."""
        m = self._st().get("motor")
        return None if m is None or m[1] is None else float(m[1])

    @motor_target.setter
    def motor_target(self, target: float | None) -> None:
        if target is None:
            self._e._joint_set("motor", None)
        else:
            m = self._st().get("motor")
            speed = float(m[0]) if m is not None else 0.0
            self._e._joint_set("motor", [speed, float(target)])

    @property
    def limits(self) -> tuple[float, float] | None:
        """``(min, max)`` angle (radians) or slide (metres), or None."""
        lim = self._st().get("limits")
        return None if lim is None else (float(lim[0]), float(lim[1]))

    @limits.setter
    def limits(self, value: Iterable[float] | None) -> None:
        if value is None:
            self._e._joint_set("limits", None)
        else:
            lo, hi = _vec(value, 2)
            self._e._joint_set("limits", [min(lo, hi), max(lo, hi)])

    @property
    def break_force(self) -> float:
        """Newtons that snap it (0 = unbreakable)."""
        return float(self._st().get("bf", 0.0))

    @break_force.setter
    def break_force(self, newtons: float) -> None:
        self._e._joint_set("bf", max(0.0, float(newtons)))

    def __repr__(self):
        return f"Joint({self._e.name!r}, {self.kind})"


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
        self._joint_st: dict[str, Any] | None = None
        self._joint_read = False
        self._joint_w: dict[str, Any] | None = None
        self._joint_was_broken = False
        self._nav_w: dict[str, Any] | None = None
        self._rag_w: dict[str, Any] | None = None
        self._spr_w: dict[str, Any] | None = None

    def _begin(self, record):
        """Starts a frame from a fresh record."""
        self._record = record
        self._vals = {}
        self._attrs = None
        self._dirty = set()
        self._audio = None
        self._anim = None
        self._anim_st = None
        self._joint_st = None
        self._joint_read = False
        self._joint_w = None
        self._nav_w = None
        self._rag_w = None
        self._spr_w = None
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
        ``speed``, ``clips``. With an animation graph, ``set(name, value)``
        and ``trigger(name)`` steer it and ``state`` / ``layer_state(layer)``
        say where it is. None when its mesh isn't animated."""
        return EntityAnimation(self) if self._anim_state() is not None else None

    def _anim_state(self):
        """This frame's playback (a dict our own requests update), or None."""
        if self._anim_st is None:
            rec = self._record.get("anim")
            if rec is None:
                self._anim_st = False
            else:
                st = dict(rec)
                for key in ("gl", "gp"):  # our requests edit these rows
                    if st.get(key) is not None:
                        st[key] = [list(row) for row in st[key]]
                self._anim_st = st
        return self._anim_st if self._anim_st is not False else None

    def _anim_cmd(self, cmd):
        if self._anim is None:
            self._anim = []
        self._anim.append(cmd)
        self._dirty.add("anim")

    @property
    def joint(self) -> EntityJoint | None:
        """This entity's Joint: ``angle``, ``broken``, ``force``,
        ``enabled``, ``motor_speed``, ``motor_target``, ``limits``. None
        without a Joint component."""
        return EntityJoint(self) if self._joint_state() is not None else None

    def _joint_state(self) -> dict[str, Any] | None:
        """This frame's joint (a dict our own writes update), or None."""
        if not self._joint_read:
            self._joint_read = True
            rec = self._record.get("joint")
            self._joint_st = None if rec is None else dict(rec)
        return self._joint_st

    def _joint_set(self, key, value):
        st = self._joint_state()
        if st is None:
            raise AttributeError(f"{self.name!r} has no Joint")
        st[key] = value
        if self._joint_w is None:
            self._joint_w = {}
        self._joint_w[key] = value
        self._dirty.add("joint")

    @property
    def view(self) -> Any:
        """The engine's own handle for the entity (``run_scene`` only)."""
        return self._record.get("view")

    def attach(
        self,
        to: Any,
        bone: str = "",
        position: Iterable[float] = (0.0, 0.0, 0.0),
        rotation_euler: Iterable[float] = (0.0, 0.0, 0.0),
        scale: Iterable[float] = (1.0, 1.0, 1.0),
    ) -> None:
        """Follows ``bone`` of ``to`` (an entity or its name; no bone: the
        entity itself) from the next frame on, sitting at ``position`` /
        ``rotation_euler`` (radians) / ``scale`` in the bone's space: a
        sword picked up into a hand, a hat put on a head. It keeps its own
        size, whatever the model's. A ``Transform`` can stand in for all
        three: ``hat.attach("Robot", "Head", at.Transform(position=...))``."""
        if hasattr(position, "rotation_euler"):
            t = position
            position, rotation_euler = t.position, t.rotation_euler
            scale = getattr(t, "scale", scale)
        name = to if isinstance(to, str) else getattr(to, "name", None)
        if not name or not isinstance(name, str):
            raise ValueError("attach() needs a named entity (or a name) to follow")
        self._write("att", [name, str(bone), list(_vec(position, 3)),
                            list(_vec(rotation_euler, 3)), list(_vec(scale, 3))])

    def detach(self) -> None:
        """Stops following a bone, staying where it is (from the next frame)."""
        self._write("att", False)

    @property
    def attached_to(self) -> tuple[str, str] | None:
        """What the entity follows, ``(entity name, bone)``, or None."""
        v = self._read("att", None)
        if not v:
            return None
        return (v[0], v[1])

    @property
    def nav(self) -> "EntityNav | None":
        """This entity's Nav Agent: ``go(target)``, ``stop()``,
        ``arrived``, ``path``, ``speed``. None without one."""
        return EntityNav(self) if self._record.get("nav") is not None else None

    @property
    def ragdoll(self) -> "EntityRagdoll | None":
        """This entity's Ragdoll: ``on()``, ``off()``, ``push(impulse,
        bone)``, ``limp``, ``resting``, ``hips``. None without one."""
        return EntityRagdoll(self) if self._record.get("rag") is not None else None

    @property
    def sprite(self) -> "EntitySprite | None":
        """This entity's Sprite: ``play(clip)``, ``stop()``, ``flip_x``,
        ``frame``, ``color``, ``clip``, ``finished``. None without one."""
        return EntitySprite(self) if self._record.get("spr") is not None else None

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
        if self._joint_w:
            w["joint"] = dict(self._joint_w)
        if self._nav_w:
            w["nav"] = dict(self._nav_w)
        if self._rag_w:
            w["rag"] = dict(self._rag_w)
        if self._spr_w:
            w["spr"] = dict(self._spr_w)
        for key in ("p", "r", "s", "v", "cam"):
            if key in d and self._vals.get(key) is not None:
                w[key] = self._vals[key]
        if "attrs" in d and self._attrs is not None:
            w["attrs"] = dict(self._attrs)
        if "att" in d:
            w["att"] = self._vals.get("att")
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
        self.pos = None
        self.raw = None
        self._finger = None  # the left button as finger 0: [x, y, start_x, start_y]

    def update(self, keys, buttons=(), delta=(0.0, 0.0), locked=False, pos=None):
        self.prev, self.down = self.down, set(keys)
        self.prev_buttons, self.buttons = self.buttons, set(buttons)
        self.delta = (float(delta[0]), float(delta[1]))
        self.locked = bool(locked)
        self.pos = None if pos is None else (float(pos[0]), float(pos[1]))
        if self.button_pressed("left"):
            x, y = self.pos or (0.0, 0.0)
            self._finger = [x, y, x, y]
        elif self._finger is not None and self.pos is not None:
            self._finger[0:2] = self.pos

    def touches(self):
        """The left mouse button as finger 0 (there's no touch screen)."""
        held, released = self.button_down("left"), self.button_released("left")
        if self._finger is None or not (held or released):
            return []
        x, y, sx, sy = self._finger
        return [(0, x, y, sx, sy, self.button_pressed("left"), released, True)]

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

    def mouse_position(self):
        return self.pos


class Touch:
    """A finger on the screen, from ``ctx.input.touches``.

    ``finger`` is a small number that stays the same while it's down (0
    for the first finger; the lowest free number for the next).
    ``position`` and ``start`` (where it landed) are screen pixels, like
    ``ctx.input.mouse_position``. ``just_started`` is True on the frame it
    lands and ``just_ended`` on the frame it lifts (its last frame in the
    list). ``primary`` marks the finger that also works as the left mouse
    button: the first one down."""

    __slots__ = ("finger", "position", "start", "just_started", "just_ended", "primary")

    def __init__(self, finger, position, start, just_started=False, just_ended=False,
                 primary=False):
        self.finger = int(finger)
        self.position = (float(position[0]), float(position[1]))
        self.start = (float(start[0]), float(start[1]))
        self.just_started = bool(just_started)
        self.just_ended = bool(just_ended)
        self.primary = bool(primary)

    @property
    def moved(self) -> tuple[float, float]:
        """How far it has moved since it landed (dx, dy); +y is down."""
        return (self.position[0] - self.start[0], self.position[1] - self.start[1])

    def __repr__(self):
        x, y = self.position
        state = " started" if self.just_started else " ended" if self.just_ended else ""
        return f"Touch(finger={self.finger}, position=({x:g}, {y:g}){state})"


#: How far a thumbstick is pushed (0 to 1) before it holds its key.
STICK_PRESS = 0.4
#: Pushes shorter than this (a fraction of the stick's reach) count as none.
STICK_DEAD_ZONE = 0.12


def _on_touch_screen():
    """Whether touch controls show by default: on Android, or when
    ``$AUTOMATA_TOUCH_CONTROLS`` says so (``1``: show, ``0``: hide)."""
    env = os.environ.get("AUTOMATA_TOUCH_CONTROLS", "").strip().lower()
    if env in ("1", "true", "on", "yes"):
        return True
    if env in ("0", "false", "off", "no"):
        return False
    return sys.platform == "android" or "ANDROID_PRIVATE" in os.environ


def _side(value, what):
    side = str(getattr(value, "value", value)).strip().lower()
    if side not in ("left", "right"):
        raise ValueError(f'{what} is "left" or "right", not {value!r}')
    return side


class _TouchStick:
    __slots__ = ("side", "keys", "finger", "origin", "vector", "rest", "radius")

    def __init__(self, side, keys):
        self.side = side
        self.keys = keys
        self.finger = None
        self.origin = (0.0, 0.0)
        self.vector = (0.0, 0.0)  # +y is up
        self.rest = (0.0, 0.0)
        self.radius = 1.0

    def strengths(self):
        """How hard each key is held, by name (0 to 1)."""
        x, y = self.vector
        if len(self.keys) == 2:
            left, right = self.keys
            return ((left, max(0.0, -x)), (right, max(0.0, x)))
        up, left, down, right = self.keys
        return ((up, max(0.0, y)), (left, max(0.0, -x)), (down, max(0.0, -y)),
                (right, max(0.0, x)))


class _TouchButton:
    __slots__ = ("key", "label", "fingers", "center", "radius")

    def __init__(self, key, label):
        self.key = key
        self.label = label
        self.fingers = set()
        self.center = (0.0, 0.0)
        self.radius = 1.0


class TouchControls:
    """``ctx.input.touch_controls``: thumbsticks and buttons on the screen
    that hold keys, so a keyboard game plays on a phone unchanged.

    A game's ``build.ron`` sets them up (``touch_controls``), or a script
    does with :meth:`set`::

        ctx.input.touch_controls.set(
            sticks=[{"side": "left", "keys": ["forward", "left", "back", "right"]}],
            buttons=[{"key": "jump", "label": "Jump"}],
        )

    A stick holds its keys (up, left, down, right; or just left, right) as
    it's pushed, and appears where the thumb lands on its half of the
    screen. ``ctx.input.axis`` reads how far it's pushed, so walking speeds
    up smoothly. A button holds its key while a finger is on it. Keys may
    be key names (``"space"``) or actions (``"jump"``); an action also
    holds the keys bound to it. ``look`` makes
    dragging on the free part of one side move ``ctx.input.mouse_delta``,
    for mouse look.

    They show on touch screens (Android) by default; ``show="always"``
    shows them everywhere, where the mouse works them as a finger, and
    ``"never"`` waits for a script to set :attr:`visible`. Fingers on them
    aren't in ``ctx.input.touches`` and don't press the left mouse button.
    """

    def __init__(self, input_):
        self._input = input_
        self._sticks = []
        self._buttons = []
        self._look = None
        self._show = "auto"
        self._visible = None  # a script's choice
        self._size = (1280.0, 720.0)
        self._held = {}  # name -> strength (0..1), this frame
        self._now = set()  # names pressed (held hard enough) this frame
        self._prev = set()
        self._look_fingers = {}  # finger -> last position
        self._look_delta = (0.0, 0.0)
        self._left = False  # the primary finger is on a control
        self._owned = set()  # fingers on a control this frame

    def set(self, sticks=(), buttons=(), look=None, show=None) -> None:
        """Replaces the controls. ``sticks`` are ``{"side": "left",
        "keys": ["w", "a", "s", "d"]}`` (up, left, down, right; or two
        keys, left and right), one per side at most. ``buttons`` are
        ``{"key": "space", "label": "Jump"}`` (up to six, placed around the
        bottom-right corner, the first in the corner). ``look`` is a side,
        or None. ``show`` is ``"auto"`` (touch screens), ``"always"`` or
        ``"never"``."""
        parsed = []
        for stick in sticks or ():
            side = _side(stick.get("side", "left"), "a stick's side")
            keys = [str(k) for k in stick.get("keys", ())]
            if len(keys) not in (2, 4) or not all(k.strip() for k in keys):
                raise ValueError(f"a stick has four keys (up, left, down, right) or two "
                                 f"(left, right), not {keys!r}")
            if any(s.side == side for s in parsed):
                raise ValueError(f"there's already a stick on the {side}")
            parsed.append(_TouchStick(side, keys))
        made = []
        for button in buttons or ():
            key = str(button.get("key", "")).strip()
            if not key:
                raise ValueError(f"a button needs a key, like {{'key': 'space'}}: {button!r}")
            label = button.get("label")
            made.append(_TouchButton(key, str(label) if label else key))
        if len(made) > 6:
            raise ValueError(f"up to 6 touch buttons fit, not {len(made)}")
        side = None if look is None or look is False else _side(look, "look")
        if side is not None and any(s.side == side for s in parsed):
            raise ValueError(f"look and a stick can't share the {side} side")
        mode = "auto" if show is None else str(show).strip().lower()
        if mode not in ("auto", "always", "never"):
            raise ValueError(f'show is "auto", "always" or "never", not {show!r}')
        self._release()
        self._sticks, self._buttons, self._look, self._show = parsed, made, side, mode

    @property
    def configured(self) -> bool:
        """Whether there are any controls to show."""
        return bool(self._sticks or self._buttons or self._look)

    @property
    def visible(self) -> bool:
        """Whether they're on the screen and working. Set it to show or
        hide them (a settings screen's "Touch controls" switch)."""
        if not self.configured:
            return False
        if self._visible is not None:
            return self._visible
        return self._show == "always" or (self._show == "auto" and _on_touch_screen())

    @visible.setter
    def visible(self, value: bool) -> None:
        self._visible = bool(value)
        if not self._visible:
            self._release()

    def stick(self, side: str = "left") -> tuple[float, float]:
        """How far the stick on ``side`` is pushed: (x, y) from -1 to 1, +y
        up (away from the player), (0, 0) when no thumb is on it."""
        side = _side(side, "side")
        for s in self._sticks:
            if s.side == side:
                return s.vector
        return (0.0, 0.0)

    def __repr__(self):
        return (f"TouchControls(sticks={len(self._sticks)}, buttons={len(self._buttons)}, "
                f"look={self._look!r}, visible={self.visible})")

    # --- frames -----------------------------------------------------------

    def _release(self):
        self._owned = set()
        for s in self._sticks:
            s.finger, s.vector = None, (0.0, 0.0)
        for b in self._buttons:
            b.fingers.clear()
        self._look_fingers = {}
        self._held = {}
        self._now = set()
        self._left = False

    def _layout(self, size):
        w, h = float(size[0]), float(size[1])
        self._size = (w, h)
        m = min(w, h)
        margin, stick_r, button_r = 0.05 * m, 0.15 * m, 0.085 * m
        cluster = 0.0
        if self._buttons:
            first = self._buttons[0]
            first.radius = button_r * 1.2
            corner = (w - margin - first.radius, h - margin - first.radius)
            first.center = corner
            others = self._buttons[1:]
            gap = 0.035 * m
            dist = first.radius + button_r + gap
            if len(others) > 1:
                step = math.pi / 2 / (len(others) - 1)
                dist = max(dist, (2 * button_r + gap) / (2 * math.sin(step / 2)))
            for i, b in enumerate(others):
                angle = math.pi - (i * math.pi / 2 / max(1, len(others) - 1))
                b.radius = button_r
                b.center = (corner[0] + dist * math.cos(angle),
                            corner[1] - dist * math.sin(angle))
            cluster = (dist + button_r if others else 0.0) + first.radius + margin
        for s in self._sticks:
            s.radius = stick_r
            y = h - margin - stick_r
            if s.side == "left":
                s.rest = (margin + stick_r, y)
            else:
                s.rest = (w - max(margin, cluster) - stick_r, y)

    def _button_at(self, x, y):
        best, best_d = None, None
        for b in self._buttons:
            d = math.hypot(x - b.center[0], y - b.center[1])
            if d <= b.radius * 1.15 and (best_d is None or d < best_d):
                best, best_d = b, d
        return best

    def _claim(self, finger, x, y):
        """Gives a finger that just landed to the control under it."""
        button = self._button_at(x, y)
        if button is not None:
            button.fingers.add(finger)
            return True
        w, h = self._size
        side = "left" if x < w / 2 else "right"
        if y > h * 0.3:
            for s in self._sticks:
                if s.side == side and s.finger is None:
                    r = s.radius
                    # The base stays on the screen however close to an edge
                    # the thumb lands.
                    s.finger = finger
                    s.origin = (min(max(x, r), w - r), min(max(y, r), h - r))
                    return True
        if self._look == side and y > h * 0.15:
            self._look_fingers[finger] = (x, y)
            return True
        return False

    def _update(self, touches, size, ui_has_left):
        """Reads this frame's fingers (``KeySource.touches`` tuples) and
        decides which keys the controls hold."""
        self._prev = self._now
        self._look_delta = (0.0, 0.0)
        if not self.visible:
            if self._held or self._left:
                self._release()
            self._now = set()
            return
        self._layout(size)
        owned = set()
        tapped = set()  # buttons a finger landed on and left this frame
        dx = dy = 0.0
        for finger, x, y, _sx, _sy, started, ended, primary in touches:
            if started and not (primary and ui_has_left):
                for s in self._sticks:
                    if s.finger == finger:
                        s.finger = None  # a new touch reusing the number
                for b in self._buttons:
                    b.fingers.discard(finger)
                self._look_fingers.pop(finger, None)
                self._claim(finger, x, y)
            mine = False
            for s in self._sticks:
                if s.finger == finger:
                    mine = True
                    if ended:
                        s.finger, s.vector = None, (0.0, 0.0)
                    else:
                        ox, oy = s.origin
                        vx, vy = (x - ox) / s.radius, (oy - y) / s.radius
                        length = math.hypot(vx, vy)
                        if length < STICK_DEAD_ZONE:
                            vx = vy = 0.0
                        elif length > 1.0:
                            vx, vy = vx / length, vy / length
                        s.vector = (vx, vy)
            for b in self._buttons:
                if finger in b.fingers:
                    mine = True
                    if ended:
                        b.fingers.discard(finger)
                        if started:
                            tapped.add(id(b))
            if finger in self._look_fingers:
                mine = True
                lx, ly = self._look_fingers[finger]
                dx, dy = dx + x - lx, dy + y - ly
                if ended:
                    del self._look_fingers[finger]
                else:
                    self._look_fingers[finger] = (x, y)
            if mine:
                owned.add(finger)
                if primary:
                    self._left = not ended
            elif primary:
                self._left = False
        # Fingers that vanished without an end (focus lost) let go.
        present = {t[0] for t in touches}
        for s in self._sticks:
            if s.finger is not None and s.finger not in present:
                s.finger, s.vector = None, (0.0, 0.0)
        for b in self._buttons:
            b.fingers &= present
        if not present:
            self._left = False
        self._owned = owned
        self._look_delta = (dx, dy)
        held = {}
        for s in self._sticks:
            for name, strength in s.strengths():
                if strength > 0.0:
                    held[name] = max(held.get(name, 0.0), strength)
        for b in self._buttons:
            if b.fingers or id(b) in tapped:
                held[b.key] = 1.0
        # Each name also under the key names it stands for: "shift" is
        # either Shift, and an action holds the keys bound to it, as if the
        # player pressed them.
        self._held = {}
        settings = self._input._settings if self._input is not None else None
        for name, strength in held.items():
            names = [name, *keys_for(name)]
            if settings is not None and settings.is_action(name):
                for k in settings.keys(name):
                    names.extend(keys_for(k))
            for n in names:
                self._held[n] = max(self._held.get(n, 0.0), strength)
        self._now = {n for n, v in self._held.items() if v >= STICK_PRESS}

    def _owns(self, finger):
        return finger in self._owned

    @property
    def _live(self):
        return bool(self._now or self._prev or self._held)

    def _down(self, name):
        return name in self._now

    def _pressed(self, name):
        return name in self._now and name not in self._prev

    def _released(self, name):
        return name in self._prev and name not in self._now

    def _draw(self, hud):
        """Draws the controls over everything else this frame."""
        if not self.visible:
            return
        white = (1.0, 1.0, 1.0)
        for s in self._sticks:
            active = s.finger is not None
            ox, oy = s.origin if active else s.rest
            r = s.radius
            base = 2 * r
            hud.image("builtin:circle", ox, oy, (base, base), (*white, 0.14 if active else 0.08))
            hud.image("builtin:ring", ox, oy, (base, base), (*white, 0.45 if active else 0.25))
            vx, vy = s.vector
            knob = 0.9 * r
            hud.image("builtin:circle", ox + vx * r * 0.6, oy - vy * r * 0.6, (knob, knob),
                      (*white, 0.55 if active else 0.3))
        for b in self._buttons:
            down = bool(b.fingers)
            x, y = b.center
            d = 2 * b.radius
            hud.image("builtin:circle", x, y, (d, d), (*white, 0.4 if down else 0.15))
            hud.image("builtin:ring", x, y, (d, d), (*white, 0.7 if down else 0.4))
            size = min(0.5 * b.radius, 1.5 * b.radius / max(1.0, _text_ems(b.label)))
            hud.text(b.label, x, y, size, (*white, 0.95 if down else 0.8), "center",
                     shadow=True)


class Input:
    """``ctx.input``: keys, mouse buttons, touches, mouse movement and capture."""

    def __init__(self, source):
        self._src = source
        self._lock_request = None
        self._settings: Settings | None = None  # ctx.settings, for action names
        self._ui: Ui | None = None  # ctx.ui, which keeps clicks on widgets
        #: On-screen thumbsticks and buttons (see :class:`TouchControls`).
        self.touch_controls = TouchControls(self)

    def _game(self, on_button):
        """``on_button`` minus the buttons ``ctx.ui`` took (pressed on a
        widget or panel, until released) and the left button while the
        first finger is on a touch control."""
        ui = self._ui
        captured = ui._captured if ui is not None else ()
        touch = self.touch_controls._left
        if not captured and not touch:
            return on_button
        return lambda b: (b not in captured and not (touch and b == "left")
                          and on_button(b))

    def _begin_touch(self):
        """Lets the touch controls take this frame's fingers (after
        ``ctx.ui`` has taken its own)."""
        fn = getattr(self._src, "touches", None)
        raw = list(fn()) if fn is not None else []
        ui = self._ui
        self.touch_controls._update(raw, ui._hud.size if ui is not None else (1280, 720),
                                    ui is not None and "left" in ui._captured)

    def _strength(self, name):
        """How hard the touch controls hold ``name`` (a key or an action),
        0 to 1."""
        held = self.touch_controls._held
        if not held:
            return 0.0
        best = max((held.get(n, 0.0) for n in keys_for(name)), default=0.0)
        best = max(best, held.get(name, 0.0))
        settings = self._settings
        if settings is not None and isinstance(name, str) and settings.is_action(name):
            for k in settings.keys(name):
                best = max(best, max((held.get(n, 0.0) for n in keys_for(k)), default=0.0))
        return best

    def _keys(self, on_key):
        """``on_key`` minus the keys ``ctx.ui`` used to navigate (until
        released)."""
        ui = self._ui
        if ui is None or not ui._taken:
            return on_key
        return lambda k: k not in ui._taken and on_key(k)

    def _any(self, key, on_key, on_button, on_touch=None):
        on_button = self._game(on_button)
        on_key = self._keys(on_key)
        if on_touch is not None and self.touch_controls._live:
            if on_touch(str(getattr(key, "value", key)).lower()):
                return True
            keys, buttons = on_key, on_button
            on_key = lambda k: keys(k) or on_touch(k)  # noqa: E731
            on_button = lambda b: buttons(b) or on_touch("mouse_" + b)  # noqa: E731
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
        t = self.touch_controls
        return self._any(key, self._src.key_down, self._src.button_down, t._down)

    def just_pressed(self, key: str) -> bool:
        """The key (or action) went down this frame."""
        t = self.touch_controls
        return self._any(key, self._src.key_pressed, self._src.button_pressed, t._pressed)

    def just_released(self, key: str) -> bool:
        """The key (or action) came up this frame."""
        t = self.touch_controls
        return self._any(key, self._src.key_released, self._src.button_released, t._released)

    def axis(self, negative: str, positive: str) -> float:
        """-1, 0, or 1 from two keys (or actions). A touch thumbstick
        holding them gives how far it's pushed, in between."""
        if self.touch_controls._held:
            analog = self._strength(positive) - self._strength(negative)
            if analog:
                down = self._src.key_down
                keys = (float(self._any(positive, down, self._src.button_down))
                        - float(self._any(negative, down, self._src.button_down)))
                return max(-1.0, min(1.0, keys + analog))
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
        """The mouse button is held. A press that began on ``ctx.ui``
        (a button, slider, panel…) belongs to the UI: it reads False
        here until it's released, so clicking a button doesn't also fire."""
        b = str(button).lower()
        return (self._game(self._src.button_down)(b)
                or self.touch_controls._down("mouse_" + b))

    def mouse_just_pressed(self, button: str = "left") -> bool:
        """The button went down this frame (not on ``ctx.ui``). A touch
        button holding ``"mouse_left"`` presses it too."""
        b = str(button).lower()
        return (self._game(self._src.button_pressed)(b)
                or self.touch_controls._pressed("mouse_" + b))

    def mouse_just_released(self, button: str = "left") -> bool:
        b = str(button).lower()
        return (self._game(self._src.button_released)(b)
                or self.touch_controls._released("mouse_" + b))

    @property
    def mouse_position(self) -> tuple[float, float] | None:
        """Where the cursor is, in screen pixels from the top-left (the
        same space as ``ctx.hud`` and ``ctx.ui``). None when it's outside
        the game view or captured (``lock_mouse``)."""
        fn = getattr(self._src, "mouse_position", None)
        pos = fn() if fn is not None else None
        if pos is None:
            return None
        return (float(pos[0]), float(pos[1]))

    @property
    def touches(self) -> list[Touch]:
        """The fingers on the screen, in the order they landed, plus any
        that lifted this frame (``just_ended``). On a computer the held
        left mouse button is finger 0, so touch controls work with a
        mouse too. A finger that landed on ``ctx.ui`` belongs to the UI
        and isn't listed, like a click on a button. Nor are fingers on the
        touch controls (``touch_controls``)."""
        fn = getattr(self._src, "touches", None)
        raw = fn() if fn is not None else ()
        ui = self._ui
        taken = ui is not None and "left" in ui._captured
        controls = self.touch_controls
        return [Touch(f, (x, y), (sx, sy), started, ended, primary)
                for f, x, y, sx, sy, started, ended, primary in raw
                if not (primary and taken) and not controls._owns(f)]

    @property
    def mouse_over_ui(self) -> bool:
        """The cursor is over ``ctx.ui`` drawn last frame (a widget or a
        panel), or dragging one of its sliders."""
        return self._ui is not None and self._ui._over

    @property
    def mouse_delta(self) -> tuple[float, float]:
        """Mouse movement since last frame (dx, dy) in pixels; +y is down.
        Dragging on the touch controls' ``look`` side adds to it."""
        dx, dy = self._src.mouse_delta()
        lx, ly = self.touch_controls._look_delta
        return (float(dx) + lx, float(dy) + ly)

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


def _nav_point(p, what="a point"):
    """A point from a point or anything with a position (an Entity)."""
    if isinstance(p, Entity):
        p = p.transform.position
    elif hasattr(p, "position") and not isinstance(p, (tuple, list)):
        p = p.position
    try:
        x, y, z = (float(v) for v in tuple(p)[:3])
    except (TypeError, ValueError):
        raise TypeError(f"expected {what}: (x, y, z) or an entity") from None
    if not all(math.isfinite(v) for v in (x, y, z)):
        raise ValueError(f"{what} must be finite")
    return (x, y, z)


class EntityNav:
    """``entity.nav``: a Nav Agent finds its own way around the navigation
    mesh. ``go(target)`` sends it to a point, an entity (followed as it
    moves) or an entity's name; it walks the shortest way, steering round
    corners and other agents and jumping the mesh's links, and stops
    ``stop_distance`` short. Reads show the start of the frame; changes
    apply after it."""

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def _rec(self):
        return self._e._record.get("nav") or {}

    def _set(self, key, value):
        e = self._e
        if e._nav_w is None:
            e._nav_w = {}
        e._nav_w[key] = value
        e._dirty.add("nav")

    def go(self, target) -> None:
        """Heads for ``target``: a point, an entity or its name (None
        stops). A new target finds the way afresh; the same one again
        changes nothing, so calling it every frame is fine."""
        if target is None:
            self._set("go", None)
        elif isinstance(target, str):
            self._set("go", target)
        elif isinstance(target, Entity) and target.name:
            self._set("go", target.name)
        else:
            self._set("go", list(_nav_point(target, "a target")))

    def stop(self) -> None:
        """Stops where it is (it slows to a halt)."""
        self._set("go", None)

    @property
    def target(self):
        """Where it's heading: a point, an entity name, or None."""
        w = self._e._nav_w
        if w is not None and "go" in w:
            t = w["go"]
        else:
            t = self._rec().get("t")
        return _tuple(t) if isinstance(t, list) else t

    @property
    def arrived(self) -> bool:
        """True once it's within ``stop_distance`` of its target (and
        while it has none)."""
        return bool(self._rec().get("arr", True))

    @property
    def moving(self) -> bool:
        """True while it's on its way somewhere."""
        return self.target is not None and not self.arrived

    @property
    def jumping(self) -> bool:
        """True in the middle of a jump link."""
        return bool(self._rec().get("jmp", False))

    @property
    def velocity(self) -> Vec3:
        """How it's moving (m/s)."""
        return _tuple(self._rec().get("vel") or (0.0, 0.0, 0.0))

    @property
    def path(self) -> list:
        """The corners still ahead, as points (the target last)."""
        return [tuple(p[:3]) for p in self._rec().get("path") or ()]

    @property
    def remaining_distance(self) -> float:
        """How far it still has to go along its path (m)."""
        pts = [self._e.transform.position, *self.path]
        return sum(math.dist(a, b) for a, b in zip(pts, pts[1:]))

    @property
    def speed(self) -> float:
        """Top speed (m/s)."""
        w = self._e._nav_w
        if w is not None and "speed" in w:
            return w["speed"]
        return float(self._rec().get("spd", 0.0))

    @speed.setter
    def speed(self, v: float) -> None:
        v = float(v)
        if not (math.isfinite(v) and v >= 0):
            raise ValueError("speed must be a finite number, 0 or more")
        self._set("speed", v)

    @property
    def stop_distance(self) -> float:
        """How near the target counts as arrived (m)."""
        w = self._e._nav_w
        if w is not None and "stop_distance" in w:
            return w["stop_distance"]
        return float(self._rec().get("stop", 0.0))

    @stop_distance.setter
    def stop_distance(self, v: float) -> None:
        v = float(v)
        if not (math.isfinite(v) and v >= 0):
            raise ValueError("stop_distance must be a finite number, 0 or more")
        self._set("stop_distance", v)

    @property
    def hover(self) -> float:
        """How high it floats above the mesh (m), if it isn't a character:
        change it every frame to bob."""
        w = self._e._nav_w
        if w is not None and "hover" in w:
            return w["hover"]
        return float(self._rec().get("hov", 0.0))

    @hover.setter
    def hover(self, v: float) -> None:
        v = float(v)
        if not math.isfinite(v):
            raise ValueError("hover must be a finite number")
        self._set("hover", v)

    def __repr__(self):
        return f"EntityNav({self._e.name!r}, target={self.target!r}, arrived={self.arrived})"


def _rag_number(name, v, lo, hi):
    v = float(v)
    if not (math.isfinite(v) and lo <= v <= hi):
        raise ValueError(f"{name} must be a number from {lo} to {hi}")
    return v


class EntityRagdoll:
    """``entity.ragdoll``: the animated model can go limp, its skeleton
    thrown about by physics (a capsule per big bone, joined at the knees,
    elbows, hips and so on). ``off()`` hands it back to the animation,
    which blends in over ``blend_time`` seconds with the entity standing
    where the hips came down. Reads show the start of the frame; changes
    apply after it::

        if hit:
            enemy.ragdoll.on(push=(0, 2, -6))   # knocked flying
        if enemy.ragdoll.resting:
            enemy.ragdoll.off()                 # back on its feet
    """

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def _rec(self):
        return self._e._record.get("rag") or {}

    def _set(self, key, value):
        e = self._e
        if e._rag_w is None:
            e._rag_w = {}
        e._rag_w[key] = value
        e._dirty.add("rag")

    def _get(self, key, rec_key, default):
        w = self._e._rag_w
        if w is not None and key in w:
            return w[key]
        return self._rec().get(rec_key, default)

    def on(self, push=None, bone: str | None = None) -> None:
        """Goes limp (from the next physics step), keeping the motion it
        had. ``push`` (an impulse in kg·m/s, ``(x, y, z)``) shoves it as
        it goes: on ``bone`` if named, else the whole body."""
        self._set("on", True)
        if push is not None:
            self.push(push, bone)

    def off(self) -> None:
        """Gets back to the animation: it blends in over ``blend_time``
        seconds, the entity standing where the hips came down."""
        self._set("on", False)

    def push(self, impulse, bone: str | None = None) -> None:
        """Shoves the limp body with ``impulse`` (kg·m/s, ``(x, y, z)``):
        on ``bone`` (a bone's name, like ``"Head"``) or, by default, the
        whole body. Pushes while it isn't limp do nothing."""
        x, y, z = _vec(impulse, 3)
        if not all(math.isfinite(v) for v in (x, y, z)):
            raise ValueError("a push must be finite")
        pushes = (self._e._rag_w or {}).get("push") or []
        self._set("push", [*pushes, [[x, y, z], None if bone is None else str(bone)]])

    @property
    def active(self) -> bool:
        """Whether it's switched on (limp, or about to be). Settable."""
        return bool(self._get("on", "on", False))

    @active.setter
    def active(self, on: bool) -> None:
        self._set("on", bool(on))

    @property
    def limp(self) -> bool:
        """Whether physics has the skeleton right now."""
        return bool(self._rec().get("limp", False))

    @property
    def resting(self) -> bool:
        """Whether it's limp and has come to rest: a good time for
        ``off()``."""
        return bool(self._rec().get("rest", False))

    @property
    def weight(self) -> float:
        """How much of the pose is the ragdoll's: 1 limp, falling to 0 as
        the animation takes over again."""
        return float(self._rec().get("w", 0.0))

    @property
    def blending(self) -> bool:
        """Whether the animation is taking over again right now."""
        return not self.limp and self.weight > 0.0

    @property
    def hips(self) -> Vec3 | None:
        """Where the hips are while it's limp or blending back (cameras
        follow this), else None."""
        h = self._rec().get("hips")
        return None if h is None else _tuple(h)

    @property
    def mass(self) -> float:
        """The whole body's mass (kg). Settable (from the next time it
        goes limp)."""
        return float(self._get("mass", "mass", 70.0))

    @mass.setter
    def mass(self, v: float) -> None:
        self._set("mass", _rag_number("mass", v, 1.0, 10000.0))

    @property
    def stiffness(self) -> float:
        """How hard the joints hold the pose it went limp in: 0 a rag
        doll, 1 stiff. Settable, even while limp."""
        return float(self._get("stiffness", "stiff", 0.0))

    @stiffness.setter
    def stiffness(self, v: float) -> None:
        self._set("stiffness", _rag_number("stiffness", v, 0.0, 1.0))

    @property
    def blend_time(self) -> float:
        """Seconds the animation takes to take over again. Settable."""
        return float(self._get("blend_time", "blend", 0.5))

    @blend_time.setter
    def blend_time(self, v: float) -> None:
        self._set("blend_time", _rag_number("blend_time", v, 0.0, 10.0))

    def __repr__(self):
        return f"EntityRagdoll({self._e.name!r}, limp={self.limp})"


class EntitySprite:
    """``entity.sprite``: the entity's 2D image. Sprite sheets play their
    clips (named in the Inspector or ``at.Sprite(clips=...)``). Reads show
    the start of the frame (or what this frame set); changes apply after
    it::

        if abs(vx) > 0.1:
            entity.sprite.play("run")
            entity.sprite.flip_x = vx < 0
        else:
            entity.sprite.play("idle")
    """

    __slots__ = ("_e",)

    def __init__(self, entity):
        self._e = entity

    def _rec(self):
        return self._e._record.get("spr") or {}

    def _set(self, key, value):
        e = self._e
        if e._spr_w is None:
            e._spr_w = {}
        e._spr_w[key] = value
        e._dirty.add("spr")

    def _get(self, key, rec_key, default):
        w = self._e._spr_w
        if w is not None and key in w:
            return w[key]
        return self._rec().get(rec_key, default)

    def play(self, clip: str, restart: bool = False) -> None:
        """Plays the clip called ``clip``. Asking for the clip that's
        already playing carries on with it (call it every frame), unless
        ``restart``."""
        clip = str(clip)
        clips = self.clips
        if clips and clip not in clips:
            raise ValueError(f"no clip {clip!r}; the sprite has {', '.join(clips)}")
        w = self._e._spr_w
        if w is not None:
            w.pop("stop", None)
            w.pop("frame", None)
        self._set("play", [clip, bool(restart)])

    def stop(self) -> None:
        """Stops the clip, holding the frame it's on."""
        w = self._e._spr_w
        if w is not None:
            w.pop("play", None)
        self._set("stop", True)

    @property
    def clip(self) -> str | None:
        """The clip playing (None when stopped)."""
        w = self._e._spr_w or {}
        if "play" in w:
            return w["play"][0]
        if "stop" in w or "frame" in w:
            return None
        return self._rec().get("clip")

    @property
    def clips(self) -> tuple[str, ...]:
        """The names of the sprite's clips."""
        return tuple(self._rec().get("clips") or ())

    @property
    def finished(self) -> bool:
        """Whether a clip that doesn't loop has reached its end."""
        return bool(self._rec().get("done", False))

    @property
    def frame(self) -> int:
        """The frame shown (counting left to right, top to bottom).
        Setting it stops the clip."""
        return int(self._get("frame", "frame", 0))

    @frame.setter
    def frame(self, v: int) -> None:
        v = int(v)
        if v < 0:
            raise ValueError("frame must be 0 or more")
        w = self._e._spr_w
        if w is not None:
            w.pop("play", None)
        self._set("frame", v)

    @property
    def flip_x(self) -> bool:
        """Mirrored left to right (face the way you walk). Settable."""
        return bool(self._get("flip_x", "fx", False))

    @flip_x.setter
    def flip_x(self, on: bool) -> None:
        self._set("flip_x", bool(on))

    @property
    def flip_y(self) -> bool:
        """Mirrored top to bottom. Settable."""
        return bool(self._get("flip_y", "fy", False))

    @flip_y.setter
    def flip_y(self, on: bool) -> None:
        self._set("flip_y", bool(on))

    @property
    def color(self) -> tuple:
        """The tint ``(r, g, b, a)``; lower ``a`` to fade it. Settable."""
        return tuple(self._get("color", "color", (1.0, 1.0, 1.0, 1.0)))

    @color.setter
    def color(self, c) -> None:
        c = [float(v) for v in c]
        if len(c) == 3:
            c.append(1.0)
        if len(c) != 4:
            raise ValueError("color is (r, g, b) or (r, g, b, a)")
        self._set("color", c)

    @property
    def layer(self) -> int:
        """Drawing order (higher on top). Settable."""
        return int(self._get("layer", "layer", 0))

    @layer.setter
    def layer(self, v: int) -> None:
        self._set("layer", int(v))

    @property
    def image(self) -> str:
        """The image file. Settable (another file from the project)."""
        return str(self._get("image", "image", ""))

    @image.setter
    def image(self, path: str) -> None:
        self._set("image", str(path))

    @property
    def size(self) -> tuple:
        """Width and height in world units. Settable."""
        return tuple(self._get("size", "size", (1.0, 1.0)))

    @size.setter
    def size(self, wh) -> None:
        w, h = (float(v) for v in wh)
        if not (w > 0 and h > 0 and math.isfinite(w) and math.isfinite(h)):
            raise ValueError("a sprite's size must be positive")
        self._set("size", [w, h])

    def __repr__(self):
        return f"EntitySprite({self._e.name!r}, clip={self.clip!r}, frame={self.frame})"


class Nav:
    """``ctx.nav``: questions about the navigation mesh, which is baked
    from the level's static colliders (Oberon's Bake button, or when the
    game starts). Points may be entities."""

    def __init__(self, host):
        self._host = host

    @property
    def ready(self) -> bool:
        """True when the scene has a navigation mesh."""
        return bool(self._host.backend.nav_ready())

    def path(self, start, end, partial: bool = False) -> list | None:
        """The shortest walkable way from ``start`` to ``end`` as a list
        of corner points, both ends included (snapped onto the mesh), or
        None when there's no way. With ``partial``, an unreachable
        ``end`` gives the way to the nearest point that can be reached."""
        a = _nav_point(start, "start")
        b = _nav_point(end, "end")
        found = self._host.backend.nav_path(a, b, bool(partial))
        if found is None:
            return None
        return [tuple(float(v) for v in p[:3]) for p in found]

    def path_length(self, start, end) -> float | None:
        """How far it is to walk from ``start`` to ``end``, or None."""
        p = self.path(start, end)
        if p is None:
            return None
        return sum(math.dist(a, b) for a, b in zip(p, p[1:]))

    def nearest(self, point, max_distance: float = 5.0) -> Vec3 | None:
        """The nearest point on the mesh to ``point``, within
        ``max_distance`` metres, or None."""
        p = self._host.backend.nav_nearest(_nav_point(point), float(max_distance))
        return None if p is None else tuple(float(v) for v in p[:3])

    def reachable(self, start, end) -> bool:
        """True when there's a walkable way from ``start`` to ``end``."""
        return self.path(start, end) is not None


class NavGraph:
    """Path finding over a navigation mesh given as plain numbers (what
    Oberon sends its script bridge; the engine answers ``ctx.nav`` itself).

    ``data`` is ``{"polys": [[x0, z0, x1, z1, y0, dx, dz], ...], "edges":
    [[[to, ax, ay, az, bx, by, bz, link], ...], ...]}``: rectangles with
    sloping planes, and each one's ways out (shared edges, or links from
    ``a`` to ``b``). It matches the engine's search: A* over polygons, then
    the funnel pulled tight between links."""

    BELOW, ABOVE, SNAP, LINK_PENALTY = 4.0, 1.0, 2.0, 1.0

    def __init__(self, data):
        self.polys = [tuple(float(v) for v in p) for p in data.get("polys", ())]
        self.edges = [[(int(e[0]), tuple(e[1:4]), tuple(e[4:7]), bool(e[7])) for e in es]
                      for es in data.get("edges", ())]
        while len(self.edges) < len(self.polys):
            self.edges.append([])

    def _h(self, i, x, z):
        x0, z0, _, _, y0, dx, dz = self.polys[i]
        return y0 + dx * (x - x0) + dz * (z - z0)

    def locate(self, p, below=BELOW, above=ABOVE, snap=SNAP):
        """``(polygon, point on it)`` under ``p`` (or within ``snap``
        metres sideways), or None."""
        x, y, z = p
        best = None
        for i, (x0, z0, x1, z1, *_rest) in enumerate(self.polys):
            if x0 <= x < x1 and z0 <= z < z1:
                h = self._h(i, x, z)
                if y - below <= h <= y + above:
                    score = y - h if h <= y else (h - y) * 3.0
                    if best is None or score < best[0]:
                        best = (score, i, (x, h, z))
        if best is not None:
            return best[1], best[2]
        if snap <= 0:
            return None
        near = None
        for i, (x0, z0, x1, z1, *_rest) in enumerate(self.polys):
            inset = min(0.002, (x1 - x0) * 0.25)
            cx = min(max(x, x0 + inset), x1 - inset)
            cz = min(max(z, z0 + inset), z1 - inset)
            d = math.hypot(cx - x, cz - z)
            if d > snap:
                continue
            h = self._h(i, cx, cz)
            if not (y - below <= h <= y + above):
                continue
            dist = d + abs(y - h) * 0.25
            if near is None or dist < near[0]:
                near = (dist, i, (cx, h, cz))
        return None if near is None else (near[1], near[2])

    def nearest(self, p, max_distance=5.0):
        r = max(0.0, max_distance)
        found = self.locate(p, max(self.BELOW, r), max(self.ABOVE, r), r)
        return None if found is None else found[1]

    @staticmethod
    def _closest_on(p, a, b):
        ab = [b[k] - a[k] for k in range(3)]
        n = sum(v * v for v in ab)
        if n <= 1e-12:
            return a
        t = min(1.0, max(0.0, sum((p[k] - a[k]) * ab[k] for k in range(3)) / n))
        return tuple(a[k] + ab[k] * t for k in range(3))

    def _center(self, i):
        x0, z0, x1, z1, *_rest = self.polys[i]
        x, z = (x0 + x1) * 0.5, (z0 + z1) * 0.5
        return (x, self._h(i, x, z), z)

    def path(self, start, end, partial=False):
        """Corners from ``start`` to ``end`` as ``(x, y, z, link)``, or None."""
        import heapq

        s = self.locate(start)
        if s is None:
            return None
        g = self.locate(end)
        if g is None and not partial:
            return None
        target = end if g is None else g[1]
        goal = -1 if g is None else g[0]
        n = len(self.polys)
        cost = [math.inf] * n
        entry = [None] * n
        parent = [(-1, -1)] * n
        closed = [False] * n
        si, a = s
        cost[si], entry[si] = 0.0, a
        heap = [(math.dist(a, target), si)]
        best = (si, math.dist(a, target))
        reached = False
        while heap:
            _, i = heapq.heappop(heap)
            if closed[i]:
                continue
            closed[i] = True
            if i == goal:
                reached = True
                best = (i, 0.0)
                break
            here = entry[i]
            hd = math.dist(here, target)
            if hd < best[1]:
                best = (i, hd)
            for k, (j, ea, eb, link) in enumerate(self.edges[i]):
                if closed[j]:
                    continue
                if link:
                    e, c = eb, math.dist(here, ea) + math.dist(ea, eb) + self.LINK_PENALTY
                else:
                    e = self._closest_on(here, ea, eb)
                    c = math.dist(here, e)
                if cost[i] + c < cost[j]:
                    cost[j], entry[j], parent[j] = cost[i] + c, e, (i, k)
                    heapq.heappush(heap, (cost[j] + math.dist(e, target), j))
        if not reached and not partial:
            return None
        end_poly = best[0]
        if reached:
            fin = target
        else:
            x0, z0, x1, z1, *_rest = self.polys[end_poly]
            cx, cz = min(max(target[0], x0), x1), min(max(target[2], z0), z1)
            fin = (cx, self._h(end_poly, cx, cz), cz)
        chain = []
        at = end_poly
        while at >= 0 and len(chain) <= n:
            chain.append((at, parent[at][1]))
            at = parent[at][0]
        chain.reverse()
        return self._pull(chain, a, fin)

    def _pull(self, chain, start, end):
        out = []
        portals = [(start, start)]

        def flush(to):
            portals.append((to, to))
            for k, c in enumerate(_string_pull(portals)):
                if k == 0 and out:
                    continue
                out.append((*c, False))
            portals.clear()

        for (frm, _), (to, edge) in zip(chain, chain[1:]):
            _, ea, eb, link = self.edges[frm][edge]
            if link:
                flush(ea)
                out.append((*eb, True))
                portals.append((eb, eb))
                continue
            c0, c1 = self._center(frm), self._center(to)
            dx, dz = c1[0] - c0[0], c1[2] - c0[2]
            mx, mz = (ea[0] + eb[0]) * 0.5, (ea[2] + eb[2]) * 0.5
            side = dx * (ea[2] - mz) - dz * (ea[0] - mx)
            portals.append((ea, eb) if side > 0 else (eb, ea))
        flush(end)
        return out


def _string_pull(portals):
    """The funnel algorithm over ``(left, right)`` portals (seen from
    above), the first and last being the ends."""

    def area(a, b, c):
        return (c[0] - a[0]) * (b[2] - a[2]) - (b[0] - a[0]) * (c[2] - a[2])

    def same(a, b):
        return (a[0] - b[0]) ** 2 + (a[2] - b[2]) ** 2 < 1e-8

    start = portals[0][0]
    apex = left = right = start
    li = ri = 0
    out = [start]
    i = 1
    while i < len(portals):
        lp, rp = portals[i]
        if area(apex, right, rp) <= 0.0:
            if same(apex, right) or area(apex, left, rp) > 0.0:
                right, ri = rp, i
            else:
                apex = left
                out.append(apex)
                left = right = apex
                ri = li
                i = li + 1
                continue
        if area(apex, left, lp) >= 0.0:
            if same(apex, left) or area(apex, right, lp) < 0.0:
                left, li = lp, i
            else:
                apex = right
                out.append(apex)
                left = right = apex
                li = ri
                i = ri + 1
                continue
        i += 1
    end = portals[-1][0]
    last = out[-1]
    if not same(last, end) or abs(last[1] - end[1]) > 1e-3:
        out.append(end)
    return out


# --------------------------------------------------------------------------
# AI helpers: state machines, behaviour trees, sight and hearing (ctx.ai)
# --------------------------------------------------------------------------

#: A behaviour tree node finished well.
SUCCESS = "success"
#: A behaviour tree node finished badly (or its condition is false).
FAILURE = "failure"
#: A behaviour tree node isn't done yet: tick it again next frame.
RUNNING = "running"
_STATUSES = (SUCCESS, FAILURE, RUNNING)


def _call_state_fn(fn, entity, ctx, machine):
    """Calls a state's function with as many of (entity, ctx, machine) as
    it takes."""
    if fn is None:
        return None
    try:
        code = fn.__code__
        n = 3 if code.co_flags & 0x04 else code.co_argcount  # *args: all three
        if hasattr(fn, "__self__"):
            n -= 1
    except AttributeError:
        n = 3
    if n >= 3:
        return fn(entity, ctx, machine)
    if n == 2:
        return fn(entity, ctx)
    if n == 1:
        return fn(entity)
    return fn()


class StateMachine:
    """A finite state machine for one entity: it's always in exactly one
    state, and each state has its own ``update`` (plus optional ``enter``
    and ``exit``). Make one per entity in ``start`` and keep it in
    ``entity.state``::

        def patrol(entity, ctx):
            ...
            if ctx.ai.can_see(entity, "Player"):
                return "chase"          # returning a name switches state

        def chase(entity, ctx):
            entity.nav.go("Player")

        def start(entity, ctx):
            entity.state["brain"] = ctx.ai.state_machine(
                {"patrol": patrol, "chase": chase}, initial="patrol")

        def update(entity, ctx):
            entity.state["brain"].update(entity, ctx)

    A state is a function (its ``update``), or a dict or object with
    ``enter``, ``update`` and ``exit`` (any may be missing). Each is called
    with ``(entity, ctx)``, or ``(entity, ctx, machine)`` if it takes three
    arguments. Whatever ``update`` returns that's a state name switches to
    it (``exit`` of the old, then ``enter`` of the new, in the same frame).
    """

    def __init__(self, states: dict | None = None, initial: str | None = None,
                 on_change: Callable | None = None):
        self._states = {}
        self._transitions = []
        for name, spec in (states or {}).items():
            self.add(name, spec)
        if initial is not None and initial not in self._states:
            raise KeyError(f"no state named {initial!r}{_did_you_mean(initial, self._states)}")
        self._initial = initial if initial is not None else next(iter(self._states), None)
        #: Called as ``on_change(entity, ctx, old, new)`` on every switch.
        self.on_change = on_change
        self._state = None
        self._previous = None
        self._since = 0.0
        self._entered = False
        self._pending = None
        self._entity = None
        self._ctx = None
        self._history = []

    def add(self, name: str, spec=None, *, enter=None, update=None, exit=None) -> "StateMachine":
        """Adds (or replaces) a state. ``spec`` is a function (the state's
        update), or a dict or object with ``enter``/``update``/``exit``.
        Returns the machine, so adds chain."""
        if not isinstance(name, str) or not name:
            raise TypeError(f"state names are non-empty strings, not {name!r}")
        parts = {"enter": None, "update": None, "exit": None}
        if callable(spec) and not isinstance(spec, type):
            parts["update"] = spec
        elif isinstance(spec, dict):
            unknown = set(spec) - set(parts)
            if unknown:
                raise KeyError(f"state {name!r}: unknown keys {sorted(unknown)} "
                               "(use enter, update, exit)")
            parts.update(spec)
        elif spec is not None:
            for k in parts:
                parts[k] = getattr(spec, k, None)
        for k, v in (("enter", enter), ("update", update), ("exit", exit)):
            if v is not None:
                parts[k] = v
        self._states[name] = parts
        if getattr(self, "_initial", None) is None:
            self._initial = name
        return self

    def when(self, source: str, target: str, condition: Callable) -> "StateMachine":
        """Switches from ``source`` (``"*"``: any state) to ``target``
        whenever ``condition(entity, ctx)`` is true, checked in order before
        the state's update. Returns the machine."""
        for s in (source, target):
            if s != "*" and s not in self._states:
                raise KeyError(f"no state named {s!r}{_did_you_mean(s, self._states)}")
        if target == "*":
            raise ValueError("a transition's target must be a state, not '*'")
        self._transitions.append((source, target, condition))
        return self

    @property
    def state(self) -> str | None:
        """The current state's name (None before the first update)."""
        return self._state if self._entered else self._initial

    @property
    def previous(self) -> str | None:
        """The state before this one, or None."""
        return self._previous

    @property
    def time(self) -> float:
        """Seconds spent in the current state."""
        return self._since

    @property
    def states(self) -> list[str]:
        return list(self._states)

    @property
    def history(self) -> list[str]:
        """The last few states entered, oldest first (for debugging)."""
        return list(self._history)

    def is_in(self, *names: str) -> bool:
        """True when the current state is one of ``names``."""
        return self.state in names

    def go(self, name: str) -> None:
        """Switches to ``name`` now (from inside a state, or from outside
        after the first update); before the first update, starts there."""
        if name not in self._states:
            raise KeyError(f"no state named {name!r}{_did_you_mean(name, self._states)}")
        if not self._entered:
            self._initial = name
            return
        self._switch(name)

    def _switch(self, name):
        entity, ctx = self._entity, self._ctx
        old = self._state
        _call_state_fn(self._states[old]["exit"], entity, ctx, self)
        self._previous, self._state, self._since = old, name, 0.0
        self._history = (self._history + [name])[-16:]
        if self.on_change is not None:
            self.on_change(entity, ctx, old, name)
        _call_state_fn(self._states[name]["enter"], entity, ctx, self)

    def update(self, entity, ctx) -> str | None:
        """Runs one frame: the first update enters the initial state, then
        the transitions are checked and the current state's ``update``
        runs. Returns the state it's in afterwards."""
        self._entity, self._ctx = entity, ctx
        if not self._states:
            return None
        if not self._entered:
            self._entered = True
            self._state = self._initial
            self._history = [self._initial]
            _call_state_fn(self._states[self._state]["enter"], entity, ctx, self)
        else:
            self._since += float(getattr(getattr(ctx, "time", None), "delta", 0.0) or 0.0)
        for source, target, condition in self._transitions:
            if (source == "*" or source == self._state) and target != self._state:
                if condition(entity, ctx):
                    self._switch(target)
                    break
        result = _call_state_fn(self._states[self._state]["update"], entity, ctx, self)
        if isinstance(result, str) and result != self._state:
            if result not in self._states:
                raise KeyError(f"state {self._state!r} returned {result!r}, which isn't a state"
                               f"{_did_you_mean(result, self._states)}")
            self._switch(result)
        return self._state

    def __repr__(self):
        return f"StateMachine(state={self.state!r}, states={self.states!r})"


def _status(value, where):
    if value is True or value is None:
        return SUCCESS
    if value is False:
        return FAILURE
    if value in _STATUSES:
        return value
    raise ValueError(f"{where} returned {value!r}: return SUCCESS, FAILURE, RUNNING, "
                     "True/False or nothing")


class Node:
    """A behaviour tree node. ``tick(entity, ctx)`` runs it for a frame and
    returns ``SUCCESS``, ``FAILURE`` or ``RUNNING``; ``reset()`` forgets
    anything half done. Nodes remember what they were doing, so build one
    tree per entity (in ``start``)."""

    def tick(self, entity, ctx) -> str:
        raise NotImplementedError

    def reset(self) -> None:
        """Forgets running children and timers."""

    #: The status of the last tick (None before the first).
    status: str | None = None

    def _done(self, status):
        self.status = status
        return status

    def describe(self, depth=0) -> str:
        """The tree as indented text, with each node's last status."""
        return "  " * depth + f"{self!r} -> {self.status}"


def _node(x):
    if isinstance(x, Node):
        return x
    if callable(x):
        return Action(x)
    raise TypeError(f"behaviour tree children are nodes or functions, not {x!r}")


class Action(Node):
    """Calls ``fn(entity, ctx)``, which returns ``SUCCESS``, ``FAILURE``
    or ``RUNNING`` (or True / False, or nothing for success)."""

    def __init__(self, fn: Callable, name: str | None = None):
        if not callable(fn):
            raise TypeError(f"Action needs a function, not {fn!r}")
        self.fn = fn
        self.name = name or getattr(fn, "__name__", "action")

    def tick(self, entity, ctx):
        return self._done(_status(self.fn(entity, ctx), f"action {self.name!r}"))

    def __repr__(self):
        return f"Action({self.name})"


class Condition(Action):
    """``fn(entity, ctx)`` is a yes/no question: true is ``SUCCESS``,
    false ``FAILURE``. It never runs over frames."""

    def tick(self, entity, ctx):
        return self._done(SUCCESS if self.fn(entity, ctx) else FAILURE)

    def __repr__(self):
        return f"Condition({self.name})"


class _Composite(Node):
    def __init__(self, *children):
        if len(children) == 1 and isinstance(children[0], (list, tuple)):
            children = tuple(children[0])
        self.children = [_node(c) for c in children]
        self._index = 0

    def reset(self):
        self._index = 0
        for c in self.children:
            c.reset()

    def describe(self, depth=0):
        lines = [super().describe(depth)]
        lines += [c.describe(depth + 1) for c in self.children]
        return "\n".join(lines)

    def __repr__(self):
        return type(self).__name__


class Sequence(_Composite):
    """Runs its children in order while they succeed: ``FAILURE`` as soon
    as one fails, ``SUCCESS`` when all have. A child that's ``RUNNING``
    carries on from there next frame ("do this, then that")."""

    def tick(self, entity, ctx):
        while self._index < len(self.children):
            s = self.children[self._index].tick(entity, ctx)
            if s == RUNNING:
                return self._done(RUNNING)
            if s == FAILURE:
                self.reset()
                return self._done(FAILURE)
            self._index += 1
        self.reset()
        return self._done(SUCCESS)


class Selector(_Composite):
    """Tries its children in order until one doesn't fail: ``SUCCESS``
    with the first that succeeds, ``FAILURE`` if all fail ("try this, or
    else that"). Each frame it starts again from the first child, so a
    higher-priority branch (attack) interrupts a lower one (patrol)."""

    def tick(self, entity, ctx):
        for i, child in enumerate(self.children):
            s = child.tick(entity, ctx)
            if s != FAILURE:
                # Whatever was running further down is interrupted.
                for other in self.children[i + 1:]:
                    other.reset()
                return self._done(s)
        return self._done(FAILURE)


class Parallel(_Composite):
    """Ticks every child each frame. ``need="all"``: ``SUCCESS`` once all
    have succeeded, ``FAILURE`` as soon as one fails. ``need="one"``:
    ``SUCCESS`` as soon as one succeeds, ``FAILURE`` once all have failed.
    Otherwise ``RUNNING``."""

    def __init__(self, *children, need: str = "all"):
        super().__init__(*children)
        if need not in ("all", "one"):
            raise ValueError(f"need is 'all' or 'one', not {need!r}")
        self.need = need
        self._finished = {}

    def reset(self):
        super().reset()
        self._finished = {}

    def tick(self, entity, ctx):
        for i, child in enumerate(self.children):
            if i in self._finished:
                continue
            s = child.tick(entity, ctx)
            if s != RUNNING:
                self._finished[i] = s
        results = list(self._finished.values())
        if self.need == "all":
            if FAILURE in results:
                self.reset()
                return self._done(FAILURE)
            if len(results) == len(self.children):
                self.reset()
                return self._done(SUCCESS)
        else:
            if SUCCESS in results:
                self.reset()
                return self._done(SUCCESS)
            if len(results) == len(self.children):
                self.reset()
                return self._done(FAILURE)
        return self._done(RUNNING)


class _Decorator(Node):
    def __init__(self, child):
        self.child = _node(child)

    def reset(self):
        self.child.reset()

    def describe(self, depth=0):
        return super().describe(depth) + "\n" + self.child.describe(depth + 1)

    def __repr__(self):
        return type(self).__name__


class Invert(_Decorator):
    """Swaps its child's ``SUCCESS`` and ``FAILURE`` ("not")."""

    def tick(self, entity, ctx):
        s = self.child.tick(entity, ctx)
        return self._done({SUCCESS: FAILURE, FAILURE: SUCCESS}.get(s, s))


class Succeed(_Decorator):
    """Runs its child but always succeeds once it's done (an optional
    step in a sequence)."""

    def tick(self, entity, ctx):
        s = self.child.tick(entity, ctx)
        return self._done(RUNNING if s == RUNNING else SUCCESS)


class Repeat(_Decorator):
    """Runs its child again and again: ``times`` successes in a row for
    ``SUCCESS`` (forever with None), ``FAILURE`` as soon as it fails. One
    repeat per frame at most."""

    def __init__(self, child, times: int | None = None):
        super().__init__(child)
        if times is not None and int(times) < 1:
            raise ValueError("times must be at least 1 (or None for forever)")
        self.times = None if times is None else int(times)
        self._count = 0

    def reset(self):
        super().reset()
        self._count = 0

    def tick(self, entity, ctx):
        s = self.child.tick(entity, ctx)
        if s == FAILURE:
            self.reset()
            return self._done(FAILURE)
        if s == SUCCESS:
            self._count += 1
            self.child.reset()
            if self.times is not None and self._count >= self.times:
                self._count = 0
                return self._done(SUCCESS)
        return self._done(RUNNING)


def _now(ctx):
    return float(getattr(getattr(ctx, "time", None), "elapsed", 0.0) or 0.0)


class Wait(Node):
    """``RUNNING`` for ``seconds`` of game time, then ``SUCCESS``."""

    def __init__(self, seconds: float):
        self.seconds = float(seconds)
        self._until = None

    def reset(self):
        self._until = None

    def tick(self, entity, ctx):
        now = _now(ctx)
        if self._until is None:
            self._until = now + self.seconds
        if now + 1e-9 >= self._until:
            self._until = None
            return self._done(SUCCESS)
        return self._done(RUNNING)

    def __repr__(self):
        return f"Wait({self.seconds:g}s)"


class Cooldown(_Decorator):
    """Runs its child at most once every ``seconds``: after the child
    succeeds, it fails straight away until the time is up (a shot, a
    shout)."""

    def __init__(self, seconds: float, child):
        super().__init__(child)
        self.seconds = float(seconds)
        self._ready_at = -math.inf

    def tick(self, entity, ctx):
        now = _now(ctx)
        if now < self._ready_at:
            return self._done(FAILURE)
        s = self.child.tick(entity, ctx)
        if s == SUCCESS:
            self._ready_at = now + self.seconds
        return self._done(s)

    def __repr__(self):
        return f"Cooldown({self.seconds:g}s)"


def _rotate_euler(v, euler):
    """``v`` turned by an XYZ Euler rotation (radians), as transforms use."""
    rx, ry, rz = euler
    x, y, z = v
    c, s = math.cos(rz), math.sin(rz)
    x, y = x * c - y * s, x * s + y * c
    c, s = math.cos(ry), math.sin(ry)
    x, z = x * c + z * s, -x * s + z * c
    c, s = math.cos(rx), math.sin(rx)
    y, z = y * c - z * s, y * s + z * c
    return (x, y, z)


class Noise:
    """A sound an AI can hear (``ctx.ai.noise``): ``position``, ``radius``
    (how far it carries, m), ``kind`` (your label: "shot", "footstep"),
    ``source`` (the entity's name, or None), ``time`` (``ctx.time.elapsed``
    when made) and, from ``hear``, ``distance`` from the listener."""

    __slots__ = ("position", "radius", "kind", "source", "time", "distance", "_id", "_frame",
                 "_heard")

    def __init__(self, position, radius, kind, source, time_, frame, nid):
        self.position = position
        self.radius = radius
        self.kind = kind
        self.source = source
        self.time = time_
        self.distance = 0.0
        self._id = nid
        self._frame = frame
        self._heard = set()

    def _for(self, distance):
        n = Noise(self.position, self.radius, self.kind, self.source, self.time, self._frame,
                  self._id)
        n.distance = distance
        return n

    def __repr__(self):
        return (f"Noise({self.kind!r}, position={tuple(round(v, 2) for v in self.position)}, "
                f"radius={self.radius:g}, source={self.source!r})")


class AI:
    """``ctx.ai``: helpers for enemies and other characters that think for
    themselves.

    - **Senses:** ``can_see`` (a cone of vision blocked by walls),
      ``visible``, ``noise`` and ``hear``.
    - **Brains:** ``state_machine`` and behaviour tree nodes
      (``selector``, ``sequence``, ``action``, ``condition``, ``wait``,
      ...), with ``SUCCESS``, ``FAILURE`` and ``RUNNING``.

    The same classes are ``automata.ai`` in Python games.
    """

    SUCCESS = SUCCESS
    FAILURE = FAILURE
    RUNNING = RUNNING
    StateMachine = StateMachine
    Node = Node

    def __init__(self, ctx):
        self._ctx = ctx
        self._noises = []
        self._next_noise = 1

    # -- Brains -------------------------------------------------------------

    def state_machine(self, states: dict | None = None, initial: str | None = None,
                      on_change: Callable | None = None) -> StateMachine:
        """A new :class:`StateMachine` (one per entity: keep it in
        ``entity.state``)."""
        return StateMachine(states, initial, on_change)

    def sequence(self, *children) -> Sequence:
        """Children in order while they succeed (see :class:`Sequence`)."""
        return Sequence(*children)

    def selector(self, *children) -> Selector:
        """The first child that doesn't fail (see :class:`Selector`)."""
        return Selector(*children)

    def parallel(self, *children, need: str = "all") -> Parallel:
        """Every child at once (see :class:`Parallel`)."""
        return Parallel(*children, need=need)

    def action(self, fn: Callable, name: str | None = None) -> Action:
        return Action(fn, name)

    def condition(self, fn: Callable, name: str | None = None) -> Condition:
        return Condition(fn, name)

    def wait(self, seconds: float) -> Wait:
        return Wait(seconds)

    def invert(self, child) -> Invert:
        return Invert(child)

    def succeed(self, child) -> Succeed:
        return Succeed(child)

    def repeat(self, child, times: int | None = None) -> Repeat:
        return Repeat(child, times)

    def cooldown(self, seconds: float, child) -> Cooldown:
        return Cooldown(seconds, child)

    # -- Sight --------------------------------------------------------------

    def _entity(self, x, what):
        if isinstance(x, str):
            e = self._ctx.scene.find(x)
            if e is None:
                return None
            return e
        return x

    @staticmethod
    def _position(x):
        if isinstance(x, (tuple, list)):
            return _vec(tuple(x)[:3], 3)
        return tuple(x.transform.position)

    def can_see(self, viewer, target, fov: float = 120.0, distance: float = 20.0,
                eye: float = 1.6, aim: float = 1.0, forward: Iterable[float] = (0.0, 0.0, -1.0),
                walls: bool = True) -> bool:
        """True when ``viewer`` (an entity) can see ``target`` (an entity,
        its name, or a point): within ``distance`` metres, inside a cone
        ``fov`` degrees wide around the way it faces, and (with ``walls``)
        no collider in between.

        ``eye`` is the eyes' height above the viewer's origin and ``aim``
        the height looked at on the target (both m; ignored for a point).
        ``forward`` is the way the viewer's model faces before it's
        turned: ``(0, 0, -1)`` for most models and cameras, ``(0, 0, 1)``
        for models that face +Z."""
        return self._sight(viewer, target, fov, distance, eye, aim, forward, walls) is not None

    def _sight(self, viewer, target, fov, distance, eye, aim, forward, walls):
        viewer = self._entity(viewer, "viewer")
        target = self._entity(target, "target")
        if viewer is None or target is None:
            return None
        vx, vy, vz = self._position(viewer)
        origin = (vx, vy + float(eye), vz)
        if isinstance(target, (tuple, list)):
            point = self._position(target)
            target_id = None
        else:
            if getattr(target, "id", None) == getattr(viewer, "id", None):
                return None
            tx, ty, tz = self._position(target)
            point = (tx, ty + float(aim), tz)
            target_id = target.id
        d = tuple(p - o for p, o in zip(point, origin))
        dist = math.sqrt(sum(c * c for c in d))
        if dist > float(distance):
            return None
        if dist > 1e-6 and float(fov) < 360.0:
            rot = getattr(viewer.transform, "rotation_euler", (0.0, 0.0, 0.0))
            f = _rotate_euler(_vec(tuple(forward)[:3], 3), rot)
            fl = math.sqrt(sum(c * c for c in f))
            if fl < 1e-9:
                return None
            cos = sum(a * b for a, b in zip(f, d)) / (fl * dist)
            if cos < math.cos(math.radians(min(float(fov), 360.0)) / 2.0):
                return None
        if walls and dist > 1e-6:
            direction = tuple(c / dist for c in d)
            hit = self._ctx.physics.raycast(origin, direction, dist, ignore=viewer)
            if hit is not None and hit.distance < dist - 0.25:
                if target_id is None:
                    return None
                if hit.entity is not None:
                    if hit.entity.id != target_id:
                        return None
                elif math.dist(hit.point, point) > 0.75:
                    # Something unnamed, not at the target: a wall.
                    return None
        return dist

    def visible(self, viewer, targets=None, fov: float = 120.0, distance: float = 20.0,
                eye: float = 1.6, aim: float = 1.0, forward: Iterable[float] = (0.0, 0.0, -1.0),
                walls: bool = True) -> list:
        """The entities in ``targets`` (names or entities; None: every
        entity in the scene) that ``viewer`` can see, nearest first (see
        ``can_see``)."""
        viewer = self._entity(viewer, "viewer")
        if viewer is None:
            return []
        if targets is None:
            pool = list(self._ctx.scene)
        elif isinstance(targets, str):
            pool = [targets]
        else:
            pool = list(targets)
        seen = []
        for t in pool:
            t = self._entity(t, "target")
            if t is None or getattr(t, "id", None) == getattr(viewer, "id", None):
                continue
            d = self._sight(viewer, t, fov, distance, eye, aim, forward, walls)
            if d is not None:
                seen.append((d, t))
        seen.sort(key=lambda p: p[0])
        return [t for _, t in seen]

    # -- Hearing ------------------------------------------------------------

    def _frame(self):
        return int(getattr(getattr(self._ctx, "time", None), "frame", 0) or 0)

    def noise(self, position, radius: float = 10.0, kind: str = "noise", source=None) -> Noise:
        """Makes a sound AIs can ``hear``: at ``position`` (a point or an
        entity), carrying ``radius`` metres. ``kind`` labels it, ``source``
        (an entity or name) says who made it, so they don't hear
        themselves. Every listener in range hears it once, this frame or
        the next."""
        if not isinstance(position, (tuple, list)) and source is None:
            source = position
        p = self._position(self._entity(position, "position")) if position is not None else None
        if p is None:
            raise ValueError("a noise needs a position")
        r = float(radius)
        if not math.isfinite(r) or r < 0:
            raise ValueError(f"radius must be a positive number, not {radius!r}")
        name = source if isinstance(source, str) or source is None else getattr(source, "name", None)
        n = Noise(p, r, str(kind), name, _now(self._ctx), self._frame(), self._next_noise)
        self._next_noise += 1
        self._noises.append(n)
        return n

    def _live_noises(self):
        frame = self._frame()
        self._noises = [n for n in self._noises if n._frame >= frame - 1]
        return self._noises

    @property
    def noises(self) -> list:
        """Every noise made this frame or the last."""
        return list(self._live_noises())

    def hear(self, listener, hearing: float = 1.0, kinds=None) -> list:
        """The noises ``listener`` hasn't heard yet that reach it, loudest
        (nearest for its radius) first. A noise reaches when the listener
        is within its radius times ``hearing`` (2.0: sharp ears).
        ``kinds`` keeps only those labels."""
        listener = self._entity(listener, "listener")
        if listener is None:
            return []
        here = self._position(listener)
        name = getattr(listener, "name", None)
        lid = getattr(listener, "id", id(listener))
        if isinstance(kinds, str):
            kinds = (kinds,)
        out = []
        for n in self._live_noises():
            if lid in n._heard or (name and n.source == name):
                continue
            if kinds is not None and n.kind not in kinds:
                continue
            d = math.dist(here, n.position)
            if d <= n.radius * float(hearing):
                n._heard.add(lid)
                out.append(n._for(d))
        out.sort(key=lambda n: n.distance / max(n.radius, 1e-6))
        return out

    def __repr__(self):
        return f"AI(noises={len(self._noises)})"


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
        self._strings = None

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
        font: str | None = None,
    ) -> None:
        """Draws ``text`` this frame.

        ``size`` is the line height in pixels. ``align`` says where ``x``
        sits on each line: ``"left"`` (its left edge), ``"center"`` or
        ``"right"``. Lines (split on ``\\n``) are centred vertically on
        ``y``. ``shadow`` and ``outline`` take a colour, or True for
        translucent / solid black. ``font`` is a ``.ttf`` or ``.otf`` file
        relative to the scene folder, like ``"fonts/Pixel.ttf"`` (None:
        the built-in font). Characters the font lacks are drawn with the
        language's font (``ctx.tr.font()``), then the built-in font.
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
        if font is not None:
            item["font"] = _font_path(font)
        if self._strings is not None:
            # The language's font draws what the text's own font lacks.
            fallback = self._strings.font()
            if fallback and fallback != item.get("font"):
                item["fallback"] = fallback
        for key, value, default in (("shadow", shadow, (0.0, 0.0, 0.0, 0.6)),
                                    ("outline", outline, (0.0, 0.0, 0.0, 1.0))):
            if value is True:
                value = default
            if value is not None and value is not False:
                item[key] = list(_vec(value, 4))
        self._items.append(item)


Color = tuple[float, float, float, float]


def _text_ems(line):
    """About how wide ``line`` draws, in multiples of the text size (the
    text can't be measured on every runner): full-width characters, such as
    Japanese and Chinese, take about twice a Latin letter's room."""
    return sum(1.0 if unicodedata.east_asian_width(c) in "WF" else 0.55 for c in line)


def _font_path(font):
    """A ``font=`` argument: a .ttf / .otf path, with forward slashes."""
    if not isinstance(font, str) or not font.strip():
        raise TypeError(f'font is a path like "fonts/Pixel.ttf", not {font!r}')
    path = font.strip().replace("\\", "/")
    if not path.lower().endswith((".ttf", ".otf")):
        raise ValueError(f"font must be a .ttf or .otf file, not {font!r}")
    return path


class UiTheme:
    """``ctx.ui.theme``: the widgets' colours and text size. Change any
    field, at any time; it applies from the next widget drawn. Colours
    are RGBA from 0 to 1 as a paint program shows them (sRGB), like every
    colour given to ``ctx.ui``::

        ctx.ui.theme.accent = (1.0, 0.75, 0.2, 1.0)
        ctx.ui.theme.text_size = 24
    """

    __slots__ = ("text", "text_dim", "panel", "button", "hover", "press", "accent",
                 "on_accent", "track", "border", "focus", "text_size", "font")

    def __init__(self):
        #: Labels and button text.
        self.text: Color = (0.95, 0.94, 0.92, 1.0)
        #: Disabled text and hints.
        self.text_dim: Color = (0.6, 0.6, 0.6, 1.0)
        #: Panel backgrounds.
        self.panel: Color = (0.05, 0.05, 0.06, 0.9)
        #: Buttons at rest, hovered and held.
        self.button: Color = (0.15, 0.15, 0.17, 0.94)
        self.hover: Color = (0.24, 0.24, 0.27, 0.96)
        self.press: Color = (0.09, 0.09, 0.1, 0.98)
        #: Filled parts: progress, slider, a toggle that's on.
        self.accent: Color = (0.95, 0.94, 0.92, 1.0)
        #: Text drawn on the accent.
        self.on_accent: Color = (0.03, 0.03, 0.03, 1.0)
        #: Empty progress bars, slider tracks and toggle boxes.
        self.track: Color = (0.0, 0.0, 0.0, 0.6)
        #: The thin edge around buttons and panels.
        self.border: Color = (1.0, 1.0, 1.0, 0.16)
        #: The ring around the widget keys and gamepads have focused.
        self.focus: Color = (1.0, 0.78, 0.3, 1.0)
        #: Text height in pixels when a widget isn't given a ``size``.
        self.text_size: float = 20.0
        #: A ``.ttf`` / ``.otf`` file for every widget's text, relative to
        #: the scene folder (None: the built-in font).
        self.font: str | None = None


def _dim(c, k=0.45):
    return (c[0], c[1], c[2], c[3] * k)


#: Where ``anchor=`` puts a box on the screen: (x, y) as fractions.
ANCHORS = {
    "top_left": (0.0, 0.0), "top": (0.5, 0.0), "top_right": (1.0, 0.0),
    "left": (0.0, 0.5), "center": (0.5, 0.5), "right": (1.0, 0.5),
    "bottom_left": (0.0, 1.0), "bottom": (0.5, 1.0), "bottom_right": (1.0, 1.0),
}


def _anchor(name):
    key = str(name).strip().lower().replace("-", "_").replace(" ", "_")
    key = {"centre": "center", "middle": "center", "top_center": "top",
           "bottom_center": "bottom", "center_left": "left", "center_right": "right"}.get(key, key)
    if key not in ANCHORS:
        raise ValueError(f"there's no anchor {name!r}{_did_you_mean(key, ANCHORS)} "
                         f"(anchors: {', '.join(ANCHORS)})")
    return ANCHORS[key]


def _margins(margin):
    if isinstance(margin, (tuple, list)):
        return float(margin[0]), float(margin[1])
    return float(margin), float(margin)


# Keys and gamepad buttons the UI navigates with.
_NAV = {
    "up": ("arrow_up", "pad_up"),
    "down": ("arrow_down", "pad_down"),
    "left": ("arrow_left", "pad_left"),
    "right": ("arrow_right", "pad_right"),
}
_ACTIVATE = ("enter", "space", "pad_south")
_CANCEL = ("escape", "pad_east")


class Screen:
    """One shown screen in :attr:`Ui.screens` (read-only)."""

    __slots__ = ("name", "modal", "pause", "escape")

    def __init__(self, name, modal, pause, escape):
        self.name = name
        self.modal = modal
        self.pause = pause
        self.escape = escape

    def __repr__(self):
        return f"Screen({self.name!r}, modal={self.modal}, pause={self.pause})"


class Screens:
    """``ctx.ui.screens``: which screens are shown, newest on top.

    A screen is a name: a script draws it when ``ctx.ui.screen(name)``
    says it's shown. A **modal** screen (the default) takes all the UI's
    clicks and keys: widgets under it still draw but don't react, and
    arrow keys / the gamepad move focus between its widgets. Escape (or the
    gamepad's east button) closes the top screen unless it was shown with
    ``escape=False``::

        if ctx.input.just_pressed("pause"):
            ctx.ui.screens.show("pause", pause=True)
        if ctx.ui.screen("pause"):
            ...  # draw the pause menu
    """

    def __init__(self, ui):
        self._ui = ui
        self._stack: list[Screen] = []

    def show(self, name: str, *, modal: bool | None = None, pause: bool | None = None,
             escape: bool | None = None) -> None:
        """Shows ``name`` on top of the others (moving it up if it's already
        shown). ``pause=True`` sets :attr:`paused` while it's shown, for
        game scripts to check. ``escape=False``: Escape doesn't close it.
        Options left out come from the screen's ``ui/<name>.ui.ron`` if it
        has one, else modal, not paused, closed by Escape."""
        if not isinstance(name, str) or not name:
            raise TypeError(f"a screen is a name like 'pause', not {name!r}")
        doc = self._ui._document(name)
        if modal is None:
            modal = doc.modal if doc is not None else True
        if pause is None:
            pause = doc.pause if doc is not None else False
        if escape is None:
            escape = doc.escape if doc is not None else True
        self._stack = [s for s in self._stack if s.name != name]
        self._stack.append(Screen(name, bool(modal), bool(pause), bool(escape)))
        self._ui._opened(modal)

    def hide(self, name: str) -> None:
        """Hides ``name`` (nothing happens if it isn't shown)."""
        self._stack = [s for s in self._stack if s.name != name]

    def toggle(self, name: str, **options: Any) -> bool:
        """Hides ``name`` if it's shown, otherwise shows it (``options`` as
        for :meth:`show`). Returns whether it's shown now."""
        if self.shown(name):
            self.hide(name)
            return False
        self.show(name, **options)
        return True

    def back(self) -> str | None:
        """Hides the top screen and returns its name (None if none)."""
        if not self._stack:
            return None
        return self._stack.pop().name

    def clear(self) -> None:
        """Hides every screen."""
        self._stack = []

    def shown(self, name: str) -> bool:
        return any(s.name == name for s in self._stack)

    @property
    def top(self) -> str | None:
        """The newest shown screen's name, or None."""
        return self._stack[-1].name if self._stack else None

    @property
    def names(self) -> list[str]:
        """The shown screens, bottom first."""
        return [s.name for s in self._stack]

    @property
    def paused(self) -> bool:
        """A screen shown with ``pause=True`` is up: game scripts should
        hold still (``if ctx.ui.screens.paused: return``)."""
        return any(s.pause for s in self._stack)

    @property
    def modal(self) -> str | None:
        """The top modal screen's name: the one taking input (or None)."""
        for s in reversed(self._stack):
            if s.modal:
                return s.name
        return None

    def _layer(self, name):
        """Draw order of a screen's widgets: 0 below every screen."""
        for i, s in enumerate(self._stack):
            if s.name == name:
                return i + 1
        return 0

    def __contains__(self, name):
        return self.shown(name)

    def __repr__(self):
        return f"Screens({self.names!r})"


class UiScreen:
    """What :meth:`Ui.screen` returns: truthy while the screen is shown.

    Use it in an ``if`` or a ``with`` to draw a screen yourself. If the
    screen was made in Oberon's UI editor (``ui/<name>.ui.ron``), it also
    talks to that document's widgets by their names::

        hud = ctx.ui.screen("hud")
        hud.set("health", player.health)      # a progress bar's value
        hud.set("score", f"Score {score}")    # a label's text
        if ctx.ui.screen("menu").clicked("play"):
            ctx.ui.screens.hide("menu")
    """

    __slots__ = ("_ui", "_shown", "_before", "_name")

    def __init__(self, ui, shown, before, name=None):
        self._ui = ui
        self._shown = shown
        self._before = before
        self._name = name

    def __bool__(self):
        return self._shown

    def __enter__(self):
        return self._shown

    def __exit__(self, *exc):
        self._ui._screen_ctx = self._before
        return False

    def __repr__(self):
        return f"UiScreen({self._name!r}, shown={self._shown})"

    @property
    def name(self) -> str | None:
        """The screen's name."""
        return self._name

    @property
    def document(self) -> UiDocument | None:
        """Its ``ui/<name>.ui.ron`` from Oberon's UI editor, if there is one."""
        return None if self._name is None else self._ui._document(self._name)

    def _doc(self):
        doc = self.document
        if doc is None:
            raise LookupError(f"there's no UI document ui/{self._name}.ui.ron to bind to "
                              f"(make one in Oberon: File ▸ New UI Screen…)")
        return doc

    def set(self, widget: str, value: Any) -> None:
        """Sets a widget of the document by its name: a label's or button's
        text, a toggle's on/off, a slider's or progress bar's value, or an
        image's path. It stays until set again."""
        doc = self._doc()
        node = doc.node(widget)
        kind = node["kind"]
        if kind == "toggle":
            value = bool(value)
        elif kind in ("slider", "progress"):
            try:
                value = float(value)
            except (TypeError, ValueError):
                raise TypeError(f"{widget!r} is a {kind}: set it to a number, not {value!r}") from None
            if not math.isfinite(value):
                raise ValueError(f"{widget!r}: {value} isn't a finite number")
        elif kind in ("column", "row", "stack", "space"):
            raise TypeError(f"{widget!r} is a {kind}, which has no value; use "
                            "set_visible() to show or hide it")
        else:
            value = str(value)
        doc.values[widget] = value

    def get(self, widget: str) -> Any:
        """A widget's value now: what :meth:`set` gave it, what the player
        changed it to (sliders, toggles), or what the editor gave it."""
        doc = self._doc()
        return doc.value(doc.node(widget))

    def clicked(self, widget: str) -> bool:
        """The button was clicked (or a toggle or slider was used) the
        last time the screen was drawn."""
        doc = self._doc()
        doc.node(widget)
        return widget in doc.clicked

    def changed(self, widget: str) -> bool:
        """The player changed this slider or toggle the last time the
        screen was drawn; :meth:`get` has the new value."""
        doc = self._doc()
        doc.node(widget)
        return widget in doc.changed

    def set_visible(self, widget: str, visible: bool = True) -> None:
        """Shows or hides one widget (or a whole column, row or stack)."""
        doc = self._doc()
        doc.node(widget)
        doc.hidden[widget] = not visible

    def set_enabled(self, widget: str, enabled: bool = True) -> None:
        """Enables or disables (dims) a widget."""
        doc = self._doc()
        doc.node(widget)
        doc.disabled[widget] = not enabled

    @property
    def widgets(self) -> list[str]:
        """The names of the document's widgets."""
        return sorted(self._doc().by_name)

    @property
    def visible(self) -> bool:
        """Whether the screen shows. For a HUD document, set it False to
        hide it (True to bring it back); for others it's the same as
        ``ctx.ui.screens.shown(name)``."""
        doc = self.document
        if doc is not None and doc.hud:
            return doc.visible
        return self._name is not None and self._ui.screens.shown(self._name)

    @visible.setter
    def visible(self, on: bool) -> None:
        doc = self.document
        if doc is not None and doc.hud:
            doc.visible = bool(on)
        elif self._name is not None:
            if on:
                self._ui.screens.show(self._name)
            else:
                self._ui.screens.hide(self._name)


class RonError(ValueError):
    """A ``.ron`` file that couldn't be read: where, and why."""


def _ron_parse(text, where="<ron>"):
    """Reads RON (Rusty Object Notation) into Python values: structs become
    dicts, tuples and lists become lists, ``Some(x)`` is ``x``, ``None`` is
    None, and bare names (enum variants) become strings."""
    return _Ron(text, where).document()


class _Ron:
    _NUMBER = None

    def __init__(self, text, where):
        self.s = text
        self.i = 0
        self.where = where

    def fail(self, msg):
        line = self.s.count("\n", 0, self.i) + 1
        col = self.i - (self.s.rfind("\n", 0, self.i) + 1) + 1
        raise RonError(f"{self.where}:{line}:{col}: {msg}")

    def skip(self):
        s, n = self.s, len(self.s)
        while self.i < n:
            c = s[self.i]
            if c in " \t\r\n":
                self.i += 1
            elif s.startswith("//", self.i):
                end = s.find("\n", self.i)
                self.i = n if end < 0 else end + 1
            elif s.startswith("/*", self.i):
                end = s.find("*/", self.i + 2)
                if end < 0:
                    self.fail("a /* comment never ends")
                self.i = end + 2
            else:
                break

    def peek(self):
        self.skip()
        return self.s[self.i] if self.i < len(self.s) else ""

    def expect(self, c):
        if self.peek() != c:
            got = self.s[self.i] if self.i < len(self.s) else "the end"
            self.fail(f"expected {c!r}, found {got!r}")
        self.i += 1

    def document(self):
        self.skip()
        while self.s.startswith("#!", self.i):  # #![enable(...)] headers
            end = self.s.find("]", self.i)
            if end < 0:
                self.fail("an attribute never ends")
            self.i = end + 1
            self.skip()
        value = self.value()
        if self.peek():
            self.fail("unexpected text after the value")
        return value

    def ident(self):
        self.skip()
        start = self.i
        s = self.s
        while self.i < len(s) and (s[self.i].isalnum() or s[self.i] == "_"):
            self.i += 1
        return s[start:self.i]

    def value(self):
        c = self.peek()
        if c == "(":
            return self.group()
        if c == "[":
            self.i += 1
            out = []
            while self.peek() != "]":
                out.append(self.value())
                if self.peek() == ",":
                    self.i += 1
                elif self.peek() != "]":
                    self.fail("expected ',' or ']' in a list")
            self.i += 1
            return out
        if c == "{":
            self.i += 1
            out = {}
            while self.peek() != "}":
                key = self.value()
                self.expect(":")
                out[key if isinstance(key, (str, int, float, bool)) else str(key)] = self.value()
                if self.peek() == ",":
                    self.i += 1
                elif self.peek() != "}":
                    self.fail("expected ',' or '}' in a map")
            self.i += 1
            return out
        if c == '"':
            return self.string()
        if c == "r" and self.s.startswith(('r"', 'r#'), self.i):
            return self.raw_string()
        if c == "'":
            return self.char()
        if c and (c.isdigit() or c in "+-."):
            return self.number()
        if c and (c.isalpha() or c == "_"):
            name = self.ident()
            if name == "true":
                return True
            if name == "false":
                return False
            if name in ("None",):
                return None
            if name in ("inf", "NaN"):
                return float(name.replace("NaN", "nan"))
            if self.peek() == "(":
                inner = self.group()
                if name == "Some":
                    return inner[0] if isinstance(inner, list) and len(inner) == 1 else inner
                if isinstance(inner, list) and len(inner) == 1:
                    return inner[0]
                return inner
            return name
        if not c:
            self.fail("the file ends where a value should be")
        self.fail(f"unexpected {c!r}")
        return None

    def group(self):
        """``( ... )``: a struct (``field: value``) or a tuple."""
        self.expect("(")
        save = self.i
        name = self.ident()
        is_struct = bool(name) and self.peek() == ":" and not self.s.startswith("::", self.i)
        self.i = save
        if is_struct:
            out = {}
            while self.peek() != ")":
                key = self.ident()
                if not key:
                    self.fail("expected a field name")
                self.expect(":")
                out[key] = self.value()
                if self.peek() == ",":
                    self.i += 1
                elif self.peek() != ")":
                    self.fail("expected ',' or ')' after a field")
            self.i += 1
            return out
        items = []
        while self.peek() != ")":
            items.append(self.value())
            if self.peek() == ",":
                self.i += 1
            elif self.peek() != ")":
                self.fail("expected ',' or ')' in a tuple")
        self.i += 1
        return items

    _ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}

    def string(self):
        self.expect('"')
        s, out = self.s, []
        while True:
            if self.i >= len(s):
                self.fail("a string never ends")
            c = s[self.i]
            self.i += 1
            if c == '"':
                return "".join(out)
            if c != "\\":
                out.append(c)
                continue
            e = s[self.i:self.i + 1]
            self.i += 1
            if e in self._ESCAPES:
                out.append(self._ESCAPES[e])
            elif e == "u" and s.startswith("{", self.i):
                end = s.find("}", self.i)
                out.append(chr(int(s[self.i + 1:end], 16)))
                self.i = end + 1
            elif e == "x":
                out.append(chr(int(s[self.i:self.i + 2], 16)))
                self.i += 2
            elif e == "\n":
                while self.i < len(s) and s[self.i] in " \t\r\n":
                    self.i += 1
            else:
                self.fail(f"unknown escape \\{e}")

    def raw_string(self):
        self.i += 1
        hashes = 0
        while self.s[self.i] == "#":
            hashes += 1
            self.i += 1
        self.expect('"')
        end = self.s.find('"' + "#" * hashes, self.i)
        if end < 0:
            self.fail("a raw string never ends")
        out = self.s[self.i:end]
        self.i = end + 1 + hashes
        return out

    def char(self):
        self.expect("'")
        if self.s[self.i] == "\\":
            self.i += 1
            c = self._ESCAPES.get(self.s[self.i], self.s[self.i])
        else:
            c = self.s[self.i]
        self.i += 1
        self.expect("'")
        return c

    def number(self):
        s = self.s
        start = self.i
        if s[self.i] in "+-":
            self.i += 1
        if s.startswith(("inf", "NaN"), self.i):
            word = s[self.i:self.i + 3]
            self.i += 3
            v = float("inf") if word == "inf" else float("nan")
            return -v if s[start] == "-" else v
        if s.startswith(("0x", "0b", "0o"), self.i):
            base = {"x": 16, "b": 2, "o": 8}[s[self.i + 1]]
            self.i += 2
            begin = self.i
            while self.i < len(s) and (s[self.i].isalnum() or s[self.i] == "_"):
                self.i += 1
            digits = s[begin:self.i].replace("_", "")
            v = int(digits, base)
            return -v if s[start] == "-" else v
        while self.i < len(s) and (s[self.i].isdigit() or s[self.i] in "._eE" or (
                s[self.i] in "+-" and s[self.i - 1] in "eE")):
            self.i += 1
        text = s[start:self.i].replace("_", "")
        # Type suffixes such as 1u8 or 2.0f32.
        while self.i < len(s) and s[self.i].isalnum():
            self.i += 1
        try:
            if any(ch in text for ch in ".eE") and not text.lstrip("+-").startswith("0x"):
                return float(text)
            return int(text)
        except ValueError:
            self.fail(f"{text!r} isn't a number")
            return 0


#: Node kinds a UI document holds, and the ones that hold other nodes.
UI_NODE_KINDS = ("column", "row", "stack", "label", "button", "toggle", "slider", "progress",
                 "image", "space")
_UI_LAYOUTS = ("column", "row", "stack")
#: What a node is when its file leaves a field out (Oberon writes only
#: what differs).
_UI_NODE_DEFAULTS: dict[str, Any] = {
    "name": "", "text": "", "anchor": "", "x": 0.0, "y": 0.0, "width": None, "height": None,
    "gap": 10.0, "padding": 0.0, "columns": 0, "panel": False, "title": "", "size": None,
    "color": None, "align": "left", "font": "", "value": 0.0, "min": 0.0, "max": 1.0,
    "step": 0.0, "enabled": True, "visible": True, "children": (),
}
_UI_THEME_FIELDS = ("text", "text_dim", "panel", "button", "hover", "press", "accent",
                    "on_accent", "track", "border", "focus", "text_size", "font")


def _ui_node(raw, where):
    if not isinstance(raw, dict):
        raise RonError(f"{where}: a node is a (kind: ..., ...) struct, not {raw!r}")
    node: dict[str, Any] = dict(_UI_NODE_DEFAULTS)
    node.update(raw)
    kind = str(node.get("kind", "")).lower()
    if kind not in UI_NODE_KINDS:
        raise RonError(f"{where}: there's no node kind {raw.get('kind')!r}"
                       f"{_did_you_mean(kind, UI_NODE_KINDS)}")
    node["kind"] = kind
    node["children"] = [_ui_node(c, f"{where} > {kind}") for c in node["children"] or ()]
    return node


def _ui_format(text, value, top):
    """A progress bar's text: ``{value}``, ``{max}`` and ``{percent}``
    filled in (the same as Oberon's preview)."""
    if "{" not in text:
        return text
    def num(v):
        return str(int(v)) if float(v).is_integer() else f"{v:.1f}"
    k = 0.0 if top <= 0 else max(0.0, min(1.0, value / top))
    return (text.replace("{value}", num(value)).replace("{max}", num(top))
            .replace("{percent}", str(round(k * 100))))


class UiDocument:
    """A UI screen made in Oberon's UI editor: ``ui/<name>.ui.ron``.

    Scripts don't make these; :meth:`Ui.screen` finds them by name and
    draws them. A HUD document (``hud: true``) shows from the start, under
    every screen; any other is drawn while its screen is shown."""

    def __init__(self, name, path, log):
        self.name = name
        self.path = path
        self._log = log
        self.hud = False
        self.modal, self.pause, self.escape = True, False, True
        self.design_size = (1280.0, 720.0)
        self.theme = {}
        self.nodes = []
        self.by_name = {}
        self.mtime = None
        self.error = None
        #: Values scripts set or players changed, by widget name.
        self.values = {}
        self.hidden = {}
        self.disabled = {}
        #: Set by the last draw, read by scripts the frame after.
        self.clicked = set()
        self.changed = set()
        #: A HUD document's own switch (``ctx.ui.screen("hud").visible``).
        self.visible = True
        self.load()

    def load(self):
        try:
            self.mtime = os.path.getmtime(self.path)
            with open(self.path, encoding="utf-8") as f:
                raw = _ron_parse(f.read(), os.path.basename(self.path))
            if not isinstance(raw, dict):
                raise RonError(f"{os.path.basename(self.path)}: expected a (...) document")
            nodes = [_ui_node(n, os.path.basename(self.path)) for n in raw.get("nodes") or ()]
        except (OSError, RonError, TypeError, ValueError) as exc:
            if str(exc) != self.error:
                self._log("error", f"UI document {self.path}: {exc}")
            self.error = str(exc)
            return False
        self.error = None
        self.nodes = nodes
        self.hud = bool(raw.get("hud", False))
        self.modal = bool(raw.get("modal", True))
        self.pause = bool(raw.get("pause", False))
        self.escape = bool(raw.get("escape", True))
        size = raw.get("design_size") or (1280.0, 720.0)
        self.design_size = (float(size[0]), float(size[1]))
        theme = raw.get("theme") or {}
        self.theme = {k: v for k, v in theme.items() if k in _UI_THEME_FIELDS and v is not None}
        self.by_name = {}
        stack = list(nodes)
        while stack:
            node = stack.pop(0)
            if node["name"] and node["name"] not in self.by_name:
                self.by_name[node["name"]] = node
            stack.extend(node["children"])
        return True

    def stale(self):
        try:
            return os.path.getmtime(self.path) != self.mtime
        except OSError:
            return False

    def node(self, name):
        node = self.by_name.get(name)
        if node is None:
            names = sorted(self.by_name)
            known = f" (it has {', '.join(names)})" if names else " (none of its widgets has a name)"
            raise KeyError(f"ui/{self.name}.ui.ron has no widget named {name!r}"
                           f"{_did_you_mean(name, names)}{known}")
        return node

    def value(self, node):
        name = node["name"]
        if name and name in self.values:
            return self.values[name]
        kind = node["kind"]
        if kind == "toggle":
            return bool(node["value"])
        if kind in ("slider", "progress"):
            return float(node["value"])
        return node["text"]

    def __repr__(self):
        return f"UiDocument({self.name!r}, hud={self.hud})"


class _Layout:
    """A column, row or stack being filled (see :meth:`Ui.column`)."""

    def __init__(self, ui, kind, key, x, y, w, h, gap, pad, columns, anchor, margin, panel,
                 title):
        self.ui, self.kind, self.key = ui, kind, key
        self.gap, self.pad, self.columns = gap, pad, columns
        self.anchor, self.margin, self.panel, self.title = anchor, margin, panel, title
        self.w, self.h = w, h  # outer size, None: fit the contents
        self.x, self.y = x, y  # outer top-left
        self.parent: _Layout | None = None
        self.top = 0.0  # title space
        if title:
            self.top = ui._size(None) + 12.0
        self.used_w = 0.0
        self.used_h = 0.0
        self.count = 0
        self.items0 = 0
        self.rects0 = 0
        self.focus0 = 0

    @property
    def inner_w(self):
        return None if self.w is None else max(self.w - 2 * self.pad, 0.0)

    @property
    def inner_h(self):
        return None if self.h is None else max(self.h - 2 * self.pad - self.top, 0.0)

    def peek(self, w, h):
        """Where the next child would go, and the size this layout would
        give it (None: its own), without taking the slot."""
        gap = self.gap if self.count else 0.0
        x0, y0 = self.x + self.pad, self.y + self.pad + self.top
        if self.kind == "column":
            return x0, y0 + self.used_h + gap, (w if w is not None else self.inner_w), h
        if self.kind == "row":
            if w is None and self.columns and self.inner_w is not None:
                w = (self.inner_w - self.gap * (self.columns - 1)) / self.columns
            return x0 + self.used_w + gap, y0, w, (h if h is not None else self.inner_h)
        return (x0, y0, w if w is not None else self.inner_w,
                h if h is not None else self.inner_h)

    def next(self, w, h, dw, dh):
        """The next child's (x, y, width, height)."""
        gap = self.gap if self.count else 0.0
        self.count += 1
        x0, y0 = self.x + self.pad, self.y + self.pad + self.top
        if self.kind == "column":
            cw = w if w is not None else (self.inner_w if self.inner_w is not None else dw)
            ch = h if h is not None else dh
            x, y = x0, y0 + self.used_h + gap
            self.used_h += gap + ch
            self.used_w = max(self.used_w, cw)
        elif self.kind == "row":
            if w is not None:
                cw = w
            elif self.columns and self.inner_w is not None:
                cw = (self.inner_w - self.gap * (self.columns - 1)) / self.columns
            else:
                cw = dw
            ch = h if h is not None else (self.inner_h if self.inner_h is not None else dh)
            x, y = x0 + self.used_w + gap, y0
            self.used_w += gap + cw
            self.used_h = max(self.used_h, ch)
        else:  # stack: everything at the same spot
            cw = w if w is not None else (self.inner_w if self.inner_w is not None else dw)
            ch = h if h is not None else (self.inner_h if self.inner_h is not None else dh)
            x, y = x0, y0
            self.used_w = max(self.used_w, cw)
            self.used_h = max(self.used_h, ch)
        return float(x), float(y), float(cw), float(ch)

    def size(self):
        w = self.w if self.w is not None else self.used_w + 2 * self.pad
        h = self.h if self.h is not None else self.used_h + 2 * self.pad + self.top
        return w, h


class Ui:
    """``ctx.ui``: buttons, labels, toggles, sliders, progress bars, panels
    and images over the game, drawn by calling them every frame
    (immediate mode)::

        def update(entity, ctx):
            ctx.ui.panel(20, 20, 220, 170)
            if ctx.ui.button("Restart", 40, 40, 180, 44):
                restart()
            entity.state["music"] = ctx.ui.toggle("Music", entity.state["music"], 40, 100)
            entity.state["vol"] = ctx.ui.slider(entity.state["vol"], 40, 140, 180)

    Or let a layout place them, against any edge of any screen::

        ctx.ui.design_size = (1280, 720)
        with ctx.ui.column(width=300, anchor="center", panel=True, padding=24):
            if ctx.ui.button("Play"):
                start_game()
            ctx.ui.button("Quit")

    Positions are in UI units: screen pixels, unless :attr:`design_size`
    is set. ``x, y`` is a widget's **top-left** corner (``ctx.hud`` shapes
    are placed by their centre). A press that starts on a widget or panel
    belongs to the UI: ``ctx.input`` doesn't see it, so clicking a button
    doesn't also fire a weapon. The UI decides that from where things were
    drawn the frame before, so a widget takes clicks from the second frame
    it's shown. On a touch screen the first finger down is the left button.

    A widget is known from frame to frame by its text (and which entity's
    script draws it). Give ``id=`` when the text changes while it's held,
    or when two widgets share a text.
    """

    def __init__(self, host, hud, input):
        self._host = host
        self._hud = hud
        self._input = input
        input._ui = self
        self._settings: Settings | None = None
        # ctx.tr: document text that's a string-table key is translated.
        self._strings: Strings | None = None
        #: Colours and text size (see :class:`UiTheme`).
        self.theme = UiTheme()
        #: The screen size the UI is laid out for, like ``(1280, 720)``:
        #: UI units are scaled so that size fits the real screen (the other
        #: side gets more room). None: UI units are screen pixels.
        self.design_size: tuple[float, float] | None = None
        #: Arrow keys and the gamepad move focus between widgets even with
        #: no modal screen shown (they always do while one is).
        self.keyboard_navigation = False
        #: Shown screens (see :class:`Screens`).
        self.screens = Screens(self)
        self._rects = []  # this frame's widgets and panels, in pixels: (x, y, w, h)
        self._prev = []  # last frame's: what the mouse is over
        self._active = None  # the widget being pressed or dragged
        self._live = False  # it was drawn this frame
        self._seen = {}
        self._pos = None
        self._over = False
        self._captured = set()
        self._down = False
        self._pressed = False
        self._released = False
        self._layouts: list[_Layout] = []
        self._sizes = {}  # anchored layouts' sizes last frame, by id
        # UI documents from Oberon's UI editor (ui/<name>.ui.ron), by name;
        # None: looked for and not there.
        self._documents: dict[str, UiDocument | None] = {}
        self._docs_scanned = False
        self._docs_checked = 0.0
        self._screen_ctx = None  # (owner id, screen name) widgets belong to
        self._layered = []  # (item, layer) for HUD items drawn in screens
        # Focus: keys and gamepads.
        self._focus = None
        self._focusables = []  # this frame's: (id, rect)
        self._prev_focusables = []
        self._focus_drawn = False
        self._autofocus = False
        self._key_mode = False  # the last input was keys / a gamepad: show the ring
        self._taken = set()  # keys the UI used, hidden from ctx.input until released
        self._activate = False
        self._slide = 0
        self._repeat = None  # (direction, seconds to the next repeat)
        self._last_pos = None

    # -- frame bookkeeping (the host calls these) --

    def _begin(self):
        src = self._input._src
        fn = getattr(src, "mouse_position", None)
        self._pos = fn() if fn is not None else None
        self._down = bool(src.button_down("left"))
        self._pressed = bool(src.button_pressed("left"))
        self._released = bool(src.button_released("left"))
        pos = self._pos
        self._over = self._active is not None or (
            pos is not None and any(_inside(pos, r) for r in self._prev))
        for b in ("left", "right", "middle"):
            if src.button_pressed(b):
                # Each press is decided where it starts.
                if self._over:
                    self._captured.add(b)
                else:
                    self._captured.discard(b)
            elif b in self._captured and not src.button_down(b) and not src.button_released(b):
                self._captured.discard(b)
        self._layouts = []
        self._screen_ctx = None
        self._keys(src)

    def _keys(self, src):
        """Keyboard / gamepad navigation, decided before scripts run."""
        # A used key stays the UI's until it's released; each new press is
        # decided again below.
        self._taken = {k for k in self._taken
                       if not src.key_pressed(k) and (src.key_down(k) or src.key_released(k))}
        self._activate = False
        self._slide = 0
        if self._pressed or (self._pos is not None and self._last_pos is not None
                             and self._pos != self._last_pos):
            self._key_mode = False
            if self._pressed:
                self._focus = None
        self._last_pos = self._pos

        def hit(names):
            return [k for k in names if src.key_pressed(k)]

        # Escape / the gamepad's east button closes the top screen.
        top = self.screens._stack[-1] if self.screens._stack else None
        cancel = hit(_CANCEL)
        if cancel and top is not None and top.escape:
            self.screens.back()
            self._taken.update(cancel)
            self._focus = None
            self._autofocus = self._key_mode or cancel[0].startswith("pad_")
            return
        if not (self.keyboard_navigation or self.screens.modal is not None):
            self._repeat = None
            return
        items = self._prev_focusables
        # Directions, repeating while held.
        fired, keys = None, []
        dt = float(self._host.ctx.time.delta) if self._host.ctx is not None else 0.0
        for d, names in _NAV.items():
            pressed = hit(names)
            if pressed:
                fired, keys = d, pressed
                self._repeat = (d, 0.4)
                break
        if fired is None and self._repeat is not None:
            d, left = self._repeat
            held = [k for k in _NAV[d] if src.key_down(k)]
            if not held:
                self._repeat = None
            else:
                left -= dt
                if left <= 0:
                    fired, keys, left = d, held, 0.08
                self._repeat = (d, left)
                self._taken.update(held)
        tab = src.key_pressed("tab")
        if tab and items:
            back = src.key_down("shift_left") or src.key_down("shift_right")
            self._taken.add("tab")
            self._key_mode = True
            ids = [i for i, _ in items]
            if self._focus in ids:
                k = ids.index(self._focus) + (-1 if back else 1)
                self._focus = ids[k % len(ids)]
            else:
                self._focus = ids[-1 if back else 0]
        if fired is not None and items:
            self._taken.update(keys)
            woke = not self._key_mode and self._focus is not None
            self._key_mode = True
            ids = [i for i, _ in items]
            if self._focus not in ids:
                self._focus = ids[-1] if fired == "up" else ids[0]
            elif woke:
                pass  # the first key press after the mouse just shows the ring
            elif self._focus_kind() == "slider" and fired in ("left", "right"):
                self._slide = -1 if fired == "left" else 1
            else:
                self._focus = self._step(fired, items)
        act = hit(_ACTIVATE)
        if act and self._focus is not None and self._focus in [i for i, _ in items]:
            self._activate = True
            self._key_mode = True
            self._taken.update(act)

    def _focus_kind(self):
        return self._focus[1] if self._focus is not None else None

    def _step(self, d, items):
        """The focusable nearest the focused one in direction ``d``."""
        rects = dict(items)
        x, y, w, h = rects[self._focus]
        cx, cy = x + w / 2, y + h / 2
        best, best_score = None, None
        for wid, (ox, oy, ow, oh) in items:
            if wid == self._focus:
                continue
            dx, dy = ox + ow / 2 - cx, oy + oh / 2 - cy
            main, cross = {"down": (dy, dx), "up": (-dy, dx),
                           "right": (dx, dy), "left": (-dx, dy)}[d]
            if main <= 1.0:
                continue
            score = main + 2.0 * abs(cross)
            if best_score is None or score < best_score:
                best, best_score = wid, score
        if best is None and d in ("up", "down"):
            # Wrap around a menu: the far end, nearest in line.
            def key(item):
                ox, oy, ow, oh = item[1]
                return ((oy if d == "down" else -oy), abs(ox + ow / 2 - cx))
            others = [it for it in items if it[0] != self._focus]
            if others:
                best = min(others, key=key)[0]
        return self._focus if best is None else best

    def _opened(self, modal):
        """A screen was shown: keys and gamepads focus its first widget."""
        if modal:
            self._focus = None
            self._autofocus = self._key_mode

    def _layer_items(self, items):
        """Puts HUD items drawn inside screens above the rest, in screen
        order (the host calls this before sending the frame's HUD)."""
        if self._layered:
            layer = {id(item): n for item, n in self._layered}
            items.sort(key=lambda item: layer.get(id(item), 0))
            self._layered = []

    def _end(self):
        self._prev, self._rects = self._rects, []
        self._prev_focusables, self._focusables = self._focusables, []
        self._seen.clear()
        if not self._live:
            self._active = None
        self._live = False
        if not self._focus_drawn:
            # The focused widget is gone (its screen closed): the next
            # direction starts over.
            self._focus = None
        self._focus_drawn = False
        self._layouts = []
        self._layered = []

    def _id(self, kind, label, given):
        running = self._host._running
        owner = running[1].id if running else None
        base = (owner, kind, label if given is None else ("id", given))
        n = self._seen.get(base, 0)
        self._seen[base] = n + 1
        return base + (n,)

    # -- scale, screens, layers --

    @property
    def scale(self) -> float:
        """Screen pixels per UI unit: from :attr:`design_size` and the
        player's ``ctx.settings.ui_scale``."""
        s = 1.0
        if self.design_size is not None:
            dw, dh = float(self.design_size[0]), float(self.design_size[1])
            if dw > 0 and dh > 0:
                w, h = self._hud.size
                s = min(w / dw, h / dh)
        if self._settings is not None:
            s *= self._settings.ui_scale
        return max(s, 0.05)

    @property
    def size(self) -> tuple[float, float]:
        """The screen in UI units: ``design_size`` stretched to the
        screen's shape (or the screen in pixels)."""
        s = self.scale
        w, h = self._hud.size
        return (w / s, h / s)

    @property
    def mouse_position(self) -> tuple[float, float] | None:
        """The cursor in UI units, or None (see ``ctx.input.mouse_position``)."""
        if self._pos is None:
            return None
        s = self.scale
        return (self._pos[0] / s, self._pos[1] / s)

    def place(self, anchor: str, width: float, height: float,
              margin: float | tuple[float, float] = 0.0) -> tuple[float, float]:
        """The top-left of a ``width × height`` box at ``anchor`` on the
        screen (``"top_left"``, ``"top"``, ``"top_right"``, ``"left"``,
        ``"center"``, ``"right"``, ``"bottom_left"``, ``"bottom"``,
        ``"bottom_right"``), ``margin`` units in from the edges::

            if ctx.ui.button("Menu", *ctx.ui.place("top_right", 120, 44, margin=20)):
                ...
        """
        fx, fy = _anchor(anchor)
        mx, my = _margins(margin)
        sw, sh = self.size
        w, h = float(width), float(height)
        x = mx + (sw - 2 * mx - w) * fx
        y = my + (sh - 2 * my - h) * fy
        return (x, y)

    def screen(self, name: str) -> UiScreen:
        """Whether screen ``name`` is shown (see :attr:`screens`). Widgets
        this script draws after it belong to that screen: they draw above
        the screens under it, and take input only when no modal screen is
        above it. Use it as ``if ctx.ui.screen("pause"):`` or as
        ``with ctx.ui.screen("hud"):``."""
        before = self._screen_ctx
        running = self._host._running
        owner = running[1].id if running else None
        self._screen_ctx = (owner, name)
        shown = self.screens.shown(name)
        if not shown and name is not None:
            doc = self._document(name)
            shown = doc is not None and doc.hud and doc.visible
        return UiScreen(self, shown, before, name)

    # -- UI documents (Oberon's UI editor) --

    def _document(self, name):
        """The ``ui/<name>.ui.ron`` document, loaded once (None: there's
        none)."""
        if name in self._documents:
            return self._documents[name]
        doc = None
        path = self._document_path(name)
        if path is not None and os.path.isfile(path):
            doc = UiDocument(name, path, self._log)
        self._documents[name] = doc
        return doc

    def _document_path(self, name):
        rel = str(name).replace("\\", "/").strip("/")
        if not rel or ".." in rel.split("/"):
            return None
        return os.path.join(self._host.backend.base_dir(), "ui", *rel.split("/")) + ".ui.ron"

    def _log(self, level, message):
        self._host.backend.log(level, message)

    def _scan_documents(self):
        """Finds every document under ``ui/`` (HUD ones show by themselves)."""
        root = os.path.join(self._host.backend.base_dir(), "ui")
        if not os.path.isdir(root):
            return
        for folder, dirs, files in os.walk(root):
            dirs.sort()
            for file in sorted(files):
                if not file.lower().endswith(".ui.ron"):
                    continue
                rel = os.path.relpath(os.path.join(folder, file), root)
                name = rel[: -len(".ui.ron")].replace(os.sep, "/")
                if self._documents.get(name) is None:
                    self._documents.pop(name, None)
                    self._document(name)

    def _check_documents(self):
        """Reloads documents whose files changed (Oberon's hot reload)."""
        self._docs_checked = time.monotonic()
        for name, doc in list(self._documents.items()):
            if doc is None:
                del self._documents[name]
            elif doc.stale():
                doc.load()
        self._scan_documents()

    def _draw_documents(self):
        """Draws the HUD documents and those of the shown screens (the host
        calls this after every script's ``update``)."""
        if not self._docs_scanned:
            self._docs_scanned = True
            self._scan_documents()
        elif self._host.hot_reload and time.monotonic() - self._docs_checked > 0.5:
            self._check_documents()
        docs = [d for d in self._documents.values() if d is not None]
        if not docs:
            return
        for doc in docs:
            doc.clicked = set()
            doc.changed = set()
        shown = self.screens.names
        before = self._screen_ctx
        try:
            for doc in docs:
                if doc.hud and doc.visible and doc.name not in shown:
                    self._draw_document(doc, None)
            for name in shown:
                doc = self._documents.get(name)
                if doc is not None:
                    self._draw_document(doc, name)
        finally:
            self._screen_ctx = before

    def _draw_document(self, doc, screen):
        self._screen_ctx = None if screen is None else (None, screen)
        design, layouts = self.design_size, self._layouts
        saved = {}
        self.design_size = doc.design_size
        self._layouts = []
        try:
            for key, value in doc.theme.items():
                saved[key] = getattr(self.theme, key)
                if isinstance(value, list):
                    value = tuple(value)
                setattr(self.theme, key, value if value != "" else None)
            for i, node in enumerate(doc.nodes):
                self._draw_node(doc, node, (i,), True)
            doc.draw_error = None
        except (TypeError, ValueError, KeyError) as exc:
            if str(exc) != getattr(doc, "draw_error", None):
                self._log("error", f"UI document {doc.path}: {exc}")
            doc.draw_error = str(exc)
        finally:
            for key, value in saved.items():
                setattr(self.theme, key, value)
            self.design_size = design
            self._layouts = layouts

    def _draw_node(self, doc, node, path, top):
        name = node["name"]
        key = name or path
        hidden = doc.hidden.get(name) if name in doc.hidden else not node["visible"]
        if hidden:
            return
        kind = node["kind"]
        wid = (doc.name, key)
        w, h = node["width"], node["height"]
        anchor = (node["anchor"] or None) if top else None
        margin = (float(node["x"]), float(node["y"]))
        x = y = None
        if kind in _UI_LAYOUTS:
            if top and anchor is None:
                x, y = margin
            panel = node["panel"]
            if panel and node["color"]:
                panel = tuple(node["color"])
            options = {"padding": float(node["padding"]), "anchor": anchor,
                       "margin": margin if anchor else 0.0, "panel": panel, "id": wid}
            if kind != "stack":
                options["gap"] = float(node["gap"])
                options["title"] = self._doc_text(node["title"]) or None
            if kind == "row":
                options["columns"] = int(node["columns"]) or None
            with getattr(self, kind)(x, y, w, h, **options):
                for i, child in enumerate(node["children"]):
                    self._draw_node(doc, child, path + (i,), False)
            return
        size = node["size"]
        if top:
            bw, bh = self._node_box(doc, node)
            if anchor is not None:
                x, y = self.place(anchor, bw, bh, margin)
            else:
                x, y = margin
            if kind == "label":
                a = str(node["align"]).lower()
                x += bw / 2 if a in ("center", "centre") else bw if a == "right" else 0.0
        enabled = bool(node["enabled"]) and not doc.disabled.get(name, False)
        value = doc.values[key] if key in doc.values else doc.value(node)
        if kind in ("label", "button"):
            value = self._doc_text(str(value))
        if kind == "label":
            self.label(str(value), x, y, size=size, color=node["color"], align=node["align"],
                       font=node["font"] or None)
        elif kind == "button":
            if self.button(str(value), x, y, w, h, id=wid, enabled=enabled, size=size) and name:
                doc.clicked.add(name)
        elif kind == "toggle":
            new = self.toggle(self._doc_text(node["text"]), bool(value), x, y, id=wid,
                              enabled=enabled, size=size)
            if new != bool(value):
                doc.values[key] = new
                if name:
                    doc.clicked.add(name)
                    doc.changed.add(name)
        elif kind == "slider":
            step = float(node["step"]) or None
            new = self.slider(float(value), x, y, w, h, min=float(node["min"]),
                              max=float(node["max"]), step=step, id=wid, enabled=enabled)
            if abs(new - float(value)) > 1e-9:
                doc.values[key] = new
                if name:
                    doc.changed.add(name)
        elif kind == "progress":
            top_value = float(node["max"])
            text = _ui_format(self._doc_text(node["text"]), float(value), top_value) or None
            self.progress(float(value), x, y, w, h, max=top_value, color=node["color"],
                          text=text)
        elif kind == "image":
            if value:
                self.image(str(value), x, y, w, h,
                           color=tuple(node["color"]) if node["color"] else (1.0, 1.0, 1.0, 1.0))
            elif self._layouts:
                self.space(w if w is not None else 64.0, h if h is not None else 64.0)
        elif kind == "space":
            if self._layouts:
                self.space(w, h)

    def _node_box(self, doc, node):
        """The size of a widget placed on its own (anchored or at x, y)."""
        kind, w, h = node["kind"], node["width"], node["height"]
        size = self._size(node["size"])
        if kind == "label":
            text = self._doc_text(str(doc.value(node)))
            lines = text.split("\n")
            return max(_text_ems(line) for line in lines) * size, size * len(lines)
        if kind == "toggle":
            box = size * 1.2
            return box + size * 0.5 + _text_ems(self._doc_text(node["text"])) * size, box
        dw, dh = {"button": (160.0, 44.0), "slider": (200.0, 24.0), "progress": (200.0, 18.0),
                  "image": (64.0, 64.0)}.get(kind, (0.0, 0.0))
        return (float(dw if w is None else w), float(dh if h is None else h))

    def _doc_text(self, text):
        """A document's text in the player's language when it's a key of
        the string tables (``menu.play``), else as written."""
        strings = self._strings
        if text and strings is not None and strings.has(text):
            return strings(text)
        return text

    def space(self, width: float | None = None, height: float | None = None) -> None:
        """Empty room in a layout: ``height`` in a column, ``width`` in a
        row (a column's spacer fills its width)."""
        if not self._layouts:
            raise TypeError("ui.space only works inside ui.column(), ui.row() or ui.stack()")
        lay = self._layouts[-1]
        w = None if width is None else float(width)
        h = None if height is None else float(height)
        if lay.kind == "row":
            lay.next(w if w is not None else 0.0, h, 0.0, 0.0)
        else:
            lay.next(w, h if h is not None else 0.0, 0.0, 0.0)

    def _current_screen(self):
        ctx = self._screen_ctx
        if ctx is None:
            return None
        running = self._host._running
        owner = running[1].id if running else None
        return ctx[1] if ctx[0] == owner else None

    def _blocked(self):
        """Widgets drawn now are under a modal screen."""
        modal = self.screens.modal
        if modal is None:
            return False
        name = self._current_screen()
        return name is None or self.screens._layer(name) < self.screens._layer(modal)

    def _tag(self, start):
        """Marks HUD items from ``start`` on with the current screen's layer."""
        name = self._current_screen()
        if name is None:
            return
        layer = self.screens._layer(name)
        if layer:
            items = self._hud._items
            for item in items[start:]:
                self._layered.append((item, layer))

    # -- layouts --

    def column(self, x: float | None = None, y: float | None = None,
               width: float | None = None, height: float | None = None, *,
               gap: float = 10.0, padding: float = 0.0, anchor: str | None = None,
               margin: float | tuple[float, float] = 0.0, panel: bool | Color = False,
               title: str | None = None, id: Any = None) -> _LayoutScope:
        """Places the widgets drawn inside it one under another::

            with ctx.ui.column(width=300, anchor="center", panel=True, padding=24):
                ctx.ui.label("PAUSED", align="center")
                resume = ctx.ui.button("Resume")

        Widgets inside take no ``x, y`` and fill its ``width``. It goes at
        ``x, y``, or at ``anchor`` on the screen, or (inside another layout)
        where that one puts it. ``panel=True`` (or a colour) draws a backdrop
        that fits it, with an optional ``title``."""
        return self._layout("column", x, y, width, height, gap, padding, None, anchor,
                            margin, panel, title, id)

    def row(self, x: float | None = None, y: float | None = None,
            width: float | None = None, height: float | None = None, *,
            gap: float = 10.0, padding: float = 0.0, columns: int | None = None,
            anchor: str | None = None, margin: float | tuple[float, float] = 0.0,
            panel: bool | Color = False, title: str | None = None,
            id: Any = None) -> _LayoutScope:
        """Places the widgets drawn inside it side by side, left to right.
        With ``columns=n`` (and a width, given or from the layout around
        it) each widget gets an equal share."""
        return self._layout("row", x, y, width, height, gap, padding, columns, anchor,
                            margin, panel, title, id)

    def stack(self, x: float | None = None, y: float | None = None,
              width: float | None = None, height: float | None = None, *,
              padding: float = 0.0, anchor: str | None = None,
              margin: float | tuple[float, float] = 0.0, panel: bool | Color = False,
              id: Any = None) -> _LayoutScope:
        """Places the widgets drawn inside it on top of each other (an
        image with a label over it)."""
        return self._layout("stack", x, y, width, height, 0.0, padding, None, anchor,
                            margin, panel, None, id)

    def _layout(self, kind, x, y, w, h, gap, pad, columns, anchor, margin, panel, title, id):
        if (x is None) != (y is None):
            raise TypeError(f"ui.{kind}: give both x and y, or neither")
        if columns is not None and int(columns) < 1:
            raise ValueError("columns must be 1 or more")
        # Unnamed layouts are told apart by screen and anchor too, so one
        # appearing or going elsewhere doesn't hand its size to another.
        key = self._id(kind, (self._current_screen(), anchor), id)
        parent = self._layouts[-1] if self._layouts else None
        w = None if w is None else float(w)
        h = None if h is None else float(h)
        if anchor is not None:
            known = self._sizes.get(key, (0.0, 0.0))
            ox, oy = self.place(anchor, w if w is not None else known[0],
                                h if h is not None else known[1], margin)
        elif x is not None:
            ox, oy = float(x), float(y)
        elif parent is not None:
            # Inside another layout: its next slot (taken when this closes).
            ox, oy, w, h = parent.peek(w, h)
        else:
            ox, oy = 0.0, 0.0
        lay = _Layout(self, kind, key, ox, oy, w, h, float(gap), float(pad),
                      None if columns is None else int(columns), anchor, margin, panel, title)
        lay.parent = parent
        lay.items0 = len(self._hud._items)
        lay.rects0 = len(self._rects)
        lay.focus0 = len(self._focusables)
        self._layouts.append(lay)
        return _LayoutScope(self, lay)

    def _close(self, lay):
        if self._layouts and self._layouts[-1] is lay:
            self._layouts.pop()
        w, h = lay.size()
        if lay.anchor is not None:
            self._sizes[lay.key] = (w, h)
            x, y = self.place(lay.anchor, w, h, lay.margin)
            dx, dy = x - lay.x, y - lay.y
            if dx or dy:
                # The guess (last frame's size) was off: move what was drawn.
                s = self.scale
                for item in self._hud._items[lay.items0:]:
                    item["x"] += dx * s
                    item["y"] += dy * s
                rects = self._rects
                for i in range(lay.rects0, len(rects)):
                    rx, ry, rw, rh = rects[i]
                    rects[i] = (rx + dx * s, ry + dy * s, rw, rh)
                for i in range(lay.focus0, len(self._focusables)):
                    wid, (rx, ry, rw, rh) = self._focusables[i]
                    self._focusables[i] = (wid, (rx + dx * s, ry + dy * s, rw, rh))
                lay.x, lay.y = x, y
        if lay.panel is not False and lay.panel is not None:
            items = self._hud._items
            n = len(items)
            fill = self.theme.panel if lay.panel is True else lay.panel
            self._box(lay.x, lay.y, w, h, fill)
            if lay.title:
                size = self._size(None)
                self._text(lay.title, lay.x + 14, lay.y + 10 + size / 2, size,
                           self.theme.text_dim, "left")
            backdrop = items[n:]
            del items[n:]
            items[lay.items0:lay.items0] = backdrop
            s = self.scale
            self._rects.append((lay.x * s, lay.y * s, w * s, h * s))
        if lay.parent is not None and lay.anchor is None:
            lay.parent.next(w, h, w, h)

    def _place(self, kind, x, y, w, h, dw, dh):
        """A widget's (x, y, width, height) in UI units: as given, or from
        the layout it's drawn in."""
        if x is None or y is None:
            if x is not None or y is not None:
                raise TypeError(f"ui.{kind}: give both x and y, or neither")
            if not self._layouts:
                raise TypeError(f"ui.{kind} needs x and y, or to be inside ui.column(), "
                                "ui.row() or ui.stack()")
            return self._layouts[-1].next(None if w is None else float(w),
                                          None if h is None else float(h), dw, dh)
        return (float(x), float(y), float(dw if w is None else w), float(dh if h is None else h))

    def _press(self, wid, rect, enabled, focusable=True):
        """Hover / press tracking for one widget (``rect`` in UI units):
        (hovered, held, clicked, focused)."""
        s = self.scale
        rect = (rect[0] * s, rect[1] * s, rect[2] * s, rect[3] * s)
        self._rects.append(rect)
        blocked = self._blocked()
        focused = False
        if focusable and enabled and not blocked:
            self._focusables.append((wid, rect))
            if self._autofocus and self._focus is None:
                self._focus, self._autofocus = wid, False
            focused = self._focus == wid
            if focused:
                self._focus_drawn = True
        pos = self._pos
        free = self._active is None or self._active == wid
        hot = enabled and not blocked and free and pos is not None and _inside(pos, rect)
        clicked = False
        if hot and self._pressed:
            self._active = wid
        if self._active == wid:
            self._live = True
            if not self._down:
                clicked = hot
                self._active = None
        if focused and self._activate:
            clicked = True
            self._activate = False
        return hot, self._active == wid, clicked, focused

    # -- drawing helpers (UI units in, pixels out) --

    def _ring(self, x, y, w, h):
        """The focus ring around a widget."""
        t, c = 2.0, self.theme.focus
        self._rect(x + w / 2, y - t / 2 - 1, w + 2 * t + 2, t, c)
        self._rect(x + w / 2, y + h + t / 2 + 1, w + 2 * t + 2, t, c)
        self._rect(x - t / 2 - 1, y + h / 2, t, h + 2, c)
        self._rect(x + w + t / 2 + 1, y + h / 2, t, h + 2, c)

    def _box(self, x, y, w, h, fill, border=True):
        if border:
            self._rect(x + w / 2, y + h / 2, w, h, self.theme.border)
            self._rect(x + w / 2, y + h / 2, max(w - 2, 0), max(h - 2, 0), fill)
        else:
            self._rect(x + w / 2, y + h / 2, w, h, fill)

    def _size(self, size, height=None):
        if size is not None:
            return float(size)
        t = float(self.theme.text_size)
        return t if height is None else min(t, height * 0.5)

    def _rect(self, x, y, w, h, color):
        s = self.scale
        n = len(self._hud._items)
        self._hud.rect(x * s, y * s, w * s, h * s, _linear(color))
        self._tag(n)

    def _text(self, text, x, y, size, color, align, shadow=None, outline=None, font=None):
        if shadow is not None and shadow is not True and shadow is not False:
            shadow = _linear(shadow)
        if outline is not None and outline is not True and outline is not False:
            outline = _linear(outline)
        if font is None:
            font = self.theme.font
        s = self.scale
        n = len(self._hud._items)
        self._hud.text(text, x * s, y * s, size * s, _linear(color), align, shadow, outline,
                       font)
        self._tag(n)

    # -- widgets --

    @property
    def mouse_over(self) -> bool:
        """The cursor is over a widget or panel (as drawn last frame), or
        a slider is being dragged. Same as ``ctx.input.mouse_over_ui``."""
        return self._over

    @property
    def focused(self) -> bool:
        """A widget has keyboard / gamepad focus."""
        return self._focus is not None

    def label(
        self,
        text: str,
        x: float | None = None,
        y: float | None = None,
        size: float | None = None,
        color: Color | None = None,
        align: str = "left",
        shadow: Any = None,
        font: str | None = None,
    ) -> None:
        """Text whose first line's top is at ``y``. ``align`` says where
        ``x`` is: ``"left"``, ``"center"`` or ``"right"`` (in a layout:
        where in its slot). ``font`` is a ``.ttf`` / ``.otf`` file (None:
        ``theme.font``). Labels don't take clicks."""
        size = self._size(size)
        text = str(text)
        lines = text.split("\n")
        if x is None or y is None:
            # The text can't be measured on every runner; estimate its width.
            est = max(_text_ems(line) for line in lines) * size
            x0, y, w, _ = self._place("label", x, y, None, None, est, size * len(lines))
            a = str(align).lower()
            x = x0 + (w / 2 if a in ("center", "centre") else w if a == "right" else 0.0)
        self._text(text, float(x), float(y) + size * len(lines) / 2.0, size,
                   self.theme.text if color is None else color, align, shadow, font=font)

    def button(
        self,
        text: str,
        x: float | None = None,
        y: float | None = None,
        width: float | None = None,
        height: float | None = None,
        *,
        id: Any = None,
        enabled: bool = True,
        size: float | None = None,
    ) -> bool:
        """A button (160 × 44 unless told otherwise, or as wide as its
        layout); True on the frame it's clicked (pressed and released on
        it) or activated with Enter / Space / the gamepad's south button
        while focused. Disabled buttons are dimmed and never click."""
        x, y, w, h = self._place("button", x, y, width, height, 160.0, 44.0)
        hot, held, clicked, focused = self._press(self._id("button", text, id), (x, y, w, h),
                                                  bool(enabled))
        t = self.theme
        if not enabled:
            fill = _dim(t.button, 0.6)
        elif held and hot:
            fill = t.press
        elif hot or held or (focused and self._key_mode):
            fill = t.hover
        else:
            fill = t.button
        self._box(x, y, w, h, fill)
        if focused and self._key_mode:
            self._ring(x, y, w, h)
        size = self._size(size, h)
        nudge = 1.0 if held and hot else 0.0
        self._text(text, x + w / 2, y + h / 2 + nudge, size,
                   t.text if enabled else t.text_dim, "center")
        return clicked

    def toggle(
        self,
        text: str,
        value: bool,
        x: float | None = None,
        y: float | None = None,
        *,
        id: Any = None,
        enabled: bool = True,
        size: float | None = None,
    ) -> bool:
        """A checkbox with ``text`` beside it. Returns the value after this
        frame: flipped when it's clicked (on the box or the text) or
        activated while focused."""
        size = self._size(size)
        box = size * 1.2
        # The text can't be measured on every runner; estimate its width.
        est = box + size * 0.5 + _text_ems(str(text)) * size
        x, y, w, h = self._place("toggle", x, y, None, None, est, box)
        hot, held, clicked, focused = self._press(self._id("toggle", text, id), (x, y, w, box),
                                                  bool(enabled))
        value = bool(value) != clicked
        t = self.theme
        if focused and self._key_mode:
            self._ring(x, y, w, box)
        edge = t.hover if hot else t.border
        self._rect(x + box / 2, y + box / 2, box, box, edge if enabled else _dim(edge))
        self._rect(x + box / 2, y + box / 2, box - 2, box - 2, t.track)
        if value:
            inner = box * 0.6
            self._rect(x + box / 2, y + box / 2, inner, inner,
                       t.accent if enabled else _dim(t.accent))
        self._text(text, x + box + size * 0.5, y + box / 2, size,
                   t.text if enabled else t.text_dim, "left")
        return value

    def slider(
        self,
        value: float,
        x: float | None = None,
        y: float | None = None,
        width: float | None = None,
        height: float | None = None,
        *,
        min: float = 0.0,
        max: float = 1.0,
        step: float | None = None,
        id: Any = None,
        enabled: bool = True,
    ) -> float:
        """A horizontal slider from ``min`` to ``max`` (200 × 24 unless
        told otherwise): press or drag on it, or focus it and press left /
        right, to set the value. Returns the value after this frame,
        clamped (and rounded to ``step`` if given). Label it with
        :meth:`label`."""
        lo, hi = float(min), float(max)
        if not hi > lo:
            raise ValueError("slider max must be more than min")
        x, y, w, h = self._place("slider", x, y, width, height, 200.0, 24.0)
        knob = h * 0.6
        hot, held, _, focused = self._press(self._id("slider", None, id), (x, y, w, h),
                                            bool(enabled))
        v = float(value)
        mouse = self.mouse_position
        if held and mouse is not None:
            span = w - knob
            k = 0.0 if span <= 0 else (mouse[0] - x - knob / 2) / span
            v = lo + (hi - lo) * k
        if focused and self._slide:
            v += self._slide * (float(step) if step else (hi - lo) / 20.0)
            self._slide = 0
        if step:
            v = lo + round((v - lo) / float(step)) * float(step)
        v = lo if v < lo else hi if v > hi else v
        t = self.theme
        k = (v - lo) / (hi - lo)
        if focused and self._key_mode:
            self._ring(x, y, w, h)
        track_h = h / 4
        cy = y + h / 2
        self._rect(x + w / 2, cy, w, track_h, t.track)
        fill = (w - knob) * k + knob / 2
        accent = t.accent if enabled else _dim(t.accent)
        self._rect(x + fill / 2, cy, fill, track_h, accent)
        kx = x + knob / 2 + (w - knob) * k
        self._rect(kx, cy, knob, h, t.hover if (hot or held) and enabled else t.border)
        self._rect(kx, cy, knob - 2, h - 2, accent)
        return v

    def progress(
        self,
        value: float,
        x: float | None = None,
        y: float | None = None,
        width: float | None = None,
        height: float | None = None,
        *,
        max: float = 1.0,
        color: Color | None = None,
        text: Any = None,
    ) -> None:
        """A bar filled ``value / max`` of the way (a health bar, a loading
        bar), 200 × 18 unless told otherwise. ``text`` is drawn on it;
        True shows the percentage. Progress bars don't take clicks."""
        x, y, w, h = self._place("progress", x, y, width, height, 200.0, 18.0)
        top = float(max)
        k = 0.0 if top <= 0 else float(value) / top
        k = 0.0 if k < 0 else 1.0 if k > 1 else k
        t = self.theme
        self._box(x, y, w, h, t.track)
        if k > 0:
            fw = (w - 4) * k
            self._rect(x + 2 + fw / 2, y + h / 2, fw, h - 4,
                       t.accent if color is None else color)
        if text is True:
            text = f"{round(k * 100)}%"
        if text:
            self._text(str(text), x + w / 2, y + h / 2, self._size(None, h * 1.5),
                       t.text, "center", outline=(0.0, 0.0, 0.0, 0.85))

    def panel(
        self,
        x: float,
        y: float,
        width: float,
        height: float,
        *,
        color: Color | None = None,
        title: str | None = None,
    ) -> None:
        """A backdrop for a group of widgets (draw it first). Clicks on it
        belong to the UI, like clicks on its widgets. Layouts draw their
        own with ``panel=True``."""
        x, y, w, h = float(x), float(y), float(width), float(height)
        s = self.scale
        self._rects.append((x * s, y * s, w * s, h * s))
        t = self.theme
        self._box(x, y, w, h, t.panel if color is None else color)
        if title:
            size = self._size(None)
            self._text(title, x + 14, y + 10 + size / 2, size, t.text_dim, "left")

    def image(
        self,
        path: str,
        x: float | None = None,
        y: float | None = None,
        width: float | None = None,
        height: float | None = None,
        color: Color = (1.0, 1.0, 1.0, 1.0),
    ) -> None:
        """An image (project-relative path) stretched over the rectangle
        (64 × 64 unless told otherwise, or as wide as its layout). Images
        don't take clicks; put a :meth:`button` behind one for that."""
        x, y, w, h = self._place("image", x, y, width, height, 64.0, 64.0)
        s = self.scale
        n = len(self._hud._items)
        self._hud.image(path, (x + w / 2) * s, (y + h / 2) * s, (w * s, h * s), _linear(color))
        self._tag(n)


class _LayoutScope:
    """What :meth:`Ui.column`, :meth:`Ui.row` and :meth:`Ui.stack` return:
    use it with ``with``."""

    __slots__ = ("_ui", "_layout")

    def __init__(self, ui, layout):
        self._ui = ui
        self._layout = layout

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self._ui._close(self._layout)
        return False

    @property
    def width(self) -> float | None:
        """The layout's inside width (None: fits its contents)."""
        return self._layout.inner_w


def _linear(c):
    """An sRGB colour as the linear one the HUD blends with."""
    r, g, b, a = _vec(c, 4)

    def ch(v):
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    return (ch(r), ch(g), ch(b), a)


def _inside(pos, rect):
    x, y, w, h = rect
    return x <= pos[0] < x + w and y <= pos[1] < y + h


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
    "graph": (lambda a: a.graph, lambda a, v: _set_attr(a, "graph", v)),
    "state": (lambda a: a.state, None),
}
_TRANSFORM_FIELDS = {
    "position": (lambda t: t.position, lambda t, v: _set_attr(t, "position", v)),
    "rotation": (lambda t: t.rotation_euler, lambda t, v: _set_attr(t, "rotation_euler", v)),
    "scale": (lambda t: t.scale, lambda t, v: _set_attr(t, "scale", v)),
}
_FIELD_ALIASES = {("transform", "rotation_euler"): "rotation"}
_JOINT_FIELDS = {
    "angle": (lambda j: j.angle, None),
    "value": (lambda j: j.value, None),
    "attached": (lambda j: j.attached, None),
    "broken": (lambda j: j.broken, None),
    "force": (lambda j: j.force, None),
    "kind": (lambda j: j.kind, None),
    "connected": (lambda j: j.connected, lambda j, v: _set_attr(j, "connected", v)),
    "enabled": (lambda j: j.enabled, lambda j, v: _set_attr(j, "enabled", v)),
    "motor_speed": (lambda j: j.motor_speed, lambda j, v: _set_attr(j, "motor_speed", v)),
    "motor_target": (lambda j: j.motor_target, lambda j, v: _set_attr(j, "motor_target", v)),
    "limits": (lambda j: j.limits, lambda j, v: _set_attr(j, "limits", v)),
    "break_force": (lambda j: j.break_force, lambda j, v: _set_attr(j, "break_force", v)),
}
COMPONENTS = {
    "animation": (lambda e: e.animation, "no Animation (or its model isn't animated)",
                  _ANIMATION_FIELDS),
    "transform": (lambda e: e.transform, "no Transform", _TRANSFORM_FIELDS),
    "joint": (lambda e: e.joint, "no Joint", _JOINT_FIELDS),
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

_KEYBOARD = (
    tuple("abcdefghijklmnopqrstuvwxyz")
    + tuple("0123456789")
    + ("space", "enter", "escape", "tab", "backspace",
       "arrow_left", "arrow_right", "arrow_up", "arrow_down",
       "shift_left", "shift_right", "ctrl_left", "ctrl_right", "alt_left", "alt_right",
       "super_left", "super_right")
    + tuple(f"f{i}" for i in range(1, 13))
    + ("home", "end", "page_up", "page_down", "insert", "delete",
       "caps_lock", "num_lock", "scroll_lock",
       "minus", "equal", "comma", "period", "slash", "backslash", "semicolon", "quote",
       "backquote", "bracket_left", "bracket_right")
    + tuple(f"numpad_{i}" for i in range(10))
    + ("numpad_add", "numpad_subtract", "numpad_multiply", "numpad_divide", "numpad_decimal",
       "numpad_enter", "print_screen", "pause", "context_menu", "back_button")
)
#: Gamepad buttons as key names, on any connected pad. ``"pad_up"`` …
#: ``"pad_right"`` are the d-pad or the left stick pushed past halfway;
#: south / east / west / north are A / B / X / Y on an Xbox pad.
PAD_KEYS = ("pad_south", "pad_east", "pad_west", "pad_north", "pad_up", "pad_down",
            "pad_left", "pad_right", "pad_start", "pad_select", "pad_lb", "pad_rb",
            "pad_lt", "pad_rt", "pad_l3", "pad_r3")
#: The key names both runners know, in the order
#: ``ctx.input.any_just_pressed()`` checks them.
KEY_NAMES = _KEYBOARD + PAD_KEYS
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
    ``$XDG_DATA_HOME`` or ``~/.local/share`` elsewhere; in the browser
    :data:`WEB_DATA_DIR`, which the web player keeps in IndexedDB).
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
    elif sys.platform == "emscripten":
        base = WEB_DATA_DIR
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


#: Where games keep saves and settings in the browser: the web player
#: mounts IndexedDB storage here (Emscripten's IDBFS) before the game starts.
WEB_DATA_DIR = "/data"

#: Called with the path after a save or settings file is written or removed.
#: The web player adds one that copies the change to IndexedDB.
store_hooks = []


def _stored(path):
    for hook in list(store_hooks):
        try:
            hook(path)
        except Exception as exc:  # a broken hook mustn't lose the save
            print(f"automata: store hook failed: {exc}", file=sys.stderr)


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
        _stored(path)

    def remove(self, rel):
        path = self.path(rel)
        if path is not None and os.path.isfile(path):
            os.remove(path)
            _stored(path)

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


STRINGS_DIR = "lang"
"""The folder string tables live in, relative to the scene's folder."""


def _norm_language(code):
    """A language code made comparable: ``pt_BR`` and ``pt-br`` match."""
    return str(code).strip().lower().replace("_", "-")


def _system_language():
    """The player's language from the system (``"en-US"``), or None."""
    for var in ("LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"):
        value = os.environ.get(var, "").split(":")[0].split(".")[0].split("@")[0].strip()
        if value and value not in ("C", "POSIX"):
            return value.replace("_", "-")
    if sys.platform == "win32":
        try:
            import ctypes

            buf = ctypes.create_unicode_buffer(85)
            if ctypes.windll.kernel32.GetUserDefaultLocaleName(buf, 85):  # type: ignore[attr-defined]
                return buf.value
        except Exception:  # noqa: BLE001 - any failure: no system language
            pass
    try:
        import locale

        code = locale.getlocale()[0]
    except Exception:  # noqa: BLE001
        code = None
    if code and "_" in code and len(code.split("_")[0]) <= 3:
        return code.replace("_", "-")
    return None


class _Fill(dict):
    """``str.format_map`` values that leave unknown ``{names}`` in place."""

    def __init__(self, values):
        super().__init__(values)
        self.unknown = []

    def __missing__(self, name):
        self.unknown.append(name)
        return "{" + name + "}"


class Strings:
    """``ctx.tr``: text in the player's language, from the project's string
    tables (``lang/*.csv``)::

        ctx.ui.button(ctx.tr("menu.play"), x, y)
        ctx.hud.text(ctx.tr("hud.coins", count=coins), 24, 24)

    A table's first row names the languages (``key,en,es,ja``); each other
    row is a key and its text in each language. ``{name}`` in a text is
    filled from the keyword arguments. A missing key gives the key itself
    (with a warning), and an empty cell gives the first language's text.

    The language is ``ctx.settings.language``. ``languages``, ``name()``
    and ``font()`` describe the tables' languages, for a language picker.
    """

    def __init__(self, host, settings):
        self._host = host
        self._settings = settings
        self._loaded = False
        self._checked = 0.0
        self._stamp = None
        self._languages = []
        self._names = []
        self._fonts = []
        self._entries = {}
        self._warned = set()

    # -- reading the tables --

    def _dir(self):
        return os.path.join(self._host.backend.base_dir(), STRINGS_DIR)

    def _files(self):
        try:
            names = os.listdir(self._dir())
        except OSError:
            return []
        return sorted(n for n in names if n.lower().endswith(".csv")
                      and os.path.isfile(os.path.join(self._dir(), n)))

    def _signature(self):
        out = []
        for name in self._files():
            try:
                st = os.stat(os.path.join(self._dir(), name))
            except OSError:
                continue
            out.append((name, st.st_mtime_ns, st.st_size))
        return tuple(out)

    def _load(self):
        if not self._loaded:
            self._loaded = True
            self._checked = time.monotonic()
            self._read()
        elif getattr(self._host, "hot_reload", False) and time.monotonic() - self._checked > 0.5:
            self._checked = time.monotonic()
            if self._signature() != self._stamp:
                self._read()
                self._host.backend.log("info", "Reloaded the string tables (lang/*.csv)")
        return self

    def _read(self):
        self._stamp = self._signature()
        self._languages, self._names, self._fonts, self._entries = [], [], [], {}
        self._warned = set()
        for name in self._files():
            label = f"{STRINGS_DIR}/{name}"
            try:
                with open(os.path.join(self._dir(), name), encoding="utf-8-sig", newline="") as f:
                    text = f.read()
            except (OSError, UnicodeDecodeError) as exc:
                self._host.backend.log("warn", f"{label}: {exc}")
                continue
            for problem in self._add_file(label, text):
                self._host.backend.log("warn", problem)

    def _add_file(self, label, text):
        """Merges one CSV file; returns its problems (with lines)."""
        problems = []
        reader = csv.reader(text.splitlines(keepends=True))
        rows = []
        line = 1
        try:
            for cells in reader:
                rows.append((line, cells))
                line = reader.line_num + 1
        except csv.Error as exc:
            problems.append(f"{label}:{line}: {exc}")
        rows = [(n, c) for n, c in rows
                if any(x.strip() for x in c) and not c[0].lstrip().startswith("#")]
        if not rows:
            return [f"{label}: empty"]
        header = rows[0][1]
        columns = []
        for i, cell in enumerate(header[1:], start=2):
            code = cell.strip()
            if not code:
                columns.append(None)
                continue
            at = self._index_exact(code)
            if at is not None:
                if at in columns:
                    problems.append(f"{label}:1: column {i} repeats language {code}; ignoring it")
                    columns.append(None)
                    continue
            else:
                self._languages.append(code)
                self._names.append("")
                self._fonts.append("")
                for entry in self._entries.values():
                    entry[1].append(None)
                at = len(self._languages) - 1
            columns.append(at)
        if all(c is None for c in columns):
            return problems + [f"{label}:1: the first row should name the languages: key,en,es,…"]
        for n, cells in rows[1:]:
            key = cells[0].strip()
            source = f"{label}:{n}"
            if len(cells) > len(header) and any(cells[len(header):]):
                problems.append(f'{source}: more cells than languages (a comma inside text '
                                f'needs "quotes")')
            if not key:
                problems.append(f"{source}: text without a key in the first column")
                continue
            texts = [None] * len(self._languages)
            for col, cell in enumerate(cells[1:]):
                lang = columns[col] if col < len(columns) else None
                if lang is not None and cell:
                    texts[lang] = cell.replace("\\n", "\n")
            if key in ("@name", "@font"):
                target = self._names if key == "@name" else self._fonts
                for i, t in enumerate(texts):
                    if t is not None:
                        target[i] = t.strip()
            elif key.startswith("@"):
                problems.append(f"{source}: unknown row {key} (the special rows are @name and "
                                "@font)")
            elif key in self._entries:
                problems.append(f"{source}: {key} is already in {self._entries[key][0]}; "
                                "keeping that one")
            else:
                self._entries[key] = (source, texts)
        return problems

    def _index_exact(self, code):
        want = _norm_language(code)
        for i, lang in enumerate(self._languages):
            if _norm_language(lang) == want:
                return i
        return None

    def _index(self, language):
        """The column of ``language`` (``pt-BR`` also finds ``pt``)."""
        if not language:
            return None
        at = self._index_exact(language)
        if at is not None:
            return at
        base = _norm_language(language).split("-")[0]
        codes = [_norm_language(lang) for lang in self._languages]
        for i, c in enumerate(codes):
            if c == base:
                return i
        for i, c in enumerate(codes):
            if c.split("-")[0] == base:
                return i
        return None

    def _resolve(self, saved):
        """The language to use: the player's choice, else the system's,
        else the tables' first."""
        self._load()
        if not self._languages:
            return saved or self._system() or "en"
        for want in (saved, self._system()):
            i = self._index(want)
            if i is not None:
                return self._languages[i]
        return self._languages[0]

    def _system(self):
        backend = self._host.backend
        return backend.language() if hasattr(backend, "language") else _system_language()

    def _warn_once(self, tag, message):
        if tag not in self._warned:
            self._warned.add(tag)
            self._host.backend.log("warn", message)

    # -- the API --

    def __call__(self, key: str, /, language: str | None = None, **values: Any) -> str:
        """``key``'s text in the player's language (or ``language``), with
        ``{name}`` filled from ``values``: ``ctx.tr("hud.coins", count=3)``."""
        if not isinstance(key, str):
            raise TypeError(f"ctx.tr takes a key like 'menu.play', not {key!r}")
        self._load()
        lang = self.language if language is None else str(language)
        entry = self._entries.get(key)
        if entry is None:
            if not self._entries:
                self._warn_once(("none",), f"ctx.tr({key!r}): there are no string tables yet "
                                f"({STRINGS_DIR}/*.csv)")
            else:
                self._warn_once(("key", key), f"ctx.tr: there's no key {key!r} in "
                                f"{STRINGS_DIR}/*.csv{_did_you_mean(key, self._entries)}")
            text = key
        else:
            source, texts = entry
            i = self._index(lang)
            text = texts[i] if i is not None else None
            if text is None:
                text = next((t for t in texts[:1] if t is not None), None)
                if i is not None and i > 0:
                    self._warn_once(("gap", key, i),
                                    f"{source}: {key} has no {self._languages[i]} text; using "
                                    f"{self._languages[0]}")
                if text is None:
                    text = key
        if values:
            fill = _Fill(values)
            try:
                text = text.format_map(fill)
            except (ValueError, IndexError, KeyError, AttributeError, TypeError) as exc:
                self._warn_once(("fmt", key, lang), f"ctx.tr({key!r}): can't fill in "
                                f"{text!r} ({exc})")
                return text
            for name in fill.unknown:
                self._warn_once(("arg", key, name), f"ctx.tr({key!r}): the text has {{{name}}} "
                                f"but no {name}= was given")
        return text

    @property
    def language(self) -> str:
        """The language texts come in (``ctx.settings.language``)."""
        return self._settings.language

    @property
    def languages(self) -> tuple[str, ...]:
        """The tables' language codes, in column order (the first is the
        default): ``("en", "es", "ja")``."""
        return tuple(self._load()._languages)

    def name(self, language: str | None = None) -> str:
        """The language's name for a picker (the ``@name`` row:
        ``"Español"``), else its code. None: the current language."""
        self._load()
        lang = self.language if language is None else str(language)
        i = self._index(lang)
        return (self._names[i] if i is not None else "") or lang

    def font(self, language: str | None = None) -> str | None:
        """The language's font (the ``@font`` row), relative to the scene
        folder: it draws the characters other fonts lack. None: the
        current language."""
        self._load()
        i = self._index(self.language if language is None else str(language))
        return (self._fonts[i] if i is not None else "") or None

    def has(self, key: str) -> bool:
        """Whether a table has ``key``."""
        return key in self._load()._entries

    def __contains__(self, key: object) -> bool:
        return key in self._load()._entries

    def keys(self, prefix: str = "") -> list[str]:
        """Every key (starting with ``prefix``), sorted."""
        return sorted(k for k in self._load()._entries if k.startswith(prefix))

    def __repr__(self):
        return (f"Strings(language={self.language!r}, languages={self.languages!r}, "
                f"keys={len(self._entries)})")


class Settings:
    """``ctx.settings``: the player's settings, kept between runs and
    shared by every save slot.

    - ``volume`` (0 to 1) sets the master volume.
    - ``graphics`` (``"low"`` … ``"beautiful"``) sets the graphics preset.
    - ``language`` (``"en"``, ``"ja"``…) picks the text ``ctx.tr`` gives.
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
        self._ui_scale = None
        self._graphics = None
        self._language = None
        self._strings = None
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
            u = raw.get("ui_scale")
            if isinstance(u, (int, float)) and not isinstance(u, bool) and math.isfinite(u):
                self._ui_scale = min(max(float(u), 0.5), 3.0)
            lang = raw.get("language")
            if isinstance(lang, str) and lang.strip():
                self._language = lang.strip()
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
        if self._ui_scale is not None:
            data["ui_scale"] = self._ui_scale
        if self._graphics is not None:
            data["graphics"] = self._graphics
        if self._language is not None:
            data["language"] = self._language
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
        v = min(max(v, 0.0), 1.0)
        if self._load()._volume == v:
            return  # unchanged: safe to assign every frame (``ctx.ui.slider``)
        self._volume = v
        self._apply["volume"] = v
        self._dirty = True

    @property
    def ui_scale(self) -> float:
        """How big ``ctx.ui`` draws, 0.5 to 3 (1, the default, is the size
        the game was made at): for players on phones or big screens."""
        v = self._load()._ui_scale
        return 1.0 if v is None else v

    @ui_scale.setter
    def ui_scale(self, v: float) -> None:
        v = float(v)
        if not math.isfinite(v):
            raise ValueError(f"ui_scale is 0.5 to 3, not {v!r}")
        v = min(max(v, 0.5), 3.0)
        if self._load()._ui_scale == v:
            return
        self._ui_scale = v
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
        if self._load()._graphics == q:
            return
        self._graphics = q
        self._apply["graphics"] = q
        self._dirty = True

    @property
    def language(self) -> str:
        """The language ``ctx.tr`` gives text in: a code from the string
        tables' first row (``"en"``, ``"es"``, ``"ja"``). Until the player
        picks one it's the system's language if the tables have it, else
        their first language."""
        saved = self._load()._language
        if self._strings is None:
            return saved or "en"
        return self._strings._resolve(saved)

    @language.setter
    def language(self, code: str) -> None:
        if not isinstance(code, str):
            raise TypeError(f"ctx.settings.language takes a language code like 'en' or "
                            f"'pt-BR', not {type(code).__name__}")
        code = code.strip()
        strings = self._strings
        langs = strings.languages if strings is not None else ()
        if langs:
            i = strings._index(code)
            if i is None:
                raise ValueError(f"there's no language {code!r} in {STRINGS_DIR}/*.csv"
                                 f"{_did_you_mean(code, langs)} (languages: {', '.join(langs)})")
            code = langs[i]
        elif not code:
            raise ValueError("a language is a code like 'en' or 'pt-BR'")
        if self._load()._language == code:
            return
        self._language = code
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
                                 "(keys are like 'space', 'w', 'arrow_up', 'shift', "
                                 "'mouse_left' or 'pad_south')")
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
        v = _plain(value, f"ctx.settings[{key!r}]")
        values = self._load()._values
        old = values.get(key, _MISSING)
        if type(old) is type(v) and old == v:
            return  # unchanged: nothing to save
        values[key] = v
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
        self._ui_scale = None
        self._language = None
        self._dirty = True

    @property
    def path(self) -> str | None:
        """The settings file (None when kept in memory)."""
        return self._store.path("settings.json")

    def __repr__(self):
        return (f"Settings(volume={self.volume:g}, ui_scale={self.ui_scale:g}, "
                f"graphics={self.graphics!r}, language={self.language!r}, "
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
        #: Path finding over the navigation mesh (see :class:`Nav`).
        self.nav = Nav(host)
        #: Sight, hearing, state machines and behaviour trees (see :class:`AI`).
        self.ai = AI(self)
        self.particles = Particles(host)
        self.scene = SceneView(host)
        self.audio = Audio(host)
        self.hud = Hud()
        #: Buttons, toggles, sliders, progress bars, panels (see :class:`Ui`).
        self.ui = Ui(host, self.hud, self.input)
        store = _Store(host.save_dir, host.backend.log)
        #: The current save slot (see :class:`SaveData`).
        self.save = SaveData(store)
        #: The player's settings (see :class:`Settings`).
        self.settings = Settings(store, host.backend.graphics)
        #: Text in the player's language from ``lang/*.csv`` (see :class:`Strings`).
        self.tr = Strings(host, self.settings)
        self.settings._strings = self.tr
        self.hud._strings = self.tr
        self.input._settings = self.settings
        self.ui._settings = self.settings
        self.ui._strings = self.tr

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

    def base_dir(self):
        """The scene's folder: project paths (``ui/hud.ui.ron``) are
        relative to it. The default is the working folder, which is the
        scene's in Oberon."""
        return os.getcwd()

    def language(self):
        """The player's system language (``"en-US"``), or None: what
        ``ctx.settings.language`` starts as when the tables have it."""
        return _system_language()

    def raycast(self, origin, direction, max_distance, ignore_id):
        """``(entity id, point, normal, distance)`` or None. ``direction``
        is normalized."""
        return None

    def grounded(self, entity):
        return False

    def water_height(self, x, z):
        """The water's surface height at ``(x, z)``, or None."""
        return None

    def nav_ready(self):
        """Whether there's a navigation mesh (see :class:`NavGraph`)."""
        return getattr(self, "nav_graph", None) is not None

    def nav_path(self, start, end, partial):
        """``ctx.nav.path``: ``[(x, y, z, link), ...]`` or None."""
        graph = getattr(self, "nav_graph", None)
        return None if graph is None else graph.path(start, end, partial)

    def nav_nearest(self, point, max_distance):
        """``ctx.nav.nearest``: a point or None."""
        graph = getattr(self, "nav_graph", None)
        return None if graph is None else graph.nearest(point, max_distance)

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
                 module_prefix="automata_script_", internal_files=(), save_dir=None,
                 touch_controls=None):
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
        if touch_controls:
            # A game's build.ron settings (game.json): "sticks", "buttons",
            # "look", "show".
            self.ctx.input.touch_controls.set(**touch_controls)
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
        self.ctx.ui._begin()
        self.ctx.input._begin_touch()
        self._sync(records)
        if self.hot_reload and (elapsed - self._last_check > 0.4 or elapsed < self._last_check):
            self._last_check = elapsed
            self.reload_changed()
        self._deliver_animation_events()
        self._deliver_joint_breaks()
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

    def _deliver_joint_breaks(self):
        """Tells each entity whose joint just snapped: its script's
        ``on_joint_broken(entity, ctx)`` and the channel
        ``<name>:joint:broken`` (data: the force, in newtons)."""
        for e in list(self.entities.values()):
            rec = e._record.get("joint")
            broken = bool(rec and rec.get("broken"))
            if broken == e._joint_was_broken:
                continue
            e._joint_was_broken = broken
            if not broken:
                continue  # rebuilt: it can snap again
            if e.script and e.id in self.started:
                s = self.script(e.script)
                if s is not None:
                    self.call(s, "on_joint_broken", e)
            if e.id not in self.entities:
                continue
            channel = f"{e.name or e.id}:joint:broken"
            self.publish(Message(channel, float(rec.get("force", 0.0)), e, "broken"))

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
        self.ctx.ui._draw_documents()
        self.ctx.ui._layer_items(self.ctx.hud._items)
        self.ctx.input.touch_controls._draw(self.ctx.hud)
        out = FrameOutput(writes, self.ctx.audio._queue, self.ctx.hud._items,
                          self.ctx.input._lock_request, self.ctx.settings._take_apply())
        self.ctx.audio._queue = []
        self.ctx.hud._items = []
        self.ctx.input._lock_request = None
        self.ctx.ui._end()
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
