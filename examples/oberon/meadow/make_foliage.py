#!/usr/bin/env python3
"""Writes Meadow's foliage models (CC0, no dependencies).

    python examples/oberon/meadow/make_foliage.py   # rewrites models/*.glb

Low-poly on purpose: foliage scatter draws a model hundreds of times, so a
flower should be tens of triangles, not thousands (a primitive sphere is
over two thousand). Every model stands on its origin.

- ``rock.glb``: a lumpy boulder, partly sunk into the ground.
- ``poppy.glb``: a stem and a red cup of petals.
- ``daisy.glb``: a stem and a flat ring of white petals round a yellow eye.
- ``tree.glb``: a trunk and two cones of leaves.
"""

from __future__ import annotations

import json
import math
import os
import random
import struct

HERE = os.path.dirname(os.path.abspath(__file__))
MODELS = os.path.join(HERE, "models")
STEM = (0.12, 0.3, 0.07)


# --- geometry: lists of triangles, each three (x, y, z) points --------------


def ring(radius, y, sides, phase=0.0):
    return [
        (radius * math.cos(phase + 2 * math.pi * i / sides), y,
         -radius * math.sin(phase + 2 * math.pi * i / sides))
        for i in range(sides)
    ]


def prism(radius, y0, y1, sides):
    lo, hi = ring(radius, y0, sides), ring(radius, y1, sides)
    tris = []
    for i in range(sides):
        j = (i + 1) % sides
        tris += [(lo[i], lo[j], hi[j]), (lo[i], hi[j], hi[i])]
    return tris


def cone(radius, y0, y1, sides, phase=0.0, base=True):
    """A cone from a ring at ``y0`` to a point at ``y1`` (below for a cup)."""
    rim = ring(radius, y0, sides, phase)
    tip, centre = (0.0, y1, 0.0), (0.0, y0, 0.0)
    tris = []
    for i in range(sides):
        j = (i + 1) % sides
        side = (rim[i], rim[j], tip) if y1 > y0 else (rim[j], rim[i], tip)
        tris.append(side)
        if base:
            tris.append((centre, rim[j], rim[i]) if y1 > y0 else (centre, rim[i], rim[j]))
    return tris


def both_sides(tris):
    return tris + [(a, c, b) for a, b, c in tris]


def rock():
    """An icosahedron, jittered and squashed, its bottom under the ground."""
    rng = random.Random(11)
    t = (1 + math.sqrt(5)) / 2
    points = [(-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0), (0, -1, t), (0, 1, t),
              (0, -1, -t), (0, 1, -t), (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1)]
    faces = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4),
             (11, 10, 2), (10, 7, 6), (7, 1, 8), (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8),
             (3, 8, 9), (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]
    verts = []
    for x, y, z in points:
        k = 0.3 / math.sqrt(x * x + y * y + z * z) * (0.8 + 0.4 * rng.random())
        verts.append((x * k * 1.2, y * k * 0.7 + 0.08, z * k))
    return [(verts[a], verts[b], verts[c]) for a, b, c in faces]


def poppy():
    stem = prism(0.012, 0.0, 0.62, 4)
    petals = both_sides(cone(0.075, 0.68, 0.6, 6, base=False))
    eye = cone(0.025, 0.64, 0.69, 5)
    return [(stem, STEM), (petals, (0.8, 0.06, 0.04)), (eye, (0.05, 0.04, 0.03))]


def daisy():
    stem = prism(0.01, 0.0, 0.55, 4)
    petals = []
    for i in range(10):
        a = 2 * math.pi * i / 10
        side = (-math.sin(a) * 0.018, 0.0, -math.cos(a) * 0.018)
        out = (math.cos(a), 0.0, -math.sin(a))
        root = (out[0] * 0.02, 0.56, out[2] * 0.02)
        tip = (out[0] * 0.075, 0.57, out[2] * 0.075)
        petals.append(((root[0] - side[0], 0.56, root[2] - side[2]), (root[0] + side[0], 0.56,
                       root[2] + side[2]), tip))
    eye = cone(0.022, 0.555, 0.58, 6)
    return [(stem, STEM), (both_sides(petals), (0.92, 0.9, 0.82)), (eye, (0.85, 0.62, 0.08))]


def tree():
    trunk = prism(0.16, 0.0, 1.0, 7)
    leaves = cone(0.95, 0.8, 2.3, 9) + cone(0.7, 1.6, 3.0, 9, phase=0.3)
    return [(trunk, (0.3, 0.17, 0.08)), (leaves, (0.1, 0.27, 0.08))]


# --- glTF --------------------------------------------------------------------


def flat(tris):
    """Triangles -> (positions, face normals), unwelded."""
    pos, nrm = [], []
    for a, b, c in tris:
        e1 = [b[i] - a[i] for i in range(3)]
        e2 = [c[i] - a[i] for i in range(3)]
        n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2],
             e1[0] * e2[1] - e1[1] * e2[0]]
        length = math.sqrt(sum(v * v for v in n)) or 1.0
        n = [round(v / length, 6) for v in n]
        for p in (a, b, c):
            pos.append([round(v, 6) for v in p])
            nrm.append(n)
    return pos, nrm


def glb(name, parts):
    """A binary glTF of ``parts`` (triangles, colour), one material each."""
    blob = bytearray()
    views, accessors, prims, materials = [], [], [], []

    def accessor(values, kind):
        while len(blob) % 4:
            blob.append(0)
        data = b"".join(struct.pack("<3f", *v) for v in values)
        views.append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(data),
                      "target": 34962})
        blob.extend(data)
        acc = {"bufferView": len(views) - 1, "componentType": 5126, "count": len(values),
               "type": kind}
        if kind == "VEC3":
            acc["min"] = [min(v[i] for v in values) for i in range(3)]
            acc["max"] = [max(v[i] for v in values) for i in range(3)]
        accessors.append(acc)
        return len(accessors) - 1

    for i, (tris, color) in enumerate(parts):
        pos, nrm = flat(tris)
        prims.append({"attributes": {"POSITION": accessor(pos, "VEC3"),
                                     "NORMAL": accessor(nrm, "VEC3")},
                      "material": i, "mode": 4})
        materials.append({"name": f"{name} {i + 1}", "pbrMetallicRoughness": {
            "baseColorFactor": [*color, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.85}})
    doc = {
        "asset": {"version": "2.0", "generator": "automata make_foliage.py"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0}],
        "meshes": [{"name": name, "primitives": prims}],
        "materials": materials,
        "buffers": [{"byteLength": len(blob)}],
        "bufferViews": views, "accessors": accessors,
    }
    text = json.dumps(doc, separators=(",", ":")).encode()
    while len(text) % 4:
        text += b" "
    total = 12 + 8 + len(text) + 8 + len(blob)
    return (struct.pack("<4sII", b"glTF", 2, total)
            + struct.pack("<I4s", len(text), b"JSON") + text
            + struct.pack("<I4s", len(blob), b"BIN\x00") + bytes(blob))


def build():
    """Every model's file name and bytes."""
    return {
        "rock.glb": glb("Rock", [(rock(), (0.2, 0.19, 0.17))]),
        "poppy.glb": glb("Poppy", poppy()),
        "daisy.glb": glb("Daisy", daisy()),
        "tree.glb": glb("Tree", tree()),
    }


if __name__ == "__main__":
    os.makedirs(MODELS, exist_ok=True)
    for name, data in build().items():
        path = os.path.join(MODELS, name)
        with open(path, "wb") as f:
            f.write(data)
        print(f"wrote {path} ({len(data)} bytes)")
