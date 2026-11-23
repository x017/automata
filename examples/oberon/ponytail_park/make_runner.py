#!/usr/bin/env python3
"""Generates ``models/runner.glb``: a low-poly runner with a ponytail and a
skirt for showing spring bones (no dependencies).

    python3 examples/oberon/ponytail_park/make_runner.py

The model is a skinned glTF with three clips (Idle, Walk, Run). The
ponytail (``Hair1`` … ``HairEnd``) and the skirt (``Skirt`` and its eight
panels) have bones but no animation: ``models/runner.glb.import.ron`` makes
them spring bones, so they swing on their own. It faces +Z, stands on its
origin and is 1.7 m tall. Made for AUTOMATA and released under CC0.
"""

from __future__ import annotations

import json
import math
import os
import struct

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "models", "runner.glb")

# --- Skeleton -------------------------------------------------------------

PANELS = 8
BONES: list[tuple[str, str | None, tuple[float, float, float]]] = [
    ("Root", None, (0.0, 0.0, 0.0)),
    ("Hips", "Root", (0.0, 0.92, 0.0)),
    ("Spine", "Hips", (0.0, 0.12, 0.0)),
    ("Chest", "Spine", (0.0, 0.18, 0.0)),
    ("Neck", "Chest", (0.0, 0.2, 0.0)),
    ("Head", "Neck", (0.0, 0.07, 0.0)),
    # The ponytail hangs from a tie high on the back of the head.
    ("Hair1", "Head", (0.0, 0.15, -0.11)),
    ("Hair2", "Hair1", (0.0, -0.03, -0.09)),
    ("Hair3", "Hair2", (0.0, -0.09, -0.035)),
    ("Hair4", "Hair3", (0.0, -0.1, -0.012)),
    ("Hair5", "Hair4", (0.0, -0.1, 0.0)),
    ("HairEnd", "Hair5", (0.0, -0.09, 0.0)),
]
for side, sx in (("L", 1.0), ("R", -1.0)):
    BONES += [
        (f"UpperArm{side}", "Chest", (0.2 * sx, 0.15, 0.0)),
        (f"Forearm{side}", f"UpperArm{side}", (0.0, -0.26, 0.0)),
        (f"Hand{side}", f"Forearm{side}", (0.0, -0.24, 0.0)),
        (f"Thigh{side}", "Hips", (0.09 * sx, -0.04, 0.0)),
        (f"Shin{side}", f"Thigh{side}", (0.0, -0.42, 0.0)),
        (f"Foot{side}", f"Shin{side}", (0.0, -0.42, 0.0)),
    ]
# The skirt: a waistband with eight panels of two bones each, flaring out.
WAIST = (0.15, 0.12)  # half-widths of the waist ellipse (x, z)
FLARE = (0.05, 0.04)  # how far each panel bone steps out
DROP = 0.16  # how far each panel bone steps down
BONES.append(("Skirt", "Hips", (0.0, 0.04, 0.0)))
for i in range(PANELS):
    a = 2 * math.pi * i / PANELS
    s, c = math.sin(a), math.cos(a)
    BONES += [
        (f"Skirt{i}A", "Skirt", (WAIST[0] * s, 0.0, WAIST[1] * c)),
        (f"Skirt{i}B", f"Skirt{i}A", (FLARE[0] * s, -DROP, FLARE[0] * c)),
        (f"Skirt{i}End", f"Skirt{i}B", (FLARE[1] * s, -DROP, FLARE[1] * c)),
    ]
INDEX = {name: i for i, (name, _, _) in enumerate(BONES)}


def world_position(name: str) -> tuple[float, float, float]:
    x = y = z = 0.0
    while name is not None:
        _, parent, (tx, ty, tz) = BONES[INDEX[name]]
        x, y, z = x + tx, y + ty, z + tz
        name = parent
    return (x, y, z)


POS = {name: world_position(name) for name, _, _ in BONES}

# --- Meshes ---------------------------------------------------------------

COLORS = {
    "Skin": (0.95, 0.76, 0.62, 1.0),
    "Shirt": (0.22, 0.5, 0.85, 1.0),
    "Skirt": (0.86, 0.24, 0.3, 1.0),
    "Hair": (0.32, 0.18, 0.09, 1.0),
    "Shoes": (0.14, 0.14, 0.17, 1.0),
    "Eyes": (0.08, 0.08, 0.1, 1.0),
}


