#!/usr/bin/env python3
"""Adds the controller-prompt symbols Geist lacks to the engine's built-in
font (``crates/automata-renderer/assets/Geist-Medium.ttf``):

- U+25A1 □  white square (PlayStation's Square button)
- U+2630 ☰  trigram for heaven (Xbox and Steam Deck's Menu button)
- U+29C9 ⧉  two joined squares (Xbox and Steam Deck's View button)

``ctx.input.prompt`` uses them, next to ×, ○ and △, which Geist has. The
outlines are plain geometry matched to Geist's ○ (stroke 89, cap height
710). Running it again changes nothing. Needs fontTools
(``pip install fonttools``). The font stays under the SIL Open Font License
(Geist has no Reserved Font Name).

    python3 scripts/add_prompt_glyphs.py [FONT.ttf]
"""

from __future__ import annotations

import sys
from pathlib import Path

from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

FONT = Path(__file__).resolve().parent.parent / "crates/automata-renderer/assets/Geist-Medium.ttf"
CAP = 710
STROKE = 89


def rect(pen: TTGlyphPen, x0: float, y0: float, x1: float, y1: float, hole: bool = False) -> None:
    """A rectangle: clockwise (filled) or, as a hole, counter-clockwise."""
    pts = [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]
    if hole:
        pts.reverse()
    pen.moveTo(pts[0])
    for p in pts[1:]:
        pen.lineTo(p)
    pen.closePath()


def frame(pen: TTGlyphPen, x0: float, y0: float, x1: float, y1: float, stroke: float) -> None:
    rect(pen, x0, y0, x1, y1)
    rect(pen, x0 + stroke, y0 + stroke, x1 - stroke, y1 - stroke, hole=True)


def square(pen: TTGlyphPen) -> int:
    frame(pen, 66, 0, 66 + CAP, CAP, STROKE)
    return 842


def trigram(pen: TTGlyphPen) -> int:
    bar = 96
    for i in range(3):
        y = 40 + i * (CAP - 80 - bar) / 2
        rect(pen, 66, round(y), 644, round(y + bar))
    return 710


def joined_squares(pen: TTGlyphPen) -> int:
    side, stroke = 470, 80
    frame(pen, 306, CAP - side, 306 + side, CAP, stroke)  # behind, up and right
    frame(pen, 66, 0, 66 + side, side, stroke)  # in front
    return 842


GLYPHS = {0x25A1: ("uni25A1", square), 0x2630: ("uni2630", trigram),
          0x29C9: ("uni29C9", joined_squares)}


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else FONT
    font = TTFont(path)
    cmap = font.getBestCmap()
    order = list(font.getGlyphOrder())
    added = []
    for cp, (name, draw) in GLYPHS.items():
        if cp in cmap:
            continue
        pen = TTGlyphPen(None)
        width = draw(pen)
        glyph = pen.glyph()
        font["glyf"][name] = glyph
        glyph.recalcBounds(font["glyf"])
        font["hmtx"][name] = (width, glyph.xMin)
        if name not in order:  # setting a glyph may add it already
            order.append(name)
        for table in font["cmap"].tables:
            if table.isUnicode():
                table.cmap[cp] = name
        added.append(f"U+{cp:04X}")
    if not added:
        print(f"{path.name} has them already")
        return 0
    font.setGlyphOrder(order)
    font["glyf"].setGlyphOrder(order)
    font.save(path)
    print(f"added {', '.join(added)} to {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
