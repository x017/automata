#!/usr/bin/env python3
"""Draws the platformer's pixel art into ``sprites/`` (no dependencies).

    python examples/oberon/platformer/build_art.py

Every image is drawn here pixel by pixel from a small palette, so the
example needs no outside assets and anyone can change the look in code.

* ``tiles.png``: 8 x 4 tiles of 16 px. Tiles 0-15 are the ``Ground``
  auto-tile (which edges have grass or a rim depends on the neighbours:
  up 1, right 2, down 4, left 8); 16 is a brick block, 17 a wooden crate,
  18 a bush, 19 a flower, 20 tall grass, 21 a sign post.
* ``hero.png``: 4 x 2 frames: idle 0-1, run 2-5, jump 6, fall 7.
* ``coin.png``: 4 frames of a spinning coin (``coin_icon.png``: the
  first, for the HUD).
* ``slime.png``: 2 frames of a slime squashing as it hops.
* ``flag.png``: 2 frames of the goal flag waving.
* ``sky.png``: the backdrop: a dusk gradient, clouds and two rows of hills.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "sprites")

CLEAR = (0, 0, 0, 0)


def hexc(s, a=255):
    s = s.lstrip("#")
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16), a)


# A warm 16-colour palette.
INK = hexc("1a1c2c")
DIRT_DARK = hexc("5d3a2e")
DIRT = hexc("8b5a3c")
DIRT_LIGHT = hexc("a8754f")
GRASS_DARK = hexc("2f7d3c")
GRASS = hexc("4fb34a")
GRASS_LIGHT = hexc("9bdc5a")
STONE_DARK = hexc("5a5f73")
STONE = hexc("8b93a8")
STONE_LIGHT = hexc("c2c9d6")
WOOD_DARK = hexc("7a4426")
WOOD = hexc("c07a3a")
GOLD_DARK = hexc("c98a1b")
GOLD = hexc("ffd23f")
GOLD_LIGHT = hexc("fff4a3")
RED = hexc("e04a3f")
RED_DARK = hexc("a32a35")
SKIN = hexc("f4c08f")
BLUE = hexc("3b6fd1")
BLUE_DARK = hexc("27468f")
WHITE = hexc("f4f4f4")
SLIME = hexc("6fd6c8")
SLIME_DARK = hexc("2f9a9a")
PINK = hexc("ff8fb0")


class Image:
    def __init__(self, w, h, fill=CLEAR):
        self.w, self.h = w, h
        self.px = [fill] * (w * h)

    def set(self, x, y, c):
        if 0 <= x < self.w and 0 <= y < self.h and c[3]:
            self.px[y * self.w + x] = c

    def get(self, x, y):
        return self.px[y * self.w + x]

    def rect(self, x, y, w, h, c):
        for j in range(y, y + h):
            for i in range(x, x + w):
                self.set(i, j, c)

    def paste(self, other, ox, oy):
        for y in range(other.h):
            for x in range(other.w):
                self.set(ox + x, oy + y, other.get(x, y))

    def save(self, path):
        raw = b"".join(
            b"\0" + b"".join(bytes(self.px[y * self.w + x]) for x in range(self.w))
            for y in range(self.h)
        )

        def chunk(kind, data):
            body = kind + data
            return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

        png = b"\x89PNG\r\n\x1a\n"
        png += chunk(b"IHDR", struct.pack(">IIBBBBB", self.w, self.h, 8, 6, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(raw, 9))
        png += chunk(b"IEND", b"")
        with open(path, "wb") as f:
            f.write(png)


def noise(x, y, seed=0):
    """A repeatable 0..1 hash for speckles."""
    n = (x * 374761393 + y * 668265263 + seed * 1442695041) & 0xFFFFFFFF
    n = (n ^ (n >> 13)) * 1274126177 & 0xFFFFFFFF
    return (n ^ (n >> 16)) / 0xFFFFFFFF


# ---------------------------------------------------------------- tiles


def ground_tile(mask):
    """One auto-tile piece: dirt, grass where nothing is above, a dark rim
    on the sides with nothing next to them."""
    up, right, down, left = (mask & 1, mask & 2, mask & 4, mask & 8)
    t = Image(16, 16, DIRT)
    for y in range(16):
        for x in range(16):
            n = noise(x, y, mask)
            if n > 0.86:
                t.set(x, y, DIRT_DARK)
            elif n < 0.08:
                t.set(x, y, DIRT_LIGHT)
    # Pebbles.
    for px, py in ((4, 9), (11, 12), (8, 5)):
        if noise(px, py, mask + 7) > 0.4:
            t.set(px, py, DIRT_LIGHT)
            t.set(px + 1, py, DIRT_DARK)
    if not left:
        t.rect(0, 0, 1, 16, DIRT_DARK)
    if not right:
        t.rect(15, 0, 1, 16, DIRT_DARK)
    if not down:
        t.rect(0, 15, 16, 1, DIRT_DARK)
    if not up:
        # A grass lip, its lower edge ragged.
        t.rect(0, 0, 16, 4, GRASS)
        t.rect(0, 0, 16, 1, GRASS_LIGHT)
        for x in range(16):
            depth = 4 + (1 if noise(x, 0, mask + 3) > 0.5 else 0) + (x % 5 == 2)
            t.rect(x, 4, 1, depth - 4, GRASS)
            t.set(x, depth, GRASS_DARK)
        for x in range(0, 16, 3):
            t.set(x + 1, 1, GRASS_LIGHT)
        if not left:
            t.rect(0, 0, 1, 5, GRASS_DARK)
        if not right:
            t.rect(15, 0, 1, 5, GRASS_DARK)
    return t


def brick_tile():
    t = Image(16, 16, STONE)
    for row in range(4):
        y = row * 4
        t.rect(0, y + 3, 16, 1, STONE_DARK)
        t.rect(0, y, 16, 1, STONE_LIGHT)
        off = 0 if row % 2 == 0 else 4
        for x in range(off, 16, 8):
            t.rect(x, y, 1, 3, STONE_DARK)
    t.rect(0, 0, 1, 16, STONE_LIGHT)
    t.rect(15, 0, 1, 16, STONE_DARK)
    return t


def crate_tile():
    t = Image(16, 16, WOOD)
    t.rect(0, 0, 16, 2, WOOD_DARK)
    t.rect(0, 14, 16, 2, WOOD_DARK)
    t.rect(0, 0, 2, 16, WOOD_DARK)
    t.rect(14, 0, 2, 16, WOOD_DARK)
    for i in range(12):
        t.set(2 + i, 2 + i, WOOD_DARK)
        t.set(3 + i, 2 + i, WOOD_DARK)
    for x in (5, 9):
        t.rect(x, 3, 1, 1, DIRT_LIGHT)
    return t


def bush_tile():
    t = Image(16, 16)
    for y in range(16):
        for x in range(16):
            for cx, cy, r in ((4, 11, 4.5), (11, 11, 4.5), (8, 8, 5.5)):
                if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                    shade = GRASS_LIGHT if y < cy - r * 0.4 else GRASS
                    t.set(x, y, GRASS_DARK if y > 13 else shade)
    t.set(6, 7, GRASS_LIGHT)
    t.set(10, 9, GRASS_LIGHT)
    return t


def flower_tile():
    t = Image(16, 16)
    t.rect(7, 9, 1, 7, GRASS_DARK)
    t.set(6, 12, GRASS)
    t.set(8, 11, GRASS)
    for dx, dy in ((0, -2), (2, 0), (0, 2), (-2, 0), (-1, -1), (1, -1), (1, 1), (-1, 1)):
        t.set(7 + dx, 7 + dy, PINK)
    t.set(7, 7, GOLD)
    return t


def grass_tile():
    t = Image(16, 16)
    for x, h in ((2, 5), (4, 8), (6, 4), (9, 7), (11, 5), (13, 9)):
        for y in range(h):
            t.set(x + (1 if y > h - 3 else 0) - 1, 15 - y, GRASS if y < h - 2 else GRASS_LIGHT)
    return t


def sign_tile():
    t = Image(16, 16)
    t.rect(7, 8, 2, 8, WOOD_DARK)
    t.rect(2, 3, 12, 6, WOOD)
    t.rect(2, 3, 12, 1, DIRT_LIGHT)
    t.rect(2, 8, 12, 1, WOOD_DARK)
    # An arrow pointing right.
    t.rect(4, 5, 6, 2, WOOD_DARK)
    for i in range(3):
        t.rect(10 + i, 4 + i, 1, 4 - 2 * i, WOOD_DARK)
    return t


def tileset():
    sheet = Image(16 * 8, 16 * 4)
    tiles = [ground_tile(m) for m in range(16)]
    tiles += [brick_tile(), crate_tile(), bush_tile(), flower_tile(), grass_tile(), sign_tile()]
    for i, t in enumerate(tiles):
        sheet.paste(t, (i % 8) * 16, (i // 8) * 16)
    return sheet


# ----------------------------------------------------------------- hero


def hero_frame(body_y=0, legs=(0, 0), arm=0, squash=False, airborne=None):
    """A small adventurer facing right: red cap, blue overalls. ``legs``
    pushes the back/front foot forward (px); ``airborne`` is "up"/"down"."""
    f = Image(16, 16)
    y0 = 2 + body_y
    # Cap and face.
    f.rect(5, y0, 6, 2, RED)
    f.rect(10, y0 + 1, 3, 1, RED_DARK)  # peak
    f.rect(5, y0 + 2, 6, 3, SKIN)
    f.set(9, y0 + 3, INK)  # eye
    f.set(4, y0 + 3, SKIN)  # ear
    f.rect(5, y0 + 2, 1, 1, hexc("6b3b1f"))  # hair
    # Body.
    top = y0 + 5
    f.rect(5, top, 6, 4 if not squash else 3, BLUE)
    f.rect(6, top, 4, 1, RED)  # shirt at the neck
    f.set(6, top + 1, GOLD)
    f.set(9, top + 1, GOLD)
    # Arms.
    if airborne == "up":
        f.rect(11, top - 2, 1, 3, SKIN)
        f.rect(4, top, 1, 2, SKIN)
    else:
        f.rect(4, top + 1 + (arm < 0), 1, 2, SKIN)
        f.rect(11, top + 1 + (arm > 0), 1, 2, SKIN)
    # Legs.
    hip = top + (4 if not squash else 3)
    back, front = legs
    if airborne == "up":
        f.rect(5, hip, 2, 2, BLUE_DARK)
        f.rect(9, hip, 2, 1, BLUE_DARK)
        f.rect(4, hip + 2, 3, 1, INK)
        f.rect(10, hip + 1, 3, 1, INK)
    elif airborne == "down":
        f.rect(5, hip, 2, 3, BLUE_DARK)
        f.rect(9, hip, 2, 3, BLUE_DARK)
        f.rect(4, hip + 3, 3, 1, INK)
        f.rect(9, hip + 3, 3, 1, INK)
    else:
        legh = 15 - hip
        f.rect(5 + back, hip, 2, legh, BLUE_DARK)
        f.rect(9 + front, hip, 2, legh, BLUE_DARK)
        f.rect(5 + back - 1, 15, 3, 1, INK)
        f.rect(9 + front, 15, 3, 1, INK)
    return f


def hero_sheet():
    frames = [
        hero_frame(0),  # idle
        hero_frame(1, squash=True),
        hero_frame(0, legs=(-1, 1), arm=1),  # run
        hero_frame(-1, legs=(0, 0), arm=0),
        hero_frame(0, legs=(1, -1), arm=-1),
        hero_frame(-1, legs=(0, 0), arm=0),
        hero_frame(-1, airborne="up"),  # jump
        hero_frame(0, airborne="down"),  # fall
    ]
    sheet = Image(16 * 4, 16 * 2)
    for i, f in enumerate(frames):
        sheet.paste(f, (i % 4) * 16, (i // 4) * 16)
    return sheet


# --------------------------------------------------------------- things


def coin_sheet():
    sheet = Image(16 * 4, 16)
    for i, half in enumerate((5, 3, 1, 3)):
        f = Image(16, 16)
        for y in range(3, 13):
            dy = (y - 7.5) / 5.0
            w = half * math.sqrt(max(0.0, 1 - dy * dy))
            for x in range(16):
                dx = x - 7.5
                if abs(dx) <= w + 0.5:
                    c = GOLD
                    if abs(dx) > w - 0.6 or y in (3, 12):
                        c = GOLD_DARK
                    elif dx < -w * 0.3 and half > 1:
                        c = GOLD_LIGHT
                    f.set(x, y, c)
        if half >= 3:
            f.rect(8 - (half > 3), 6, 1, 4, GOLD_DARK)
        sheet.paste(f, i * 16, 0)
    return sheet


def coin_icon():
    """The HUD's coin: the sheet's first frame."""
    icon = Image(16, 16)
    sheet = coin_sheet()
    for y in range(16):
        for x in range(16):
            icon.set(x, y, sheet.get(x, y))
    return icon


def slime_sheet():
    sheet = Image(32, 16)
    # (half width, height): sitting tall, then squashed wide.
    for i, (w, h) in enumerate(((6.0, 9.0), (7.5, 6.5))):
        f = Image(16, 16)
        cx, base = 7.5, 15
        for y in range(16):
            for x in range(16):
                dx = (x - cx) / (w + 0.5)
                dy = (base + 0.5 - y) / h
                if y <= base and dx * dx + dy * dy <= 1.0:
                    c = SLIME
                    if y >= base - 1 or dx * dx + dy * dy > 0.8 and dx > 0:
                        c = SLIME_DARK
                    f.set(x, y, c)
        top = int(round(base + 0.5 - h))
        f.set(int(cx - w * 0.45), top + 2, WHITE)
        f.set(int(cx - w * 0.45), top + 3, WHITE)
        ey = top + int(h * 0.5)
        f.rect(5, ey, 1, 2, INK)
        f.rect(10, ey, 1, 2, INK)
        sheet.paste(f, i * 16, 0)
    return sheet


def flag_sheet():
    sheet = Image(32, 32)
    for i in range(2):
        f = Image(16, 32)
        f.rect(3, 2, 1, 30, STONE_LIGHT)
        f.rect(2, 1, 3, 2, GOLD)
        for x in range(4, 15):
            wave = int(round(math.sin((x / 11.0) * math.pi * 2 + i * math.pi) * 1.2))
            f.rect(x, 4 + wave, 1, 7, RED if (x // 3) % 2 == 0 else RED_DARK)
        f.rect(1, 30, 5, 2, STONE_DARK)
        sheet.paste(f, i * 16, 0)
    return sheet


def sky():
    w, h = 160, 90
    img = Image(w, h)
    top, mid, low = hexc("2b2d5c"), hexc("6f4f8f"), hexc("f29e6d")
    for y in range(h):
        t = y / (h - 1)
        a, b, k = (top, mid, t / 0.55) if t < 0.55 else (mid, low, (t - 0.55) / 0.45)
        c = tuple(int(a[i] + (b[i] - a[i]) * k) for i in range(3)) + (255,)
        # Dithered bands, so the pixel look carries through.
        for x in range(w):
            if (x + y) % 2 and k > 0.5:
                img.set(x, y, tuple(int(a[i] + (b[i] - a[i]) * min(1, k + 0.08)) for i in range(3)) + (255,))
            else:
                img.set(x, y, c)
    for sx, sy in ((12, 6), (40, 14), (77, 4), (101, 11), (130, 7), (150, 17), (60, 20)):
        img.set(sx, sy, WHITE)
    # A big low sun.
    for y in range(h):
        for x in range(w):
            if (x - 118) ** 2 + (y - 54) ** 2 <= 14 ** 2:
                img.set(x, y, hexc("ffd08a") if y < 52 or y % 3 else hexc("f29e6d"))
    # Clouds.
    for cx, cy, r in ((30, 30, 6), (38, 28, 8), (48, 31, 5), (88, 22, 5), (96, 20, 7)):
        for y in range(cy - r, cy + r):
            for x in range(cx - r * 2, cx + r * 2):
                if ((x - cx) / 2) ** 2 + (y - cy) ** 2 <= r * r and y <= cy + 1:
                    img.set(x, y, hexc("e8b7c8") if y > cy - 2 else hexc("f6d6df"))
    # Far and near hills.
    for x in range(w):
        far = int(58 + 6 * math.sin(x / 13.0) + 3 * math.sin(x / 5.0 + 1))
        img.rect(x, far, 1, h - far, hexc("4b3b74"))
        near = int(70 + 5 * math.sin(x / 9.0 + 2) + 2 * math.sin(x / 3.7))
        img.rect(x, near, 1, h - near, hexc("2d4a5c"))
        img.set(x, near, hexc("3f6b72"))
    return img


def main():
    os.makedirs(OUT, exist_ok=True)
    images = {
        "tiles.png": tileset(),
        "hero.png": hero_sheet(),
        "coin.png": coin_sheet(),
        "coin_icon.png": coin_icon(),
        "slime.png": slime_sheet(),
        "flag.png": flag_sheet(),
        "sky.png": sky(),
    }
    for name, img in images.items():
        img.save(os.path.join(OUT, name))
        print(f"wrote sprites/{name} ({img.w}x{img.h})")


if __name__ == "__main__":
    main()
