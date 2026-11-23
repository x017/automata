"""Tests for Robot Plaza, the animation example.

    pytest examples/oberon/robot_plaza/tests
"""

import math
import os
import shutil

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
player = load_script(os.path.join(GAME, "scripts", "player.py"))


def test_level_matches_build_level():
    with open(LEVEL) as f:
        assert f.read() == build_level.build().to_ron()


def test_level_runs_and_every_robot_animates():
    seen = {}

    def after(ctx):
        for name in ("Robot", "Greeter", "Dancer", "Sitter", "Slowpoke"):
            anim = ctx.scene[name].animation
            seen[name] = (anim.clip, anim.playing, anim.time, anim.speed)

    frames = at.run_scene(LEVEL, headless=True, max_frames=90, update=after, fixed_delta=1 / 60)
    assert frames == 90
    assert seen["Robot"][:2] == ("Idle", True)
    assert seen["Dancer"][:2] == ("Dance", True)
    # Sitting is a 0.42 s one-shot: by 1.5 s it's over and holds its last frame.
    assert seen["Sitter"][0] == "Sitting" and not seen["Sitter"][1]
    assert seen["Sitter"][2] == pytest.approx(0.417, abs=0.01)
    assert seen["Slowpoke"][3] == pytest.approx(0.35)


def test_the_player_stands_on_the_plaza():
    heights = []
    at.run_scene(
        LEVEL,
        headless=True,
        max_frames=60,
        update=lambda ctx: heights.append(ctx.scene["Robot"].transform.position[1]),
    )
    assert heights[-1] == pytest.approx(build_level.STAND, abs=0.15)


def test_the_greeter_waves_when_the_player_walks_up(tmp_path):
    shutil.copytree(os.path.join(GAME, "scripts"), tmp_path / "scripts")
    shutil.copytree(os.path.join(GAME, "models"), tmp_path / "models")
    original = list(build_level.CROWD)
    # Put the Greeter right next to where the player starts.
    build_level.CROWD[0] = ("Greeter", (1.5, 0.0, 3.0), -90, at.Animation("Idle"))
    try:
        build_level.build().save(str(tmp_path / "level.ron"))
    finally:
        build_level.CROWD[:] = original
    clips = []

    def after(ctx):
        greeter = ctx.scene["Greeter"]
        clips.append(greeter.animation.clip)
        after.waves = greeter.attributes.get("waves", 0)

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=180, update=after,
                 fixed_delta=1 / 60)
    assert "Wave" in clips, "waves on the first frame the player is near"
    assert clips[-1] == "Idle", "and goes back to Idle when the wave ends"
    assert after.waves == 1, "only once per visit"


@pytest.mark.parametrize(
    ("grounded", "speed", "current", "playing", "wave", "expected"),
    [
        (True, 0.0, "Idle", True, False, ("Idle", True)),
        (True, 2.2, "Idle", True, False, ("Walking", True)),
        (True, 5.0, "Walking", True, False, ("Running", True)),
        (False, 2.2, "Walking", True, False, ("Jump", False)),
        (True, 0.0, "Idle", True, True, ("Wave", False)),
        (True, 0.0, "Wave", True, False, ("Wave", False)),
        (True, 0.0, "Wave", False, False, ("Idle", True)),
        (True, 2.2, "Wave", True, False, ("Walking", True)),
    ],
)
def test_choose_clip(grounded, speed, current, playing, wave, expected):
    assert player.choose_clip(grounded, speed, current, playing, wave) == expected


def test_turn_towards_takes_the_short_way_round():
    yaw = player.turn_towards(math.radians(170), math.radians(-170), 1.0, 0.5)
    assert math.degrees(yaw) == pytest.approx(180.0)
    assert player.turn_towards(0.0, 1.0, 100.0, 1.0) == pytest.approx(1.0)
