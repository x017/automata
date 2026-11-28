#!/usr/bin/env python3
"""Plays Coin Run outside the editor (tutorial: From Oberon to Python).

    python examples/oberon/coin_run/main.py            # the afternoon level
    python examples/oberon/coin_run/main.py dusk       # or dusk / night
    python examples/oberon/coin_run/main.py --headless # no window, 600 frames

WASD or the arrow keys walk, Space jumps (and skips the intro), Shift
sprints, and Q / E orbit the camera.
"""

import os
import sys

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
LEVELS = {"afternoon": "level.ron", "dusk": "level_dusk.ron", "night": "level_night.ron"}


def main(argv):
    headless = "--headless" in argv
    moods = [a for a in argv if not a.startswith("-")]
    mood = moods[0] if moods else "afternoon"
    if mood not in LEVELS:
        sys.exit(f"unknown level {mood!r}; choose one of: {', '.join(LEVELS)}")
    level = os.path.join(HERE, LEVELS[mood])
    if headless:
        frames = at.run_scene(level, headless=True, max_frames=600)
        print(f"ran {frames} frames of {LEVELS[mood]}")
    else:
        at.run_scene(level, title="Coin Run")


if __name__ == "__main__":
    main(sys.argv[1:])
