"""Oberon's Play-mode script runner.

Oberon starts this file with the project's Python interpreter and talks to it
over stdin/stdout, one JSON object per line. Every frame Oberon sends the
scene state (transforms, character velocities, attributes, input); this
runner calls each behaviour script's `start(entity, ctx)` once and
`update(entity, ctx)` every frame, and replies with what the scripts changed.

It is pure standard-library Python on purpose: the editor works with any
`python3`, whether or not the `automata` package is installed. Scripts see
the same small API as the `automata` runtime (see the script template).
"""

import importlib.util
import json
import math
import os
import sys
import traceback

PROTOCOL = 1

_out = sys.stdout


def send(msg):
    _out.write(json.dumps(msg, separators=(",", ":")) + "\n")
    _out.flush()


class _ConsoleStream:
    """Redirects print() to Oberon's Console."""

    def __init__(self, level):
        self.level = level
        self.buf = ""

    def write(self, text):
        self.buf += text
        while "\n" in self.buf:
            line, self.buf = self.buf.split("\n", 1)
            send({"op": "log", "level": self.level, "msg": line})
        return len(text)

    def flush(self):
        if self.buf:
            send({"op": "log", "level": self.level, "msg": self.buf})
            self.buf = ""


sys.stdout = _ConsoleStream("info")


# --------------------------------------------------------------------------
# The script-facing API
# --------------------------------------------------------------------------


def _vec(v, n):
    t = tuple(float(x) for x in v)
    if len(t) != n:
        raise ValueError(f"expected {n} numbers, got {len(t)}")
    return t


class Transform:
    """`entity.transform`: position / rotation_euler (radians) / scale."""

    def __init__(self, entity):
        self._e = entity

    @property
    def position(self):
        return self._e._p

    @position.setter
    def position(self, v):
        self._e._p = _vec(v, 3)
        self._e._dirty.add("p")

    @property
    def rotation_euler(self):
        return self._e._r

    @rotation_euler.setter
    def rotation_euler(self, v):
        self._e._r = _vec(v, 3)
        self._e._dirty.add("r")

    @property
    def scale(self):
        return self._e._s

    @scale.setter
    def scale(self, v):
        self._e._s = _vec(v, 3)
        self._e._dirty.add("s")

    def translate(self, delta):
        d = _vec(delta, 3)
        self.position = tuple(a + b for a, b in zip(self.position, d))

    def rotate(self, delta):
        d = _vec(delta, 3)
        self.rotation_euler = tuple(a + b for a, b in zip(self.rotation_euler, d))

    def __repr__(self):
        return f"Transform(position={self.position}, rotation_euler={self.rotation_euler})"


class Camera:
    """An entity's perspective camera (``entity.camera`` / ``ctx.camera``).

    Play mode renders through the scene's first camera. Move it by setting
    ``position`` / ``target`` or with ``look_at`` and ``follow``.
    """

    def __init__(self, entity):
        self._e = entity

    def _get(self, a, b):
        return tuple(self._e._cam[a:b])

    def _set(self, a, values):
        cam = list(self._e._cam)
        cam[a:a + len(values)] = values
        self._e._cam = tuple(cam)
        self._e._dirty.add("cam")

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
        return self._e._cam[6]

    @fov.setter
    def fov(self, degrees):
        self._set(6, (min(179.0, max(1.0, float(degrees))),))

    def look_at(self, point):
        """Aims the camera at a point or an entity."""
        if isinstance(point, Entity):
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
            k = 1.0 - math.exp(-float(smooth) * max(_CTX.time.delta, 0.0)) if _CTX else 1.0
        cur = self.position
        self.position = tuple(cur[i] + (goal[i] - cur[i]) * k for i in range(3))
        self.target = (p[0], p[1] + float(look_height), p[2])

    def __repr__(self):
        return f"Camera(position={self.position}, target={self.target}, fov={self.fov})"


class Attributes(dict):
    """`entity.attributes`: a dict whose edits are sent back to Oberon."""

    def __init__(self, entity, values):
        super().__init__(values)
        self._e = entity

    def _touch(self):
        self._e._dirty.add("attrs")

    def __setitem__(self, key, value):
        if isinstance(value, list):
            value = tuple(value)
        super().__setitem__(key, value)
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

    def setdefault(self, key, default=None):
        if key not in self:
            self[key] = default
        return self[key]

    def clear(self):
        super().clear()
        self._touch()


