"""The Material Workshop's textures, drawn with the standard library only.

Each texture is a function of pixel coordinates, so the PNGs come out the
same every time. Height fields become normal maps (OpenGL convention,
green up, as glTF and Automata use) by finite differences.

    python examples/oberon/workshop/textures.py   # writes textures/*.png
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TEXTURES = os.path.join(HERE, "textures")


# ----- PNG ------------------------------------------------------------------

def write_png(path, width, height, pixels):
    """Writes RGB pixels (a flat list of (r, g, b) tuples, rows from the top)."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # no filter
        for r, g, b in pixels[y * width:(y + 1) * width]:
            raw += bytes((r, g, b))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


# ----- noise ----------------------------------------------------------------

def _hash(x, y, seed):
    n = (x * 374761393 + y * 668265263 + seed * 2147483647) & 0xFFFFFFFF
    n = ((n ^ (n >> 13)) * 1274126177) & 0xFFFFFFFF
    return ((n ^ (n >> 16)) & 0xFFFF) / 65535.0


def value_noise(x, y, period, seed):
    """Smooth noise in 0..1 that tiles every ``period`` cells."""
    x0, y0 = math.floor(x), math.floor(y)
    fx, fy = x - x0, y - y0
    sx, sy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)

    def h(i, j):
        return _hash((x0 + i) % period, (y0 + j) % period, seed)

    top = h(0, 0) * (1 - sx) + h(1, 0) * sx
    bottom = h(0, 1) * (1 - sx) + h(1, 1) * sx
    return top * (1 - sy) + bottom * sy


def fbm(u, v, period, seed, octaves=4):
    """Layered noise over a tile (``u``, ``v`` in 0..1), tiling seamlessly."""
    total, amp, norm, p = 0.0, 1.0, 0.0, period
    for o in range(octaves):
        total += value_noise(u * p, v * p, p, seed + o) * amp
        norm += amp
        amp *= 0.5
        p *= 2
    return total / norm


def clamp8(v):
    return max(0, min(255, int(round(v * 255))))


def rgb(c):
    return (clamp8(c[0]), clamp8(c[1]), clamp8(c[2]))


def normal_map(heights, size, strength):
    """A normal map from a ``size``×``size`` height field (wrapping)."""
    out = []
    for y in range(size):
        for x in range(size):
            dx = heights[y * size + (x + 1) % size] - heights[y * size + (x - 1) % size]
            dy = heights[((y + 1) % size) * size + x] - heights[((y - 1) % size) * size + x]
            nx, ny, nz = -dx * strength, dy * strength, 1.0
            k = 1.0 / math.sqrt(nx * nx + ny * ny + nz * nz)
            out.append(rgb((nx * k * 0.5 + 0.5, ny * k * 0.5 + 0.5, nz * k * 0.5 + 0.5)))
    return out


def roughness_metal(values, metal):
    """glTF packing: green = roughness, blue = metalness."""
    return [(0, clamp8(r), clamp8(metal)) for r in values]


# ----- the textures ---------------------------------------------------------

def bricks(size=128):
    """Running-bond bricks: 4 courses, 2 bricks per course, in mortar."""
    base, height, rough = [], [], []
    rows, cols, mortar = 4, 2, 0.06
    for y in range(size):
        v = y / size
        row = int(v * rows)
        fv = v * rows - row
        for x in range(size):
            u = x / size
            shift = 0.5 if row % 2 else 0.0
            cu = (u * cols + shift) % cols
            col = int(cu)
            fu = cu - col
            edge = min(fu, 1 - fu) * 2.0 / cols * rows, min(fv, 1 - fv)
            in_mortar = edge[0] < mortar or edge[1] < mortar
            grain = fbm(u, v, 8, 11)
            if in_mortar:
                c = 0.55 + grain * 0.1
                base.append(rgb((c, c * 0.97, c * 0.92)))
                height.append(0.0)
                rough.append(1.0)
            else:
                tone = _hash(row, (col + int(shift * 2)) % cols, 5)
                r = 0.42 + tone * 0.18 + (grain - 0.5) * 0.18
                base.append(rgb((r, r * 0.42, r * 0.3)))
                bevel = min(1.0, min(edge) / (mortar * 2.2))
                height.append(0.6 * bevel + grain * 0.4)
                rough.append(0.75 + grain * 0.2)
    return base, normal_map(height, size, 2.5), roughness_metal(rough, 0.0)


