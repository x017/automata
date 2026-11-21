#!/usr/bin/env python3
"""Plays the FPS example outside the editor.

    python examples/oberon/fps/main.py             # in a window
    python examples/oberon/fps/main.py --headless  # no window, 600 frames

Mouse to look (Esc frees the cursor, click to take it back), WASD to walk,
Space to jump (twice), left mouse button to shoot, E to switch blaster.
"""

import os
import sys

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
LEVEL = os.path.join(HERE, "level.ron")


def main(argv):
    if "--headless" in argv:
        frames = at.run_scene(LEVEL, headless=True, max_frames=600)
        print(f"ran {frames} frames of level.ron")
    else:
        at.run_scene(LEVEL, title="Blaster Arena")


if __name__ == "__main__":
    main(sys.argv[1:])