class Entity:
    """A scene entity as seen by a script."""

    def __init__(self, data):
        self.id = data["id"]
        self.name = data.get("name", "")
        self.script = data.get("script")
        self.state = {}
        self.transform = Transform(self)
        self._dirty = set()
        self._audio = None
        self._despawn = False
        self._load(data)

    def _load(self, data):
        self._p = tuple(data.get("p", (0.0, 0.0, 0.0)))
        self._r = tuple(data.get("r", (0.0, 0.0, 0.0)))
        self._s = tuple(data.get("s", (1.0, 1.0, 1.0)))
        self._v = tuple(data["v"]) if data.get("v") is not None else None
        self._grounded = bool(data.get("g", False))
        self._cam = tuple(data["cam"]) if data.get("cam") is not None else None
        self._has_audio = bool(data.get("snd", False))
        attrs = {k: tuple(v) if isinstance(v, list) else v for k, v in data.get("attrs", {}).items()}
        self.attributes = Attributes(self, attrs)
        self._dirty = set()

    @property
    def character_velocity(self):
        return self._v

    @character_velocity.setter
    def character_velocity(self, v):
        self._v = _vec(v, 3)
        self._dirty.add("v")

    @property
    def grounded(self):
        return self._grounded

    @property
    def camera(self):
        """This entity's Camera, or None if it has no Camera component."""
        return Camera(self) if self._cam is not None else None

    @property
    def audio(self):
        """This entity's Audio Source: ``play()``, ``stop()``, ``playing``."""
        return EntityAudio(self)

    def despawn(self):
        """Removes the entity from the running scene (after this frame)."""
        self._despawn = True
        self._dirty.add("despawn")

    def _writes(self):
        if not self._dirty:
            return None
        w = {"id": self.id}
        if "despawn" in self._dirty:
            w["despawn"] = True
        if self._audio is not None:
            w["audio"] = self._audio
            self._audio = None
        if "p" in self._dirty:
            w["p"] = list(self._p)
        if "r" in self._dirty:
            w["r"] = list(self._r)
        if "s" in self._dirty:
            w["s"] = list(self._s)
        if "v" in self._dirty and self._v is not None:
            w["v"] = list(self._v)
        if "cam" in self._dirty and self._cam is not None:
            w["cam"] = list(self._cam)
        if "attrs" in self._dirty:
            w["attrs"] = {k: list(v) if isinstance(v, tuple) else v for k, v in self.attributes.items()}
        self._dirty = set()
        return w

    def __repr__(self):
        return f"Entity({self.name!r})"


class EntityAudio:
    """``entity.audio``: plays the entity's Audio Source component."""

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
        return _CTX is not None and self._e.id in _CTX._playing

    @property
    def source(self):
        """True-ish when the entity has an Audio Source (details live in Oberon)."""
        return self._e._has_audio or None

    def __repr__(self):
        return f"Audio({self._e.name!r})"


class Input:
    def __init__(self):
        self._down = set()
        self._prev = set()
        self._buttons = set()
        self._prev_buttons = set()
        self.mouse_delta = (0.0, 0.0)
        self._locked = False
        self._lock_request = None

    def _update(self, keys, buttons=(), delta=(0.0, 0.0), locked=False):
        self._prev = self._down
        self._down = set(keys)
        self._prev_buttons = self._buttons
        self._buttons = set(buttons)
        self.mouse_delta = (float(delta[0]), float(delta[1]))
        self._locked = bool(locked)

    def mouse_pressed(self, button="left"):
        return str(button).lower() in self._buttons

    def mouse_just_pressed(self, button="left"):
        b = str(button).lower()
        return b in self._buttons and b not in self._prev_buttons

    def mouse_just_released(self, button="left"):
        b = str(button).lower()
        return b in self._prev_buttons and b not in self._buttons

    def lock_mouse(self, locked=True):
        """Hides and captures the cursor for mouse-look (False releases it).
        In Oberon, Esc gives the cursor back; click the view to recapture."""
        self._lock_request = bool(locked)

    @property
    def mouse_locked(self):
        if self._lock_request is not None:
            return self._lock_request
        return self._locked

    def pressed(self, key):
        return any(k in self._down for k in _keys(key))

    def just_pressed(self, key):
        return any(k in self._down and k not in self._prev for k in _keys(key))

    def just_released(self, key):
        return any(k in self._prev and k not in self._down for k in _keys(key))

    def axis(self, negative, positive):
        return float(self.pressed(positive)) - float(self.pressed(negative))


