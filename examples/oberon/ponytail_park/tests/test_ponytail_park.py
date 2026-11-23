"""Tests for Ponytail Park, the spring bones example.

    pytest examples/oberon/ponytail_park/tests

The swinging itself is tested in Rust, on this same model
(crates/automata-scene/tests/spring_runner.rs). These check the level, the
model file and the scripts.
"""

import math
import os

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
make_runner = load_script(os.path.join(GAME, "make_runner.py"))
jogger = load_script(os.path.join(GAME, "scripts", "jogger.py"))
player = load_script(os.path.join(GAME, "scripts", "player.py"))
RUNNERS = ("Runner", "Jogger", "Stroller", "Spectator")


def test_level_matches_build_level():
    with open(LEVEL) as f:
        assert f.read() == build_level.build().to_ron()


def test_the_model_file_matches_make_runner():
    with open(make_runner.OUT, "rb") as f:
        assert f.read() == make_runner.build()


def test_the_runner_has_spring_bones_and_every_copy_animates():
    seen = {}

    def after(ctx):
        for name in RUNNERS:
            anim = ctx.scene[name].animation
            seen[name] = (anim.clip, anim.playing, anim.springs, set(anim.clips))

    at.run_scene(LEVEL, headless=True, max_frames=30, update=after, fixed_delta=1 / 60)
    for name in RUNNERS:
        clip, playing, springs, clips = seen[name]
        assert playing, name
        assert springs, f"{name}'s model has spring bones"
        assert clips == {"Idle", "Walk", "Run"}
    assert seen["Stroller"][0] == "Walk"
    assert seen["Spectator"][0] == "Idle"


def test_the_jogger_runs_the_track_then_stops():
    track = []

    def after(ctx):
        jog = ctx.scene["Jogger"]
        track.append((jog.transform.position[0], jog.animation.clip))

    at.run_scene(LEVEL, headless=True, max_frames=270, update=after, fixed_delta=1 / 60)
    x0 = build_level.JOG_START[0]
    # Standing for the first second, then running up the track...
    assert track[30] == (pytest.approx(x0), "Idle")
    assert track[120][1] == "Run"
    assert track[120][0] > x0 + 2.0
    # ...and stopped at the far end by 3.5 s.
    x_end, clip = track[-1]
    assert clip == "Idle"
    assert x_end == pytest.approx(x0 + jogger.LAP, abs=1e-3)


def test_a_jog_is_a_lap_one_way_a_rest_then_a_lap_back():
    state = {"leg": "rest", "left": 0.5, "heading": -1.0}
    x, t, legs = 0.0, 0.0, []
    while t < 2 * (jogger.LAP / jogger.RUN + jogger.PAUSE) + 0.5:
        before = state["leg"]
        x += jogger.next_leg(state, 1 / 60)
        t += 1 / 60
        if state["leg"] != before:
            legs.append((state["leg"], round(x, 3)))
    assert legs[:4] == [
        ("run", 0.0),
        ("rest", jogger.LAP),
        ("run", jogger.LAP),
        ("rest", 0.0),
    ]


def test_the_stroller_stays_on_her_circle():
    from_centre = []

    def after(ctx):
        x, _, z = ctx.scene["Stroller"].transform.position
        bx, bz = build_level.BED
        from_centre.append(math.hypot(x - bx, z - bz))

    at.run_scene(LEVEL, headless=True, max_frames=120, update=after, fixed_delta=1 / 60)
    assert min(from_centre) == pytest.approx(build_level.STROLL_RADIUS, abs=1e-3)
    assert max(from_centre) == pytest.approx(build_level.STROLL_RADIUS, abs=1e-3)


@pytest.mark.parametrize(
    "speed, clip", [(0.0, "Idle"), (player.WALK, "Walk"), (player.RUN, "Run")]
)
def test_the_player_picks_the_clip_for_her_speed(speed, clip):
    assert player.clip_for(speed) == clip


def test_scripts_can_put_the_springs_back_at_rest():
    calls = []

    def after(ctx):
        anim = ctx.scene["Runner"].animation
        anim.reset_springs()
        calls.append(anim.springs)

    frames = at.run_scene(LEVEL, headless=True, max_frames=10, update=after, fixed_delta=1 / 60)
    assert frames == 10 and all(calls)
