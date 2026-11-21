"""Generates the sample models in this folder (no dependencies).

    python3 examples/models/make_models.py

- ``tree.glb``: a low-poly tree in binary glTF with two materials: a plain
  brown trunk and a textured canopy (the PNG is embedded in the file).
- ``crate.obj`` + ``crate.mtl`` + ``textures/crate.png``: a wooden crate in
  OBJ form, its material and texture in companion files.

Both stand on their origin (lowest point at y = 0), like most models
exported from Blender with "apply transforms".
"""

from __future__ import annotations

import json
import math
import os
import random
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


# --- PNG ------------------------------------------------------------------


def png(width: int, height: int, pixel) -> bytes:
    """An RGB PNG; `pixel(x, y)` returns an (r, g, b) tuple."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)  # filter: none
        for x in range(width):
            rows.extend(bytes(pixel(x, y)))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + chunk(b"IEND", b"")
    )


def leaves_texture() -> bytes:
    rng = random.Random(7)
    noise = [[rng.random() for _ in range(32)] for _ in range(32)]

    def pixel(x: int, y: int):
        n = noise[y][x] * 0.6 + noise[(y + 1) % 32][(x + 3) % 32] * 0.4
        return (int(40 + 40 * n), int(110 + 80 * n), int(40 + 30 * n))

    return png(32, 32, pixel)


def crate_texture() -> bytes:
    rng = random.Random(3)
    grain = [rng.random() for _ in range(64)]

    def pixel(x: int, y: int):
        border = x < 5 or x > 58 or y < 5 or y > 58
        brace = abs(x - y) < 4 or abs(x + y - 63) < 4
        plank_gap = y % 16 == 0
        g = grain[(x // 2 + y * 7) % 64]
        if border or brace:
            return (int(120 + 30 * g), int(78 + 20 * g), int(40 + 10 * g))
        if plank_gap:
            return (70, 45, 25)
        return (int(170 + 40 * g), int(120 + 30 * g), int(70 + 20 * g))

    return png(64, 64, pixel)


# --- geometry -------------------------------------------------------------


def flat(tris):
    """Unwelded triangles -> (positions, normals, uvs) with face normals."""
    pos, nrm, uv = [], [], []
    for tri in tris:
        (a, ua), (b, ub), (c, uc) = tri
        e1 = [b[i] - a[i] for i in range(3)]
        e2 = [c[i] - a[i] for i in range(3)]
        n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ]
        length = math.sqrt(sum(v * v for v in n)) or 1.0
        n = [v / length for v in n]
        for p, t in ((a, ua), (b, ub), (c, uc)):
            pos.append(p)
            nrm.append(n)
            uv.append(t)
    return pos, nrm, uv


def ring(radius: float, y: float, sides: int, phase: float = 0.0):
    return [
        (
            radius * math.cos(phase + 2 * math.pi * i / sides),
            y,
            -radius * math.sin(phase + 2 * math.pi * i / sides),
        )
        for i in range(sides)
    ]


def prism(r: float, y0: float, y1: float, sides: int):
    lo, hi = ring(r, y0, sides), ring(r, y1, sides)
    tris = []
    for i in range(sides):
        j = (i + 1) % sides
        u0, u1 = i / sides, (i + 1) / sides
        tris.append(((lo[i], (u0, 1)), (lo[j], (u1, 1)), (hi[j], (u1, 0))))
        tris.append(((lo[i], (u0, 1)), (hi[j], (u1, 0)), (hi[i], (u0, 0))))
    return tris


def cone(r: float, y0: float, y1: float, sides: int, phase: float):
    base = ring(r, y0, sides, phase)
    tip = (0.0, y1, 0.0)
    centre = (0.0, y0, 0.0)
    tris = []
    for i in range(sides):
        j = (i + 1) % sides
        u0, u1 = i / sides * 2, (i + 1) / sides * 2
        tris.append(((base[i], (u0, 1)), (base[j], (u1, 1)), (tip, ((u0 + u1) / 2, 0))))
        tris.append(((centre, (0.5, 0.5)), (base[j], (u1, 0)), (base[i], (u0, 0))))
    return tris


# --- glTF -----------------------------------------------------------------


def write_tree(path: str) -> None:
    trunk = flat(prism(0.14, 0.0, 0.9, 7))
    canopy = flat(
        cone(0.75, 0.7, 1.9, 9, 0.0) + cone(0.55, 1.3, 2.4, 9, 0.3)
    )
    texture = leaves_texture()

    blob = bytearray()
    views, accessors = [], []

    def add_view(data: bytes, target=None) -> int:
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target:
            view["target"] = target
        blob.extend(data)
        views.append(view)
        return len(views) - 1

    def add_accessor(values, kind: str) -> int:
        width = len(values[0])
        data = b"".join(struct.pack(f"<{width}f", *v) for v in values)
        acc = {
            "bufferView": add_view(data, 34962),
            "componentType": 5126,
            "count": len(values),
            "type": kind,
        }
        if kind == "VEC3":
            acc["min"] = [min(v[i] for v in values) for i in range(3)]
            acc["max"] = [max(v[i] for v in values) for i in range(3)]
        accessors.append(acc)
        return len(accessors) - 1

    def primitive(mesh, material: int) -> dict:
        pos, nrm, uv = mesh
        return {
            "attributes": {
                "POSITION": add_accessor(pos, "VEC3"),
                "NORMAL": add_accessor(nrm, "VEC3"),
                "TEXCOORD_0": add_accessor(uv, "VEC2"),
            },
            "material": material,
            "mode": 4,
        }

    prims = [primitive(trunk, 0), primitive(canopy, 1)]
    image_view = add_view(texture)
    doc = {
        "asset": {"version": "2.0", "generator": "automata make_models.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "Tree", "mesh": 0}],
        "meshes": [{"name": "Tree", "primitives": prims}],
        "materials": [
            {
                "name": "Bark",
                "pbrMetallicRoughness": {"baseColorFactor": [0.42, 0.27, 0.16, 1.0]},
            },
            {
                "name": "Leaves",
                "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}},
            },
        ],
        "textures": [{"source": 0}],
        "images": [{"bufferView": image_view, "mimeType": "image/png"}],
        "buffers": [{"byteLength": 0}],
        "bufferViews": views,
        "accessors": accessors,
    }
    while len(blob) % 4:
        blob.append(0)
    doc["buffers"][0]["byteLength"] = len(blob)
    text = json.dumps(doc, separators=(",", ":")).encode()
    while len(text) % 4:
        text += b" "
    total = 12 + 8 + len(text) + 8 + len(blob)
    with open(path, "wb") as f:
        f.write(struct.pack("<4sII", b"glTF", 2, total))
        f.write(struct.pack("<I4s", len(text), b"JSON") + text)
        f.write(struct.pack("<I4s", len(blob), b"BIN\x00") + bytes(blob))


# --- OBJ ------------------------------------------------------------------


def write_crate(folder: str) -> None:
    s = 0.45
    corners = [
        (x, y, z) for y in (0.0, 2 * s) for z in (s, -s) for x in (-s, s)
    ]
    # Faces as corner indices (counter-clockwise seen from outside).
    faces = [
        (0, 1, 5, 4),  # front (+z)
        (3, 2, 6, 7),  # back
        (2, 0, 4, 6),  # left
        (1, 3, 7, 5),  # right
        (4, 5, 7, 6),  # top
        (2, 3, 1, 0),  # bottom
    ]
    lines = ["# Wooden crate (generated by make_models.py)", "mtllib crate.mtl", "o Crate"]
    lines += [f"v {x:.3f} {y:.3f} {z:.3f}" for x, y, z in corners]
    lines += ["vt 0 0", "vt 1 0", "vt 1 1", "vt 0 1"]
    lines.append("usemtl Wood")
    for a, b, c, d in faces:
        lines.append(f"f {a + 1}/1 {b + 1}/2 {c + 1}/3 {d + 1}/4")
    with open(os.path.join(folder, "crate.obj"), "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(folder, "crate.mtl"), "w") as f:
        f.write("newmtl Wood\nKd 1 1 1\nmap_Kd textures/crate.png\n")
    os.makedirs(os.path.join(folder, "textures"), exist_ok=True)
    with open(os.path.join(folder, "textures", "crate.png"), "wb") as f:
        f.write(crate_texture())


if __name__ == "__main__":
    write_tree(os.path.join(HERE, "tree.glb"))
    write_crate(HERE)
    print("wrote tree.glb, crate.obj, crate.mtl, textures/crate.png")