_ALIASES = {
    "shift": ("shift_left", "shift_right"),
    "ctrl": ("ctrl_left", "ctrl_right"),
    "alt": ("alt_left", "alt_right"),
    "left": ("arrow_left",),
    "right": ("arrow_right",),
    "up": ("arrow_up",),
    "down": ("arrow_down",),
}


def _keys(key):
    k = str(getattr(key, "value", key)).lower()
    return _ALIASES.get(k, (k,))


class Time:
    delta = 0.0
    elapsed = 0.0
    frame = 0


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


def _qrot(q, v):
    """Rotates v by the unit quaternion q = (x, y, z, w)."""
    x, y, z, w = q
    vx, vy, vz = v
    # t = 2 * cross(q.xyz, v); v' = v + w*t + cross(q.xyz, t)
    tx = 2.0 * (y * vz - z * vy)
    ty = 2.0 * (z * vx - x * vz)
    tz = 2.0 * (x * vy - y * vx)
    return (
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    )


def _qinv(q):
    return (-q[0], -q[1], -q[2], q[3])


def _ray_box(o, d, h):
    """Slab test in box space; returns (t, normal) or None."""
    tmin, tmax = -math.inf, math.inf
    n_enter = None
    for i in range(3):
        if abs(d[i]) < 1e-12:
            if o[i] < -h[i] or o[i] > h[i]:
                return None
            continue
        t1 = (-h[i] - o[i]) / d[i]
        t2 = (h[i] - o[i]) / d[i]
        sign = -1.0
        if t1 > t2:
            t1, t2 = t2, t1
            sign = 1.0
        if t1 > tmin:
            tmin = t1
            n_enter = tuple(sign if j == i else 0.0 for j in range(3))
        tmax = min(tmax, t2)
        if tmin > tmax:
            return None
    if tmax < 0:
        return None
    if tmin < 0:  # inside: report a hit at the origin
        return 0.0, tuple(-x for x in d)
    return tmin, n_enter or (0.0, 1.0, 0.0)


def _ray_sphere(o, d, c, r):
    m = (o[0] - c[0], o[1] - c[1], o[2] - c[2])
    b = m[0] * d[0] + m[1] * d[1] + m[2] * d[2]
    cc = m[0] * m[0] + m[1] * m[1] + m[2] * m[2] - r * r
    if cc > 0 and b > 0:
        return None
    disc = b * b - cc
    if disc < 0:
        return None
    t = max(0.0, -b - math.sqrt(disc))
    p = (o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t)
    n = ((p[0] - c[0]) / r, (p[1] - c[1]) / r, (p[2] - c[2]) / r) if r > 0 else (0.0, 1.0, 0.0)
    return t, n


def _ray_capsule(o, d, hh, r):
    """Capsule along local Y: the closest of its cylinder and end spheres."""
    best = None
    a = d[0] * d[0] + d[2] * d[2]
    if a > 1e-12:
        b = o[0] * d[0] + o[2] * d[2]
        c = o[0] * o[0] + o[2] * o[2] - r * r
        disc = b * b - a * c
        if disc >= 0:
            t = (-b - math.sqrt(disc)) / a
            y = o[1] + d[1] * t
            if t >= 0 and -hh <= y <= hh:
                p = (o[0] + d[0] * t, o[2] + d[2] * t)
                best = (t, (p[0] / r, 0.0, p[1] / r))
    for cy in (-hh, hh):
        hit = _ray_sphere(o, d, (0.0, cy, 0.0), r)
        if hit and (best is None or hit[0] < best[0]):
            best = hit
    return best


def raycast_shapes(shapes, origin, direction, max_distance, ignore_id=None):
    """The closest hit among collider shapes: ``(id, point, normal, t)``.

    Shapes are ``[id, "box", cx, cy, cz, qx, qy, qz, qw, hx, hy, hz]``,
    ``[id, "ball", cx, cy, cz, r]`` or
    ``[id, "capsule", cx, cy, cz, qx, qy, qz, qw, half_height, r]``.
    """
    o = tuple(float(v) for v in origin)[:3]
    d = tuple(float(v) for v in direction)[:3]
    length = math.sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
    if length < 1e-9:
        return None
    d = (d[0] / length, d[1] / length, d[2] / length)
    best = None
    for s in shapes:
        sid, kind = s[0], s[1]
        if ignore_id is not None and sid == ignore_id:
            continue
        c = (s[2], s[3], s[4])
        if kind == "ball":
            hit = _ray_sphere(o, d, c, s[5])
        else:
            q = (s[5], s[6], s[7], s[8])
            qi = _qinv(q)
            lo = _qrot(qi, (o[0] - c[0], o[1] - c[1], o[2] - c[2]))
            ld = _qrot(qi, d)
            hit = _ray_box(lo, ld, (s[9], s[10], s[11])) if kind == "box" else _ray_capsule(lo, ld, s[9], s[10])
            if hit:
                hit = (hit[0], _qrot(q, hit[1]))
        if hit is None or hit[0] > max_distance:
            continue
        if best is None or hit[0] < best[3]:
            t, n = hit
            p = (o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t)
            best = (sid, p, n, t)
    return best


