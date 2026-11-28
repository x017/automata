#!/usr/bin/env python3
"""Plays the Lagoon outside the editor.

    python examples/oberon/lagoon/main.py                     # Beautiful
    python examples/oberon/lagoon/main.py --graphics high     # any preset
    python examples/oberon/lagoon/main.py --headless          # no window, 300 frames

Drag with the mouse (or A / D, Q / E, arrow keys) to orbit, W / S to zoom.
G cycles the graphics quality live, so you can compare the presets on the
same view. Esc quits.
"""

import argparse
import os
import sys

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
LEVEL = os.path.join(HERE, "level.ron")
QUALITIES = [q.value for q in at.GraphicsQuality]


def next_quality(current):
    """The preset after `current`, wrapping round to the first."""
    return QUALITIES[(QUALITIES.index(current) + 1) % len(QUALITIES)]


def controls(ctx):
    """Runs after the scene's scripts every frame: quality switch and HUD."""
    if ctx.input.just_pressed(at.Key.G):
        ctx.screen.set_graphics(next_quality(ctx.screen.graphics))
    if ctx.input.just_pressed(at.Key.ESCAPE):
        ctx.quit()
    ctx.screen.text(
        f"Graphics: {ctx.screen.graphics.title()}   (G to change)\n"
        "Drag to orbit · W / S zoom",
        16, 36, size=20, shadow=(0.0, 0.0, 0.0, 0.6),
    )


def main(argv):
    parser = argparse.ArgumentParser(description="The Lagoon water showcase.")
    parser.add_argument("--graphics", choices=QUALITIES, default="beautiful")
    parser.add_argument("--headless", action="store_true")
    args = parser.parse_args(argv)
    if args.headless:
        frames = at.run_scene(LEVEL, headless=True, max_frames=300, graphics=args.graphics,
                              update=controls)
        print(f"ran {frames} frames of level.ron")
    else:
        at.run_scene(LEVEL, title="Lagoon", graphics=args.graphics, update=controls)


if __name__ == "__main__":
    main(sys.argv[1:])
