#!/usr/bin/env python3
"""Fails unless the smoke test's screenshot shows the scene: the spike's
orange cube must cover part of the canvas (a blank or black canvas means
WebGL drew nothing).

    check_screenshot.py web/dist/smoke/screenshot.png
"""

import sys

from PIL import Image


def main() -> int:
    image = Image.open(sys.argv[1]).convert("RGB")
    pixels = list(image.getdata())
    # The cube is (0.95, 0.55, 0.20), lit: clearly red > green > blue.
    orange = sum(1 for r, g, b in pixels if r > 140 and r > g + 30 and g > b + 25)
    colours = len(set(pixels[:: max(1, len(pixels) // 20000)]))
    share = orange / len(pixels)
    print(f"orange pixels: {orange} ({share:.2%}), distinct colours (sampled): {colours}")
    if share < 0.005:
        print("no cube in the screenshot: WebGL drew nothing (or the wrong thing)")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