class Physics:
    def __init__(self, runner=None):
        self._r = runner
        self._shapes = []

    def grounded(self, entity):
        return bool(getattr(entity, "grounded", False))

    def raycast(self, origin, direction, max_distance=100.0, ignore=None):
        """The first collider along a ray, right now, or None.

        ``ignore`` skips one entity (usually the caster). Sensors are skipped.
        """
        ignore_id = getattr(ignore, "id", ignore)
        hit = raycast_shapes(self._shapes, origin, direction, float(max_distance), ignore_id)
        if hit is None:
            return None
        sid, p, n, t = hit
        entity = self._r.entities.get(sid) if self._r is not None else None
        return Hit(entity, p, n, t)


class Audio:
    """``ctx.audio``: one-shot sounds by file name."""

    def __init__(self):
        self._queue = []

    def play(self, path, volume=1.0, pitch=1.0, position=None):
        """Plays a sound file once (relative to the scene folder). With a
        ``position`` (a point or an entity) it fades with distance."""
        if isinstance(position, Entity):
            position = position.transform.position
        pos = list(_vec(position, 3)) if position is not None else None
        self._queue.append([str(path), float(volume), float(pitch), pos])


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


class SceneView:
    def __init__(self, runner):
        self._r = runner

    def find(self, name):
        for e in self._r.entities.values():
            if e.name == name:
                return e
        return None

    def __getitem__(self, name):
        e = self.find(name)
        if e is None:
            raise KeyError(f"no entity named {name!r}")
        return e

    def __iter__(self):
        return iter(list(self._r.entities.values()))

    def names(self):
        return [e.name for e in self._r.entities.values()]

    def by_id(self, entity_id):
        """The entity with this id (as in ``Hit.entity.id``), or None."""
        return self._r.entities.get(entity_id)

    @property
    def entity_count(self):
        return len(self._r.entities)


class Context:
    def __init__(self, runner):
        self.input = Input()
        self.time = Time()
        self.physics = Physics(runner)
        self.scene = SceneView(runner)
        self.audio = Audio()
        self.hud = Hud()
        self._playing = set()

    @property
    def camera(self):
        """The scene camera Play mode renders through (None if the scene has none)."""
        for e in self.scene:
            if e._cam is not None:
                return Camera(e)
        return None

    def log(self, *parts):
        send({"op": "log", "level": "info", "msg": " ".join(str(p) for p in parts)})

    def warn(self, *parts):
        send({"op": "log", "level": "warn", "msg": " ".join(str(p) for p in parts)})


# --------------------------------------------------------------------------
# Runner
# --------------------------------------------------------------------------


