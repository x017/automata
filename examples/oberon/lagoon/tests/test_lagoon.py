"""Tests for the Lagoon example.

    pip install pytest
    pytest examples/oberon/lagoon/tests
"""

import os
import re

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
main = load_script(os.path.join(GAME, "main.py"))
FLOATING = [entry[0] for entry in build_level.FLOATING]
NUMBER = r"-?\d+\.\d+(?:[eE][-+]?\d+)?"


def same_scene_text(a, b):
    """Scene files equal up to float noise in the last digit."""
    if re.split(NUMBER, a) != re.split(NUMBER, b):
        return False
    xs, ys = re.findall(NUMBER, a), re.findall(NUMBER, b)
    return all(abs(float(x) - float(y)) <= 1e-5 for x, y in zip(xs, ys))


def test_level_file_is_up_to_date():
    with open(LEVEL, encoding="utf-8") as f:
        saved = f.read()
    assert same_scene_text(build_level.build().to_ron(), saved), (
        "level.ron is stale: run examples/oberon/lagoon/build_level.py"
    )


@pytest.fixture(scope="module")
def played():
    """Ten seconds of the real scene, headless: every floating thing's
    position each frame, plus the water height beside the pier."""
    frames = []

    def record(ctx):
        frames.append(
            (
                {name: ctx.scene[name].transform.position for name in FLOATING},
                ctx.physics.water_height(-8.5, 24.0),
                ctx.physics.water_height(500.0, 0.0),
            )
        )

    count = at.run_scene(LEVEL, headless=True, max_frames=600, update=record)
    assert count == 600
    return frames


def test_floating_things_ride_the_swell(played):
    settled = played[300:]
    for name in FLOATING:
        ys = [frame[0][name][1] for frame in settled]
        # Afloat: neither sunk to the seabed nor thrown into the air.
        assert -1.0 < min(ys) and max(ys) < 1.0, (name, min(ys), max(ys))
        # Riding the waves, not sitting still.
        assert max(ys) - min(ys) > 0.1, (name, min(ys), max(ys))


def test_floating_things_stay_roughly_where_they_were_put(played):
    first, last = played[0][0], played[-1][0]
    for name in FLOATING:
        dx = last[name][0] - first[name][0]
        dz = last[name][2] - first[name][2]
        assert dx * dx + dz * dz < 2.0**2, (name, dx, dz)


def test_scripts_see_the_moving_surface(played):
    heights = [frame[1] for frame in played]
    assert all(h is not None for h in heights)
    assert max(heights) - min(heights) > 0.1
    assert all(frame[2] is None for frame in played)


def test_quality_cycle_wraps():
    assert main.next_quality("low") == "medium"
    assert main.next_quality("beautiful") == main.QUALITIES[0]


def test_headless_entry_point_runs_on_every_preset(capsys):
    for quality in ("low", "beautiful"):
        main.main(["--headless", "--graphics", quality])
    assert "ran 300 frames" in capsys.readouterr().out