class Part:
    """One material's triangles: positions, normals, skin weights."""

    def __init__(self) -> None:
        self.pos: list[tuple[float, float, float]] = []
        self.nrm: list[tuple[float, float, float]] = []
        self.joints: list[tuple[int, int, int, int]] = []
        self.weights: list[tuple[float, float, float, float]] = []
        self.tris: list[int] = []

    def vertex(self, p, n, influences) -> int:
        """Adds a vertex; `influences` is [(bone name, weight), …]."""
        inf = sorted(influences, key=lambda bw: -bw[1])[:4]
        total = sum(w for _, w in inf) or 1.0
        inf = [(INDEX[b], w / total) for b, w in inf] + [(0, 0.0)] * (4 - len(inf))
        self.pos.append(tuple(p))
        self.nrm.append(tuple(n))
        self.joints.append(tuple(j for j, _ in inf))
        self.weights.append(tuple(w for _, w in inf))
        return len(self.pos) - 1


PARTS = {name: Part() for name in COLORS}


def add(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def scale(a, k):
    return (a[0] * k, a[1] * k, a[2] * k)


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def norm(a):
    length = math.sqrt(a[0] ** 2 + a[1] ** 2 + a[2] ** 2) or 1.0
    return (a[0] / length, a[1] / length, a[2] / length)


def box(part: str, center, size, bone: str) -> None:
    """A box, flat shaded, all on one bone."""
    p = PARTS[part]
    hx, hy, hz = (s / 2 for s in size)
    faces = [
        ((1, 0, 0), [(1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1)]),
        ((-1, 0, 0), [(-1, -1, 1), (-1, 1, 1), (-1, 1, -1), (-1, -1, -1)]),
        ((0, 1, 0), [(-1, 1, -1), (-1, 1, 1), (1, 1, 1), (1, 1, -1)]),
        ((0, -1, 0), [(-1, -1, 1), (-1, -1, -1), (1, -1, -1), (1, -1, 1)]),
        ((0, 0, 1), [(-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]),
        ((0, 0, -1), [(1, -1, -1), (-1, -1, -1), (-1, 1, -1), (1, 1, -1)]),
    ]
    for n, corners in faces:
        ids = [
            p.vertex(add(center, (cx * hx, cy * hy, cz * hz)), n, [(bone, 1.0)])
            for cx, cy, cz in corners
        ]
        p.tris += [ids[0], ids[1], ids[2], ids[0], ids[2], ids[3]]


def sphere(part: str, center, radius, bone: str, rings=8, segments=12, keep=None) -> None:
    """A UV sphere on one bone; `keep(normal)` drops triangles it rejects."""
    p = PARTS[part]
    grid = []
    for r in range(rings + 1):
        theta = math.pi * r / rings
        row = []
        for s in range(segments + 1):
            phi = 2 * math.pi * s / segments
            n = (math.sin(theta) * math.sin(phi), math.cos(theta), math.sin(theta) * math.cos(phi))
            row.append(n)
        grid.append(row)
    ids = {}

    def vid(r, s):
        if (r, s) not in ids:
            n = grid[r][s]
            ids[(r, s)] = p.vertex(add(center, scale(n, radius)), n, [(bone, 1.0)])
        return ids[(r, s)]

    for r in range(rings):
        for s in range(segments):
            quad = [(r, s), (r + 1, s), (r + 1, s + 1), (r, s + 1)]
            if keep is not None:
                mid = norm(tuple(sum(grid[a][b][k] for a, b in quad) for k in range(3)))
                if not keep(mid):
                    continue
            a, b, c, d = (vid(*q) for q in quad)
            p.tris += [a, b, c, a, c, d]


def tube(part: str, rings, sides=8) -> None:
    """A smooth tube through `rings`: [(centre, radius, influences), …]."""
    p = PARTS[part]
    loops = []
    for i, (centre, radius, inf) in enumerate(rings):
        ahead = rings[min(i + 1, len(rings) - 1)][0]
        behind = rings[max(i - 1, 0)][0]
        axis = norm(sub(ahead, behind))
        side = norm(cross(axis, (1.0, 0.0, 0.0)))
        up = cross(side, axis)
        loop = []
        for k in range(sides):
            a = 2 * math.pi * k / sides
            n = add(scale(side, math.cos(a)), scale(up, math.sin(a)))
            loop.append(p.vertex(add(centre, scale(n, radius)), n, inf))
        loops.append(loop)
    for a_loop, b_loop in zip(loops, loops[1:]):
        for k in range(sides):
            a, b = a_loop[k], a_loop[(k + 1) % sides]
            c, d = b_loop[(k + 1) % sides], b_loop[k]
            p.tris += [a, d, c, a, c, b]
    # Cap the tip.
    tip = rings[-1]
    end = p.vertex(tip[0], norm(sub(rings[-1][0], rings[-2][0])), tip[2])
    last = loops[-1]
    for k in range(sides):
        p.tris += [last[k], end, last[(k + 1) % sides]]


def body() -> None:
    hips, spine, chest, neck, head = (POS[n] for n in ("Hips", "Spine", "Chest", "Neck", "Head"))
    box("Shirt", add(hips, (0, 0.03, 0)), (0.28, 0.14, 0.19), "Hips")
    box("Shirt", add(spine, (0, 0.09, 0)), (0.29, 0.2, 0.19), "Spine")
    box("Shirt", add(chest, (0, 0.1, 0)), (0.34, 0.22, 0.2), "Chest")
    box("Skin", add(neck, (0, 0.03, 0)), (0.08, 0.1, 0.08), "Neck")
    sphere("Skin", add(head, (0, 0.09, 0)), 0.12, "Head")
    for sx in (1, -1):
        sphere("Eyes", add(head, (0.045 * sx, 0.1, 0.105)), 0.018, "Head", 4, 6)
    # Hair: a cap over the top and back of the head, and a tie.
    sphere(
        "Hair",
        add(head, (0, 0.1, -0.012)),
        0.128,
        "Head",
        keep=lambda n: n[1] > 0.3 or n[2] < -0.15,
    )
    sphere("Hair", POS["Hair1"], 0.035, "Head", 4, 8)
    hair = ["Hair1", "Hair2", "Hair3", "Hair4", "Hair5", "HairEnd"]
    radii = [0.04, 0.05, 0.047, 0.04, 0.03, 0.012]
    rings = []
    for k, name in enumerate(hair):
        here = POS[name]
        if k == 0:
            inf = [("Head", 1.0)]
        elif name == "HairEnd":
            inf = [("Hair5", 1.0)]
        else:
            inf = [(hair[k - 1], 0.5), (name, 0.5)]
        rings.append((here, radii[k], inf))
        if k + 1 < len(hair):
            # Halfway along a bone moves with that bone alone.
            mid = scale(add(here, POS[hair[k + 1]]), 0.5)
            rings.append((mid, (radii[k] + radii[k + 1]) / 2, [(name, 1.0)]))
    tube("Hair", rings)
    for side in ("L", "R"):
        up, fore, hand = (POS[f"{n}{side}"] for n in ("UpperArm", "Forearm", "Hand"))
        box("Shirt", add(up, (0, -0.05, 0)), (0.1, 0.13, 0.1), f"UpperArm{side}")
        box("Skin", add(up, (0, -0.15, 0)), (0.075, 0.2, 0.075), f"UpperArm{side}")
        box("Skin", add(fore, (0, -0.12, 0)), (0.068, 0.26, 0.068), f"Forearm{side}")
        box("Skin", add(hand, (0, -0.05, 0.005)), (0.06, 0.1, 0.045), f"Hand{side}")
        thigh, shin, foot = (POS[f"{n}{side}"] for n in ("Thigh", "Shin", "Foot"))
        box("Skin", add(thigh, (0, -0.2, 0)), (0.11, 0.44, 0.11), f"Thigh{side}")
        box("Skin", add(shin, (0, -0.2, 0)), (0.09, 0.42, 0.09), f"Shin{side}")
        box("Shoes", add(foot, (0, -0.01, 0.04)), (0.1, 0.06, 0.21), f"Foot{side}")


def skirt(segments=32) -> None:
    """A flared skirt skinned to the eight panels, inside and out."""
    p = PARTS["Skirt"]
    band = POS["Skirt"]
    # Ring levels: (height below the band, ellipse half-widths, influences).
    levels = [
        (0.0, WAIST, lambda i, j, w: [("Skirt", 1.0)]),
        (DROP / 2, (WAIST[0] + FLARE[0] / 2, WAIST[1] + FLARE[0] / 2),
         lambda i, j, w: [(f"Skirt{i}A", 1 - w), (f"Skirt{j}A", w)]),
        (DROP, (WAIST[0] + FLARE[0], WAIST[1] + FLARE[0]),
         lambda i, j, w: [(f"Skirt{i}A", (1 - w) / 2), (f"Skirt{j}A", w / 2),
                          (f"Skirt{i}B", (1 - w) / 2), (f"Skirt{j}B", w / 2)]),
        (DROP * 1.5, (WAIST[0] + FLARE[0] + FLARE[1] / 2, WAIST[1] + FLARE[0] + FLARE[1] / 2),
         lambda i, j, w: [(f"Skirt{i}B", 1 - w), (f"Skirt{j}B", w)]),
        (DROP * 2, (WAIST[0] + FLARE[0] + FLARE[1], WAIST[1] + FLARE[0] + FLARE[1]),
         lambda i, j, w: [(f"Skirt{i}B", 1 - w), (f"Skirt{j}B", w)]),
    ]
    for inside in (False, True):
        grid = []
        for depth, (rx, rz), weights in levels:
            if inside:
                rx, rz = rx - 0.004, rz - 0.004
            row = []
            for s in range(segments):
                a = 2 * math.pi * s / segments
                f = a / (2 * math.pi) * PANELS
                i = int(f) % PANELS
                j = (i + 1) % PANELS
                w = f - int(f)
                pos = (band[0] + rx * math.sin(a), band[1] - depth, band[2] + rz * math.cos(a))
                # Outward and a little down, like the flare.
                n = norm((math.sin(a), 0.35, math.cos(a)))
                if inside:
                    n = scale(n, -1)
                row.append(p.vertex(pos, n, weights(i, j, w)))
            grid.append(row)
        for r in range(len(levels) - 1):
            for s in range(segments):
                a, b = grid[r][s], grid[r][(s + 1) % segments]
                c, d = grid[r + 1][(s + 1) % segments], grid[r + 1][s]
                if inside:
                    p.tris += [a, b, c, a, c, d]
                else:
                    p.tris += [a, c, b, a, d, c]


# --- Animation ------------------------------------------------------------


def quat(axis, angle):
    s = math.sin(angle / 2)
    return (axis[0] * s, axis[1] * s, axis[2] * s, math.cos(angle / 2))


def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    )


def euler(x=0.0, y=0.0, z=0.0):
    """Turn about Y, then X, then Z (radians)."""
    return qmul(qmul(quat((0, 1, 0), y), quat((1, 0, 0), x)), quat((0, 0, 1), z))


def gait(t, period, stride, knee, arm, elbow, bob, lean, twist, height):
    """One frame of a walk or run: {bone: rotation}, hips height."""
    phase = 2 * math.pi * t / period
    s = math.sin(phase)
    pose = {
        "Hips": euler(y=twist * s),
        "Spine": euler(x=lean, y=-twist * 0.6 * s),
        "Chest": euler(y=-twist * 0.6 * s),
        "Head": euler(x=-lean * 0.7),
    }
    for side, sign, offset in (("L", 1, 0.0), ("R", -1, math.pi)):
        swing = math.sin(phase + offset)
        forward = max(0.0, math.cos(phase + offset))
        pose[f"Thigh{side}"] = euler(x=-stride * swing)
        pose[f"Shin{side}"] = euler(x=0.15 + knee * forward**1.5)
        pose[f"Foot{side}"] = euler(x=-0.2 * swing)
        pose[f"UpperArm{side}"] = euler(x=arm * swing, z=0.1 * sign)
        pose[f"Forearm{side}"] = euler(x=-elbow)
    return pose, height + bob * math.sin(phase) ** 2


def idle(t, period):
    phase = 2 * math.pi * t / period
    breathe = math.sin(phase)
    pose = {
        "Hips": euler(),
        "Spine": euler(x=0.02 * breathe),
        "Chest": euler(x=0.03 * breathe),
        "Head": euler(y=0.12 * math.sin(phase / 2), x=-0.02 * breathe),
    }
    for side, sign in (("L", 1), ("R", -1)):
        pose[f"UpperArm{side}"] = euler(z=(0.1 + 0.02 * breathe) * sign)
        pose[f"Forearm{side}"] = euler(x=-0.15)
        pose[f"Thigh{side}"] = euler()
        pose[f"Shin{side}"] = euler(x=0.04)
        pose[f"Foot{side}"] = euler()
    return pose, 0.92 - 0.006 * (1 - math.cos(phase)) / 2


CLIPS = {
    "Idle": (2.4, lambda t: idle(t, 2.4)),
    "Walk": (1.1, lambda t: gait(t, 1.1, 0.45, 0.65, 0.35, 0.3, 0.02, 0.05, 0.08, 0.91)),
    "Run": (0.72, lambda t: gait(t, 0.72, 0.8, 1.25, 0.75, 1.3, 0.06, 0.18, 0.12, 0.88)),
}
FPS = 30


# --- glTF -----------------------------------------------------------------


class Glb:
    def __init__(self) -> None:
        self.bin = bytearray()
        self.views: list[dict] = []
        self.accessors: list[dict] = []

    def accessor(self, fmt: str, rows, ctype: int, kind: str, minmax=False) -> int:
        while len(self.bin) % 4:
            self.bin.append(0)
        start = len(self.bin)
        for row in rows:
            self.bin += struct.pack("<" + fmt, *(row if isinstance(row, tuple) else (row,)))
        self.views.append({"buffer": 0, "byteOffset": start, "byteLength": len(self.bin) - start})
        acc = {
            "bufferView": len(self.views) - 1,
            "componentType": ctype,
            "count": len(rows),
            "type": kind,
        }
        if minmax:
            cols = list(zip(*[r if isinstance(r, tuple) else (r,) for r in rows]))
            acc["min"] = [min(c) for c in cols]
            acc["max"] = [max(c) for c in cols]
        self.accessors.append(acc)
        return len(self.accessors) - 1


def build() -> bytes:
    for part in PARTS.values():
        part.__init__()
    body()
    skirt()
    g = Glb()
    primitives, materials = [], []
    for name, part in PARTS.items():
        if not part.tris:
            continue
        materials.append({
            "name": name,
            "pbrMetallicRoughness": {
                "baseColorFactor": list(COLORS[name]),
                "metallicFactor": 0.0,
                "roughnessFactor": 0.75,
            },
            "doubleSided": name == "Skirt",
        })
        primitives.append({
            "attributes": {
                "POSITION": g.accessor("3f", part.pos, 5126, "VEC3", minmax=True),
                "NORMAL": g.accessor("3f", part.nrm, 5126, "VEC3"),
                "JOINTS_0": g.accessor("4B", part.joints, 5121, "VEC4"),
                "WEIGHTS_0": g.accessor("4f", part.weights, 5126, "VEC4"),
            },
            "indices": g.accessor("I", part.tris, 5125, "SCALAR"),
            "material": len(materials) - 1,
        })
    nodes = []
    for name, parent, t in BONES:
        node = {"name": name, "translation": list(t)}
        kids = [INDEX[n] for n, p, _ in BONES if p == name]
        if kids:
            node["children"] = kids
        nodes.append(node)
    nodes.append({"name": "Runner", "mesh": 0, "skin": 0})
    inverse_binds = []
    for name, _, _ in BONES:
        x, y, z = POS[name]
        inverse_binds.append((1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -x, -y, -z, 1))
    skin = {
        "joints": list(range(len(BONES))),
        "skeleton": 0,
        "inverseBindMatrices": g.accessor("16f", inverse_binds, 5126, "MAT4"),
    }
    animations = []
    for clip, (length, frame) in CLIPS.items():
        count = round(length * FPS)
        times = [i / FPS for i in range(count + 1)]
        frames = [frame(min(t, length - 1e-6) if i < count else 0.0) for i, t in enumerate(times)]
        time_acc = g.accessor("f", times, 5126, "SCALAR", minmax=True)
        samplers, channels = [], []
        for bone in frames[0][0]:
            rotations = [f[0][bone] for f in frames]
            samplers.append({"input": time_acc, "output": g.accessor("4f", rotations, 5126, "VEC4")})
            channels.append({"sampler": len(samplers) - 1,
                             "target": {"node": INDEX[bone], "path": "rotation"}})
        hips = [(0.0, f[1], 0.0) for f in frames]
        samplers.append({"input": time_acc, "output": g.accessor("3f", hips, 5126, "VEC3")})
        channels.append({"sampler": len(samplers) - 1,
                         "target": {"node": INDEX["Hips"], "path": "translation"}})
        animations.append({"name": clip, "samplers": samplers, "channels": channels})
    doc = {
        "asset": {"version": "2.0", "generator": "AUTOMATA make_runner.py (CC0)"},
        "scene": 0,
        "scenes": [{"nodes": [0, len(BONES)]}],
        "nodes": nodes,
        "meshes": [{"name": "Runner", "primitives": primitives}],
        "materials": materials,
        "skins": [skin],
        "animations": animations,
        "buffers": [{"byteLength": len(g.bin)}],
        "bufferViews": g.views,
        "accessors": g.accessors,
    }
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    data = bytes(g.bin) + b"\0" * (-len(g.bin) % 4)
    chunks = struct.pack("<I4s", len(text), b"JSON") + text
    chunks += struct.pack("<I4s", len(data), b"BIN\0") + data
    return struct.pack("<4sII", b"glTF", 2, 12 + len(chunks)) + chunks


if __name__ == "__main__":
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    blob = build()
    with open(OUT, "wb") as f:
        f.write(blob)
    print(f"wrote {OUT} ({len(blob) // 1024} KB, {len(BONES)} bones)")
