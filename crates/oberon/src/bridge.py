"""Oberon's Play-mode script runner.

Oberon starts this file with the project's Python interpreter and talks to it
over stdin/stdout, one JSON object per line. Every frame Oberon sends the
scene state (transforms, character velocities, attributes, input); this
runner calls each behaviour script's `start(entity, ctx)` once and
`update(entity, ctx)` every frame, and replies with what the scripts changed.

The script-facing API (`entity`, `ctx`, loading, hot reload, errors) is
`_automata_script_api.py` next to this file: the same module `run_scene`
uses (`automata/script_api.py`), so scripts behave the same in the editor
and in the game. This file is only the driver: the JSON protocol, the
Console, and ray casts against the collider shapes Oberon sends.

It is pure standard-library Python on purpose: the editor works with any
`python3`, whether or not the `automata` package is installed.
"""

import json
import math
import os
import sys

import _automata_script_api as api

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


# --------------------------------------------------------------------------
# Ray casts against the collider shapes Oberon sends each frame
# --------------------------------------------------------------------------


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


class Mesh:
    """Collision triangles in a collider's local space, plus their bounds."""

    def __init__(self, flat_vertices, flat_indices):
        v = flat_vertices
        self.vertices = [(v[i], v[i + 1], v[i + 2]) for i in range(0, len(v) - 2, 3)]
        n = len(self.vertices)
        self.triangles = [
            tuple(flat_indices[i : i + 3])
            for i in range(0, len(flat_indices) - 2, 3)
            if all(0 <= j < n for j in flat_indices[i : i + 3])
        ]
        if self.vertices:
            lo = [min(p[k] for p in self.vertices) for k in range(3)]
            hi = [max(p[k] for p in self.vertices) for k in range(3)]
        else:
            lo = hi = [0.0, 0.0, 0.0]
        self.center = tuple((a + b) * 0.5 for a, b in zip(lo, hi))
        self.half = tuple((b - a) * 0.5 + 1e-4 for a, b in zip(lo, hi))


def _ray_mesh(o, d, mesh):
    """Closest triangle hit (both faces) in mesh space: (t, normal) or None."""
    if mesh is None:
        return None
    c = mesh.center
    if _ray_box((o[0] - c[0], o[1] - c[1], o[2] - c[2]), d, mesh.half) is None:
        return None
    best = None
    verts = mesh.vertices
    for i, j, k in mesh.triangles:
        a, b, cc = verts[i], verts[j], verts[k]
        e1 = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
        e2 = (cc[0] - a[0], cc[1] - a[1], cc[2] - a[2])
        # Moller-Trumbore.
        p = (d[1] * e2[2] - d[2] * e2[1], d[2] * e2[0] - d[0] * e2[2], d[0] * e2[1] - d[1] * e2[0])
        det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2]
        if abs(det) < 1e-12:
            continue
        inv = 1.0 / det
        s = (o[0] - a[0], o[1] - a[1], o[2] - a[2])
        u = (s[0] * p[0] + s[1] * p[1] + s[2] * p[2]) * inv
        if u < 0.0 or u > 1.0:
            continue
        q = (s[1] * e1[2] - s[2] * e1[1], s[2] * e1[0] - s[0] * e1[2], s[0] * e1[1] - s[1] * e1[0])
        v = (d[0] * q[0] + d[1] * q[1] + d[2] * q[2]) * inv
        if v < 0.0 or u + v > 1.0:
            continue
        t = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) * inv
        if t < 0.0 or (best is not None and t >= best[0]):
            continue
        n = (e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0])
        length = math.sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) or 1.0
        n = (n[0] / length, n[1] / length, n[2] / length)
        if n[0] * d[0] + n[1] * d[1] + n[2] * d[2] > 0:  # face the ray
            n = (-n[0], -n[1], -n[2])
        best = (t, n)
    return best


def raycast_shapes(shapes, origin, direction, max_distance, ignore_id=None, meshes=None):
    """The closest hit among collider shapes: ``(id, point, normal, t)``.

    Shapes are ``[id, "box", cx, cy, cz, qx, qy, qz, qw, hx, hy, hz]``,
    ``[id, "ball", cx, cy, cz, r]``,
    ``[id, "capsule", cx, cy, cz, qx, qy, qz, qw, half_height, r]`` or
    ``[id, "mesh", cx, cy, cz, qx, qy, qz, qw, key]``, where ``key`` names
    a :class:`Mesh` in ``meshes``.
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
            if kind == "box":
                hit = _ray_box(lo, ld, (s[9], s[10], s[11]))
            elif kind == "mesh":
                hit = _ray_mesh(lo, ld, (meshes or {}).get(s[9]))
            else:
                hit = _ray_capsule(lo, ld, s[9], s[10])
            if hit:
                hit = (hit[0], _qrot(q, hit[1]))
        if hit is None or hit[0] > max_distance:
            continue
        if best is None or hit[0] < best[3]:
            t, n = hit
            p = (o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t)
            best = (sid, p, n, t)
    return best


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------


class BridgeBackend(api.Backend):
    """Answers the shared API's questions from the current frame message."""

    def __init__(self):
        self.shapes = []
        self.meshes = {}
        self.playing = set()

    def raycast(self, origin, direction, max_distance, ignore_id):
        return raycast_shapes(self.shapes, origin, direction, max_distance, ignore_id, self.meshes)

    def source_playing(self, entity):
        return entity.id in self.playing

    def log(self, level, message):
        send({"op": "log", "level": level, "msg": message})

    def error(self, report):
        send({"op": "error", **report})

    def reloaded(self, path):
        send({"op": "reloaded", "file": path})


class Runner:
    """Turns frame messages into script calls, and the results into replies."""

    def __init__(self):
        self.backend = BridgeBackend()
        self.keys = api.KeySource()
        self.host = api.ScriptHost(
            self.backend,
            self.keys,
            on_error="report",
            hot_reload=True,
            module_prefix="oberon_script_",
            internal_files=[__file__],
        )

    @property
    def ctx(self):
        return self.host.ctx

    @property
    def entities(self):
        return self.host.entities

    def frame(self, msg):
        mouse = msg.get("mouse") or {}
        self.keys.update(
            msg.get("keys", []),
            mouse.get("buttons", []),
            mouse.get("delta", (0.0, 0.0)),
            mouse.get("locked", False),
        )
        self.backend.shapes = msg.get("cols", [])
        # Mesh geometry arrives once per session, keyed for "mesh" shapes.
        for key, verts, indices in msg.get("meshes", []):
            self.backend.meshes[key] = Mesh(verts, indices)
        self.backend.playing = set(msg.get("playing", []))
        out = self.host.frame(
            msg.get("entities", []),
            delta=msg.get("dt", 0.0),
            elapsed=msg.get("time", 0.0),
            frame=msg.get("frame", 0),
            screen=msg.get("screen"),
        )
        sys.stdout.flush()
        reply = {"op": "frame", "frame": self.host.ctx.time.frame, "writes": out.writes}
        if out.sounds:
            reply["sounds"] = out.sounds
        if out.hud:
            reply["hud"] = out.hud
        if out.lock is not None:
            reply["lock"] = out.lock
        send(reply)

    def stop(self):
        self.host.stop()
        sys.stdout.flush()


def main():
    sys.stdout = _ConsoleStream("info")
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
