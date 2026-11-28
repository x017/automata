#!/usr/bin/env python3
"""Plays the platformer outside the editor.

    python examples/oberon/platformer/main.py             # in a window
    python examples/oberon/platformer/main.py --headless  # no window, 600 frames

A/D or the arrows run, Space jumps (hold it for a higher jump).
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
        at.run_scene(LEVEL, title="Pixel Peaks")


if __name__ == "__main__":
    main(sys.argv[1:])
