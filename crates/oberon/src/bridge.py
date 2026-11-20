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
        self._load(data)

    def _load(self, data):
        self._p = tuple(data.get("p", (0.0, 0.0, 0.0)))
        self._r = tuple(data.get("r", (0.0, 0.0, 0.0)))
        self._s = tuple(data.get("s", (1.0, 1.0, 1.0)))
        self._v = tuple(data["v"]) if data.get("v") is not None else None
        self._grounded = bool(data.get("g", False))
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

    def _writes(self):
        if not self._dirty:
            return None
        w = {"id": self.id}
        if "p" in self._dirty:
            w["p"] = list(self._p)
        if "r" in self._dirty:
            w["r"] = list(self._r)
        if "s" in self._dirty:
            w["s"] = list(self._s)
        if "v" in self._dirty and self._v is not None:
            w["v"] = list(self._v)
        if "attrs" in self._dirty:
            w["attrs"] = {k: list(v) if isinstance(v, tuple) else v for k, v in self.attributes.items()}
        self._dirty = set()
        return w

    def __repr__(self):
        return f"Entity({self.name!r})"


class Input:
    def __init__(self):
        self._down = set()
        self._prev = set()

    def _update(self, keys):
        self._prev = self._down
        self._down = set(keys)

    def pressed(self, key):
        return _key(key) in self._down

    def just_pressed(self, key):
        k = _key(key)
        return k in self._down and k not in self._prev

    def just_released(self, key):
        k = _key(key)
        return k in self._prev and k not in self._down

    def axis(self, negative, positive):
        return float(self.pressed(positive)) - float(self.pressed(negative))


def _key(key):
    k = str(getattr(key, "value", key)).lower()
    return {"shift": "shift_left", "ctrl": "ctrl_left", "alt": "alt_left", "left": "arrow_left",
            "right": "arrow_right", "up": "arrow_up", "down": "arrow_down"}.get(k, k)


class Time:
    delta = 0.0
    elapsed = 0.0
    frame = 0


class Physics:
    def grounded(self, entity):
        return bool(getattr(entity, "grounded", False))


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

    @property
    def entity_count(self):
        return len(self._r.entities)


class Context:
    def __init__(self, runner):
        self.input = Input()
        self.time = Time()
        self.physics = Physics()
        self.scene = SceneView(runner)

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


class Runner:
    def __init__(self):
        self.entities = {}
        self.scripts = {}
        self.started = set()
        self.ctx = Context(self)
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
        self.ctx.input._update(msg.get("keys", []))
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
        sys.stdout.flush()
        send({"op": "frame", "frame": t.frame, "writes": writes})

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
