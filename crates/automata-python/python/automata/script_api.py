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
- ``view``: the engine's own handle (run_scene only).

Scripts read those values and write new ones. **Writes are visible to every
script that reads later in the same frame, and apply to the simulation
after all scripts have run.** That's why four drones doing
``player.attributes["health"] -= 5`` in one frame take 20 health, not 5.
The host returns the writes (plus sounds, HUD items and cursor-lock
requests) as a :class:`FrameOutput` for the driver to apply.
"""

import importlib.util
import math
import os
import sys
import traceback

__all__ = [
    "Attributes",
    "Camera",
    "Context",
    "Entity",
    "EntityAudio",
    "FrameOutput",
    "Hit",
    "Hud",
    "Input",
    "KeySource",
    "ScriptHost",
    "Time",
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
    def position(self):
        return self._e._read("p", (0.0, 0.0, 0.0))

    @position.setter
    def position(self, v):
        self._e._write("p", _vec(v, 3))

    @property
    def rotation_euler(self):
        return self._e._read("r", (0.0, 0.0, 0.0))

    @rotation_euler.setter
    def rotation_euler(self, v):
        self._e._write("r", _vec(v, 3))

    @property
    def scale(self):
        return self._e._read("s", (1.0, 1.0, 1.0))

    @scale.setter
    def scale(self, v):
        self._e._write("s", _vec(v, 3))

    def translate(self, delta):
        """Moves by ``delta`` (x, y, z)."""
        d = _vec(delta, 3)
        self.position = tuple(a + b for a, b in zip(self.position, d))

    def rotate(self, delta):
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
    def position(self):
        """The eye position (x, y, z)."""
        return self._get(0, 3)

    @position.setter
    def position(self, v):
        self._set(0, _vec(v, 3))

    @property
    def target(self):
        """The point the camera looks at (x, y, z)."""
        return self._get(3, 6)

    @target.setter
    def target(self, v):
        self._set(3, _vec(v, 3))

    @property
    def fov(self):
        """Vertical field of view in degrees."""
        return self._e._read("cam")[6]

    @fov.setter
    def fov(self, degrees):
        self._set(6, (min(179.0, max(1.0, float(degrees))),))

    def look_at(self, point):
        """Aims the camera at a point or an entity."""
        if hasattr(point, "transform"):
            point = point.transform.position
        self.target = point

    def follow(self, entity, offset=(0.0, 3.0, 8.0), smooth=8.0, look_height=0.6):
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

    def play(self):
        """Starts the sound (restarts it if it's already playing)."""
        self._e._audio = "play"
        self._e._dirty.add("audio")

    def stop(self):
        self._e._audio = "stop"
        self._e._dirty.add("audio")

    @property
    def playing(self):
        """Whether it was playing at the start of the frame."""
        return bool(self._e._host.backend.source_playing(self._e))

    @property
    def source(self):
        """The Audio Source (truthy), or None if the entity has none."""
        return self._e._record.get("snd") or None

    def __repr__(self):
        return f"Audio({self._e.name!r})"


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
        self._despawn = False

    def _begin(self, record):
        """Starts a frame from a fresh record."""
        self._record = record
        self._vals = {}
        self._attrs = None
        self._dirty = set()
        self._audio = None
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
    def transform(self):
        """Position, rotation (``rotation_euler``, radians) and scale."""
        return self._transform

    @transform.setter
    def transform(self, value):
        # Any transform-like object: ours, the engine's Transform, ...
        self._transform.position = value.position
        if hasattr(value, "rotation_euler"):
            self._transform.rotation_euler = value.rotation_euler
        if hasattr(value, "scale"):
            self._transform.scale = value.scale

    @property
    def character_velocity(self):
        """The Character Controller's velocity, or None without one."""
        return self._read("v")

    @character_velocity.setter
    def character_velocity(self, v):
        self._write("v", _vec(v, 3))

    @property
    def grounded(self):
        """True when a character is standing on something."""
        return bool(self._read("g", False))

    @property
    def attributes(self):
        """Custom values from the Inspector; read and write like a dict."""
        if self._attrs is None:
            raw = self._record.get("attrs") or {}
            self._attrs = Attributes(self, {k: _tuple(v) for k, v in raw.items()})
        return self._attrs

    @attributes.setter
    def attributes(self, values):
        self._attrs = Attributes(self, {k: _tuple(v) for k, v in dict(values).items()})
        self._dirty.add("attrs")

    @property
    def camera(self):
        """This entity's Camera, or None if it has no Camera component."""
        return Camera(self) if self._read("cam") is not None else None

    @property
    def camera_3d(self):
        """The engine's ``Camera3D`` (``run_scene`` only; prefer ``camera``)."""
        return self._host.backend.camera_3d(self)

    @camera_3d.setter
    def camera_3d(self, value):
        self._write("cam", (*_vec(value.position, 3), *_vec(value.target, 3), float(value.fov)))

    @property
    def audio(self):
        """This entity's Audio Source: ``play()``, ``stop()``, ``playing``."""
        return EntityAudio(self)

    @property
    def view(self):
        """The engine's own handle for the entity (``run_scene`` only)."""
        return self._record.get("view")

    def despawn(self):
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

    def pressed(self, key):
        """The key is held down."""
        return any(self._src.key_down(k) for k in keys_for(key))

    def just_pressed(self, key):
        """The key went down this frame."""
        return any(self._src.key_pressed(k) for k in keys_for(key))

    def just_released(self, key):
        """The key came up this frame."""
        return any(self._src.key_released(k) for k in keys_for(key))

    def axis(self, negative, positive):
        """-1, 0, or 1 from two keys."""
        return float(self.pressed(positive)) - float(self.pressed(negative))

    def mouse_pressed(self, button="left"):
        return self._src.button_down(str(button).lower())

    def mouse_just_pressed(self, button="left"):
        return self._src.button_pressed(str(button).lower())

    def mouse_just_released(self, button="left"):
        return self._src.button_released(str(button).lower())

    @property
    def mouse_delta(self):
        """Mouse movement since last frame (dx, dy) in pixels; +y is down."""
        dx, dy = self._src.mouse_delta()
        return (float(dx), float(dy))

    def lock_mouse(self, locked=True):
        """Hides and captures the cursor for mouse look (False frees it).
        In Oberon, Esc gives the cursor back; click the view to recapture."""
        self._lock_request = bool(locked)

    @property
    def mouse_locked(self):
        if self._lock_request is not None:
            return self._lock_request
        return bool(self._src.mouse_locked())

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

    def __getattr__(self, name):
        raw = self.__dict__.get("raw")
        if raw is None:
            raise AttributeError(f"ctx.time has no {name!r}")
        return getattr(raw, name)


class Hit:
    """A raycast hit: ``entity`` (or None), ``point``, ``normal``, ``distance``."""

    __slots__ = ("entity", "point", "normal", "distance")

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

    def grounded(self, entity):
        if isinstance(entity, Entity):
            return entity.grounded
        return bool(self._host.backend.grounded(entity))

    def raycast(self, origin, direction, max_distance=100.0, ignore=None):
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

    def water_height(self, x, z):
        """The height of the water surface at world ``(x, z)``, waves
        included (what floating bodies ride), or None where there's no
        water."""
        h = self._host.backend.water_height(float(x), float(z))
        return None if h is None else float(h)

    def __getattr__(self, name):
        return self._host.backend.extra("physics", name)


class Water:
    """``ctx.water``: the scene's water."""

    def __init__(self, host):
        self._host = host

    def height(self, x, z):
        """The height of the water surface at world ``(x, z)``, waves
        included, or None where there's no water (same as
        ``ctx.physics.water_height``)."""
        h = self._host.backend.water_height(float(x), float(z))
        return None if h is None else float(h)

    def splash(self, position, strength=1.0):
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

    def burst(self, effect, position, direction=None, count=None, **fields):
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

    def emit(self, entity, count):
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

    def __getattr__(self, name):
        return self._host.backend.extra("particles", name)


class Audio:
    """``ctx.audio``: one-shot sounds by file name."""

    def __init__(self, host):
        self._host = host
        self._queue = []

    def play(self, path, volume=1.0, pitch=1.0, position=None):
        """Plays a sound file once (relative to the scene folder). With a
        ``position`` (a point or an entity) it fades with distance."""
        if hasattr(position, "transform"):
            position = position.transform.position
        pos = list(_vec(tuple(position)[:3], 3)) if position is not None else None
        self._queue.append([str(path), float(volume), float(pitch), pos])

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
    def center(self):
        return (self.size[0] / 2.0, self.size[1] / 2.0)

    def image(self, path, x, y, size=None, color=(1.0, 1.0, 1.0, 1.0), rotation=0.0):
        item = {"k": "img", "path": str(path), "x": float(x), "y": float(y),
                "c": list(_vec(color, 4)), "rot": float(rotation)}
        if size is not None:
            item["w"], item["h"] = float(size[0]), float(size[1])
        self._items.append(item)

    def rect(self, x, y, width, height, color=(1.0, 1.0, 1.0, 1.0)):
        self._items.append({"k": "rect", "x": float(x), "y": float(y), "w": float(width),
                            "h": float(height), "c": list(_vec(color, 4))})

    def text(self, text, x, y, size=24.0, color=(1.0, 1.0, 1.0, 1.0), align="left",
             shadow=None, outline=None):
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

    def find(self, name):
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

    def names(self):
        return [e.name for e in self._host.entities.values()]

    def by_id(self, entity_id):
        """The entity with this id (as in ``Hit.entity.id``), or None."""
        return self._host.entities.get(entity_id)

    @property
    def entity_count(self):
        return len(self._host.entities)

    def __getattr__(self, name):
        return self._host.backend.extra("scene", name)


class Context:
    """The ``ctx`` every script function receives."""

    def __init__(self, host, input_source):
        self._host = host
        self.input = Input(input_source)
        self.time = Time()
        self.physics = Physics(host)
        self.water = Water(host)
        self.particles = Particles(host)
        self.scene = SceneView(host)
        self.audio = Audio(host)
        self.hud = Hud()

    @property
    def camera(self):
        """The scene camera Play renders through (None if the scene has none)."""
        for e in self._host.entities.values():
            if e._read("cam") is not None:
                return Camera(e)
        return None

    def log(self, *parts):
        """Writes to the Console (``print()`` works too)."""
        self._host.backend.log("info", " ".join(str(p) for p in parts))

    def warn(self, *parts):
        self._host.backend.log("warn", " ".join(str(p) for p in parts))

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
    or None.
    """

    __slots__ = ("writes", "sounds", "hud", "lock")

    def __init__(self, writes, sounds, hud, lock):
        self.writes = writes
        self.sounds = sounds
        self.hud = hud
        self.lock = lock


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
                 module_prefix="automata_script_", internal_files=()):
        self.backend = backend
        self.on_error = on_error
        self.hot_reload = hot_reload
        self.module_prefix = module_prefix
        self.internal = {os.path.abspath(__file__)} | {os.path.abspath(f) for f in internal_files}
        self.entities = {}
        self._by_name = {}
        self.scripts = {}
        self.started = set()
        self.ctx = Context(self, input_source if input_source is not None else KeySource())
        self._last_check = 0.0

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
        for record in records:
            eid = record.get("id")
            e = old.get(eid)
            if e is None:
                e = Entity(self, eid)
            e._begin(record)
            self.entities[eid] = e
            if e.name and e.name not in self._by_name:
                self._by_name[e.name] = e

    def _collect(self):
        writes = [w for w in (e._writes() for e in self.entities.values()) if w]
        for e in [e for e in self.entities.values() if e._despawn]:
            del self.entities[e.id]
            if self._by_name.get(e.name) is e:
                del self._by_name[e.name]
        out = FrameOutput(writes, self.ctx.audio._queue, self.ctx.hud._items,
                          self.ctx.input._lock_request)
        self.ctx.audio._queue = []
        self.ctx.hud._items = []
        self.ctx.input._lock_request = None
        return out

    def stop(self):
        """Calls ``stop(entity, ctx)`` on every started entity."""
        for e in list(self.entities.values()):
            if e.script and e.id in self.started:
                s = self.scripts.get(e.script)
                if s and s.module and not s.broken:
                    self.call(s, "stop", e)

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

    def call(self, script, fn_name, entity):
        fn = getattr(script.module, fn_name, None)
        if fn is None:
            return
        try:
            fn(entity, self.ctx)
        except BaseException as exc:  # noqa: BLE001 - user code
            # One report, then the script pauses until its file changes.
            script.broken = True
            self._failed(exc, script.path, f"{fn_name}({entity.name})")

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
