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
        for name in ("Robot", "Greeter", "Dancer", "Sitter", "Slowpoke", "Wobbly"):
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


def test_the_hat_stays_on_the_robots_head():
    seen = []

    def after(ctx):
        robot, brim = ctx.scene["Robot"], ctx.scene["Hat Brim"]
        seen.append((robot.transform.position, brim.transform.position, brim.attached_to))

    at.run_scene(LEVEL, headless=True, max_frames=30, update=after, fixed_delta=1 / 60)
    (rx, ry, rz), (hx, hy, hz), attached = seen[-1]
    assert attached == ("Robot", "Head")
    assert math.hypot(hx - rx, hz - rz) < 0.3, "above the robot"
    assert 0.5 < hy - ry < 1.2, "at head height (the robot is 1.9 m tall at scale 0.4)"


def test_feet_follow_the_ground_on_the_player_and_the_stepper():
    seen = {}

    def after(ctx):
        for name in ("Robot", "Stepper", "Dancer"):
            seen[name] = ctx.scene[name].animation.foot_ik

    at.run_scene(LEVEL, headless=True, max_frames=3, update=after)
    assert seen == {"Robot": True, "Stepper": True, "Dancer": False}


def test_the_greeter_watches_the_player_when_near(tmp_path):
    for folder in ("models", "anims", "scripts"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    original = list(build_level.CROWD)
    # 5 m away: close enough to watch, too far to wave.
    build_level.CROWD[0] = ("Greeter", (5.0, 0.0, 3.0), -90, at.Animation("Idle"))
    try:
        build_level.build().save(str(tmp_path / "level.ron"))
    finally:
        build_level.CROWD[:] = original
    looking = []
    at.run_scene(
        str(tmp_path / "level.ron"),
        headless=True,
        max_frames=20,
        update=lambda ctx: looking.append(ctx.scene["Greeter"].animation.looking),
    )
    assert not looking[0] and looking[-1], "looks at the player"


def test_a_script_picks_up_and_drops_a_hat(tmp_path):
    for folder in ("models", "anims", "scripts"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    (tmp_path / "scripts" / "greeter.py").write_text(
        "def update(entity, ctx):\n"
        "    hat, f = ctx.scene.find('Hat Crown'), ctx.time.frame\n"
        "    if f == 2:\n"
        "        hat.attach('Greeter', 'Palm2.R', at.Transform(position=(0, 0.1, 0)))\n"
        "    if f == 6:\n"
        "        entity.attributes['held'] = ','.join(hat.attached_to)\n"
        "        hat.detach()\n"
        "    if f == 10:\n"
        "        entity.attributes['after'] = str(hat.attached_to)\n"
        "        entity.attributes['y'] = hat.transform.position[1]\n"
        "    if f == 14:\n"
        "        entity.attributes['still'] = hat.transform.position[1]\n"
        "import automata as at\n"
    )
    build_level.build().save(str(tmp_path / "level.ron"))
    seen = {}
    at.run_scene(
        str(tmp_path / "level.ron"),
        headless=True,
        max_frames=16,
        fixed_delta=1 / 60,
        update=lambda ctx: seen.update(ctx.scene["Greeter"].attributes),
    )
    assert seen["held"] == "Greeter,Palm2.R"
    assert seen["after"] == "None", "dropped"
    assert seen["still"] == pytest.approx(seen["y"]), "and it stays where it was let go"


def test_wobbly_falls_over_when_bumped_and_gets_back_up(tmp_path):
    for folder in ("models", "anims", "scripts"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    original = list(build_level.CROWD)
    # Put Wobbly right next to where the player starts: an instant bump.
    build_level.CROWD[-1] = ("Wobbly", (0.7, 0.0, 3.0), -90, at.Animation("Idle"))
    try:
        build_level.build().save(str(tmp_path / "level.ron"))
    finally:
        build_level.CROWD[:] = original
    seen = {"limp": 0, "lowest": 9.0}

    def after(ctx):
        wobbly = ctx.scene["Wobbly"]
        state = wobbly.ragdoll_state
        if state["limp"]:
            seen["limp"] += 1
            seen["lowest"] = min(seen["lowest"], state["hips"][1])
        seen["state"] = state
        seen["falls"] = wobbly.attributes.get("falls", 0)
        seen["at"] = wobbly.transform.position

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=480, update=after,
                 fixed_delta=1 / 60)
    assert seen["falls"] >= 1, "the bump knocked it over"
    assert seen["limp"] > 60 and seen["lowest"] < 0.5, "it lay on the ground"
    assert not seen["state"]["limp"], "and got back up"
    x, y, _ = seen["at"]
    assert x > 0.9, "away from the player, who bumped it along +x"
    assert y == pytest.approx(build_level.STAND, abs=0.05)