class Script:
    """A loaded behaviour module, reloaded when its file changes."""

    def __init__(self, path):
        self.path = path
        self.module = None
        self.mtime = None
        self.broken = False

    def load(self):
        self.mtime = _mtime(self.path)
        name = "oberon_script_" + str(abs(hash(self.path)))
        spec = importlib.util.spec_from_file_location(name, self.path)
        if spec is None or spec.loader is None:
            raise ImportError(f"cannot load {self.path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
        self.module = module
        self.broken = False


def _mtime(path):
    try:
        return os.stat(path).st_mtime
    except OSError:
        return None


def report(exc, path, during):
    """Sends a script error with the innermost frame inside the script."""
    tb = traceback.extract_tb(exc.__traceback__)
    line = None
    for frame in tb:
        if os.path.abspath(frame.filename) == os.path.abspath(path):
            line = frame.lineno
    if isinstance(exc, SyntaxError) and exc.lineno:
        line = exc.lineno
    # Drop the runner's own frames from the traceback shown to the user.
    here = os.path.abspath(__file__)
    user = [f for f in tb if os.path.abspath(f.filename) != here]
    text = "Traceback (most recent call last):\n" if user else ""
    text += "".join(traceback.format_list(user))
    text += "".join(traceback.format_exception_only(type(exc), exc))
    lines = text.rstrip().split("\n")
    send({
        "op": "error",
        "msg": f"{os.path.basename(path)}: {type(exc).__name__}: {exc} (in {during})",
        "file": path,
        "line": line or 0,
        "traceback": "\n".join(lines),
    })


_CTX = None


class Runner:
    def __init__(self):
        global _CTX
        self.entities = {}
        self.scripts = {}
        self.started = set()
        self.ctx = Context(self)
        _CTX = self.ctx
        self.last_check = 0.0

    def script(self, path):
        s = self.scripts.get(path)
        if s is None:
            s = self.scripts[path] = Script(path)
            try:
                s.load()
            except BaseException as exc:  # noqa: BLE001 - user code
                s.broken = True
                report(exc, path, "import")
        return s

    def hot_reload(self):
        for s in self.scripts.values():
            m = _mtime(s.path)
            if m is not None and m != s.mtime:
                try:
                    s.load()
                    send({"op": "log", "level": "info", "msg": f"Reloaded {os.path.basename(s.path)}"})
                    send({"op": "reloaded", "file": s.path})
                except BaseException as exc:  # noqa: BLE001 - user code
                    s.broken = True
                    s.mtime = m
                    report(exc, s.path, "reload")

    def call(self, script, fn_name, entity):
        fn = getattr(script.module, fn_name, None)
        if fn is None:
            return
        try:
            fn(entity, self.ctx)
        except BaseException as exc:  # noqa: BLE001 - user code
            # One report, then the script pauses until its file changes.
            script.broken = True
            report(exc, script.path, f"{fn_name}({entity.name})")

    def frame(self, msg):
        t = self.ctx.time
        t.delta = float(msg.get("dt", 0.0))
        t.elapsed = float(msg.get("time", 0.0))
        t.frame = int(msg.get("frame", 0))
        mouse = msg.get("mouse") or {}
        self.ctx.input._update(
            msg.get("keys", []),
            mouse.get("buttons", []),
            mouse.get("delta", (0.0, 0.0)),
            mouse.get("locked", False),
        )
        self.ctx.physics._shapes = msg.get("cols", [])
        self.ctx._playing = set(msg.get("playing", []))
        screen = msg.get("screen")
        if screen:
            self.ctx.hud.size = (int(screen[0]), int(screen[1]))
        seen = set()
        for data in msg.get("entities", []):
            e = self.entities.get(data["id"])
            if e is None:
                e = self.entities[data["id"]] = Entity(data)
            else:
                e._load(data)
                e.name = data.get("name", e.name)
            seen.add(e.id)
        for gone in set(self.entities) - seen:
            del self.entities[gone]
        if t.elapsed - self.last_check > 0.4 or t.elapsed < self.last_check:
            self.last_check = t.elapsed
            self.hot_reload()
        for e in list(self.entities.values()):
            if not e.script:
                continue
            s = self.script(e.script)
            if s.broken or s.module is None:
                continue
            if e.id not in self.started:
                self.started.add(e.id)
                self.call(s, "start", e)
                if s.broken:
                    continue
            self.call(s, "update", e)
        writes = [w for w in (e._writes() for e in self.entities.values()) if w]
        for e in [e for e in self.entities.values() if e._despawn]:
            del self.entities[e.id]
        sys.stdout.flush()
        reply = {"op": "frame", "frame": t.frame, "writes": writes}
        if self.ctx.audio._queue:
            reply["sounds"] = self.ctx.audio._queue
            self.ctx.audio._queue = []
        if self.ctx.hud._items:
            reply["hud"] = self.ctx.hud._items
            self.ctx.hud._items = []
        if self.ctx.input._lock_request is not None:
            reply["lock"] = self.ctx.input._lock_request
            self.ctx.input._lock_request = None
        send(reply)

    def stop(self):
        for e in list(self.entities.values()):
            if e.script and e.id in self.started:
                s = self.scripts.get(e.script)
                if s and s.module and not s.broken:
                    self.call(s, "stop", e)
        sys.stdout.flush()


def main():
    runner = Runner()
    send({"op": "ready", "protocol": PROTOCOL, "python": sys.version.split()[0]})
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            msg = json.loads(raw)
        except ValueError:
            send({"op": "log", "level": "warn", "msg": f"bridge: bad message {raw[:80]!r}"})
            continue
        op = msg.get("op")
        if op == "frame":
            runner.frame(msg)
        elif op == "stop":
            runner.stop()
            break
    send({"op": "bye"})


if __name__ == "__main__":
    main()
