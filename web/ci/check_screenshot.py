#!/usr/bin/env python3
"""Fails unless a web test's screenshot shows a scene.

By default it looks for the spike's orange cube, which must cover part of
the canvas (a blank or black canvas means WebGL drew nothing). With
``--any`` (games such as Coin Run) the picture only has to be varied: many
colours, and no single colour over most of it.

    check_screenshot.py web/dist/smoke/screenshot.png
    check_screenshot.py --any web/dist/coin_run/1-playing.png
"""

import sys
from collections import Counter

from PIL import Image


def main() -> int:
    args = sys.argv[1:]
    anything = "--any" in args
    path = [a for a in args if a != "--any"][0]
    image = Image.open(path).convert("RGB")
    pixels = list(image.getdata())
    sample = pixels[:: max(1, len(pixels) // 20000)]
    colours = len(set(sample))
    if anything:
        top = Counter(sample).most_common(1)[0][1] / len(sample)
        print(f"distinct colours (sampled): {colours}, most common colour: {top:.0%}")
        if colours < 64 or top > 0.85:
            print("the screenshot is (nearly) one colour: WebGL drew nothing")
            return 1
        return 0
    # The cube is (0.95, 0.55, 0.20), lit: clearly red > green > blue.
    orange = sum(1 for r, g, b in pixels if r > 140 and r > g + 30 and g > b + 25)
    share = orange / len(pixels)
    print(f"orange pixels: {orange} ({share:.2%}), distinct colours (sampled): {colours}")
    if share < 0.005:
        print("no cube in the screenshot: WebGL drew nothing (or the wrong thing)")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
