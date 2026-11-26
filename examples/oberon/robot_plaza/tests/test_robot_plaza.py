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
    assert seen["Robot"][:2] == ("Idle", True), "the graph starts standing still"
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
    for folder in ("models", "anims", "scripts"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
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


class FakeAnimation:
    """Records what a script asks of ``entity.animation``."""

    def __init__(self):
        self.params, self.triggers = {}, []

    def set(self, name, value):
        self.params[name] = value

    def trigger(self, name):
        self.triggers.append(name)


@pytest.mark.parametrize(
    ("speed", "grounded", "jumped", "waved", "triggers"),
    [
        (0.0, True, False, False, []),
        (5.0, True, False, True, ["wave"]),
        (2.2, False, True, False, ["jump"]),
        (2.2, False, True, True, ["jump", "wave"]),
    ],
)
def test_the_player_only_sets_parameters(speed, grounded, jumped, waved, triggers):
    anim = FakeAnimation()
    player.animate(anim, speed, grounded, jumped, waved)
    assert anim.params == {"speed": speed, "grounded": grounded}
    assert anim.triggers == triggers


def test_the_robot_graph_walks_jumps_and_waves(tmp_path):
    """The real graph, driven the way scripts/player.py drives it."""
    for folder in ("models", "anims", "scripts"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    (tmp_path / "scripts" / "player.py").write_text(
        "def update(entity, ctx):\n"
        "    anim, f, seen = entity.animation, ctx.time.frame, entity.attributes\n"
        "    if f == 2:\n"
        "        seen['start'] = ','.join([anim.state, anim.clip, *anim.layers])\n"
        "        anim.set('speed', 5.0)\n"
        "    if f == 30:\n"
        "        seen['running'] = anim.clip\n"
        "        anim.trigger('wave')\n"
        "    if f == 40:\n"
        "        seen['wave'] = anim.state + ',' + anim.layer_state('Arms')\n"
        "        anim.set('grounded', False)\n"
        "        anim.trigger('jump')\n"
        "    if f == 45:\n"
        "        seen['jump'] = anim.state\n"
        "        anim.set('grounded', True)\n"
        "    if f == 150:\n"
        "        seen['landed'] = anim.state + ',' + anim.layer_state('Arms')\n"
    )
    build_level.build().save(str(tmp_path / "level.ron"))
    seen = {}
    at.run_scene(
        str(tmp_path / "level.ron"),
        headless=True,
        max_frames=152,
        fixed_delta=1 / 60,
        update=lambda ctx: seen.update(ctx.scene["Robot"].attributes),
    )
    assert seen["start"] == "Move,Idle,Base,Arms"
    assert seen["running"] == "Running", "speed 5 is all Running"
    assert seen["wave"] == "Move,Wave", "the arms wave; the legs keep running"
    assert seen["jump"] == "Jump"
    assert seen["landed"] == "Move,Off", "back on its feet, the wave over"


def test_turn_towards_takes_the_short_way_round():
    yaw = player.turn_towards(math.radians(170), math.radians(-170), 1.0, 0.5)
    assert math.degrees(yaw) == pytest.approx(180.0)
    assert player.turn_towards(0.0, 1.0, 100.0, 1.0) == pytest.approx(1.0)