def planks(size=128):
    """Crate boards: four planks with wood grain and dark seams."""
    base, height = [], []
    boards = 4
    for y in range(size):
        v = y / size
        b = int(v * boards)
        fv = v * boards - b
        for x in range(size):
            u = x / size
            n = fbm(u * 0.5, v, 4, 21 + b, 3)
            grain = 0.5 + 0.5 * math.sin((v * 40 + n * 6.0 + b * 1.7) * math.pi)
            seam = min(fv, 1 - fv) < 0.035
            frame = min(u, 1 - u) < 0.07
            t = 0.55 + grain * 0.2 + (_hash(b, 0, 9) - 0.5) * 0.15
            if frame:
                t *= 0.8
            if seam:
                t *= 0.35
            base.append(rgb((t * 0.82, t * 0.56, t * 0.32)))
            height.append(0.0 if seam else (0.8 if frame else 0.5) + grain * 0.08)
    return base, normal_map(height, size, 3.0)


def tiles(size=128):
    """Worn stone floor tiles, 2×2 per texture."""
    base, height, rough = [], [], []
    n = 2
    for y in range(size):
        v = y / size
        for x in range(size):
            u = x / size
            fu, fv = (u * n) % 1.0, (v * n) % 1.0
            tu, tv = int(u * n), int(v * n)
            edge = min(fu, 1 - fu, fv, 1 - fv)
            stone = fbm(u, v, 6, 31)
            tone = 0.45 + _hash(tu, tv, 3) * 0.12 + (stone - 0.5) * 0.25
            if edge < 0.03:
                base.append(rgb((0.18, 0.17, 0.16)))
                height.append(0.0)
                rough.append(1.0)
            else:
                base.append(rgb((tone, tone * 0.96, tone * 0.9)))
                height.append(min(1.0, edge / 0.06) * 0.7 + stone * 0.3)
                rough.append(0.55 + stone * 0.35)
    return base, normal_map(height, size, 2.0), roughness_metal(rough, 0.0)


def brushed(size=128):
    """Brushed metal: fine horizontal streaks of roughness, fully metal."""
    rough = []
    for y in range(size):
        streak = _hash(y, 0, 41)
        for x in range(size):
            n = value_noise(x / size * 4, y / size * 64, 64, 43)
            rough.append(0.22 + streak * 0.12 + n * 0.12)
    return roughness_metal(rough, 1.0)


def neon(size=128):
    """An arrow and a ring, white on black, for the glowing sign."""
    out = []
    for y in range(size):
        v = (y + 0.5) / size * 2 - 1
        for x in range(size):
            u = (x + 0.5) / size * 2 - 1
            r = math.hypot(u, v)
            ring = abs(r - 0.82) < 0.05
            # A right-pointing arrow.
            shaft = -0.5 < u < 0.15 and abs(v) < 0.1
            head = 0.05 < u < 0.55 and abs(v) < (0.55 - u) * 0.9
            lit = ring or shaft or head
            out.append((255, 255, 255) if lit else (0, 0, 0))
    return out


def grass_block(size=16):
    """A pixel-art grass block side: grass on top, dirt below."""
    out = []
    for y in range(size):
        for x in range(size):
            n = _hash(x, y, 51)
            lip = 3 + int(_hash(x, 0, 52) * 2.5)
            if y < lip:
                g = 0.45 + n * 0.25
                out.append(rgb((g * 0.45, g, g * 0.3)))
            else:
                d = 0.38 + n * 0.18
                out.append(rgb((d, d * 0.68, d * 0.45)))
    return out


def build(folder=TEXTURES):
    """Writes every texture into ``folder``; returns the file names."""
    os.makedirs(folder, exist_ok=True)
    out = {}

    def save(name, size, pixels):
        write_png(os.path.join(folder, name), size, size, pixels)
        out[name] = size

    b, bn, brm = bricks()
    save("brick.png", 128, b)
    save("brick_normal.png", 128, bn)
    save("brick_rm.png", 128, brm)
    w, wn = planks()
    save("planks.png", 128, w)
    save("planks_normal.png", 128, wn)
    t, tn, trm = tiles()
    save("tiles.png", 128, t)
    save("tiles_normal.png", 128, tn)
    save("tiles_rm.png", 128, trm)
    save("brushed_rm.png", 128, brushed())
    save("neon.png", 128, neon())
    save("grass_block.png", 16, grass_block())
    return out


if __name__ == "__main__":
    for name in build():
        print(f"wrote textures/{name}")
