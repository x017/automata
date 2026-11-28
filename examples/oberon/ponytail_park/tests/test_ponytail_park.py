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


def test_the_emotes_file_matches_make_runner():
    with open(make_runner.EMOTES, "rb") as f:
        assert f.read() == make_runner.build_emotes()


def test_emotes_come_from_their_own_file_and_play_to_the_end_then_idle():
    seen = []

    def after(ctx):
        anim = ctx.scene["Runner"].animation
        if not seen:
            # The model's clips plus the three borrowed from emotes.glb.
            assert anim.clips == ["Idle", "Walk", "Run", "emotes:Wave", "emotes:Cheer",
                                  "emotes:Leap"]
            assert anim.length("emotes:Wave") == pytest.approx(2.0, abs=0.04)
            anim.play("emotes:Wave", loop=False)
        seen.append((anim.clip, anim.playing))

    at.run_scene(LEVEL, headless=True, max_frames=150, update=after, fixed_delta=1 / 60)
    # The player script lets the emote finish (2 s), then goes back to Idle.
    assert seen[10] == seen[110] == ("emotes:Wave", True)
    assert seen[-1] == ("Idle", True)
    assert set(player.EMOTES.values()) == {"emotes:Wave", "emotes:Cheer", "emotes:Leap"}


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
        assert clips == {"Idle", "Walk", "Run", "emotes:Wave", "emotes:Cheer", "emotes:Leap"}
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


def test_root_motion_carries_her_through_the_leap():
    seen = []

    def after(ctx):
        runner = ctx.scene["Runner"]
        anim = runner.animation
        if len(seen) == 5:
            assert anim.root_motion
            anim.play("emotes:Leap", loop=False)
        seen.append((runner.transform.position, anim.clip))

    at.run_scene(LEVEL, headless=True, max_frames=120, update=after, fixed_delta=1 / 60)
    (x0, y0, z0), _ = seen[5]
    (x1, y1, z1), _ = seen[-1]
    # She faces -Z (yaw 180), so the 2.4 m forward is towards -Z.
    assert z0 - z1 == pytest.approx(make_runner.LEAP, abs=0.05)
    assert x1 == pytest.approx(x0, abs=1e-3) and y1 == pytest.approx(y0, abs=1e-6)
    # Back to Idle after landing, where she landed.
    assert seen[-1][1] == "Idle"


LISTENER = """
def start(entity, ctx):
    ctx.subscribe("Jogger:animation:footstep", step)
    ctx.subscribe("*:animation:land", landed)


def step(msg):
    msg.ctx.scene["Ear"].attributes["steps"] += 1


def landed(msg):
    msg.ctx.scene["Ear"].attributes["heard"] = msg.channel + " " + msg.clip
"""


def test_scripts_hear_animation_events_on_channels(tmp_path):
    import shutil

    # Scripts and models are found next to the level.
    for folder in ("scripts", "models"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    (tmp_path / "listener.py").write_text(LISTENER)
    scene = build_level.build()
    ear = at.Entity("Ear").add(at.Transform())
    ear.attributes["steps"] = 0
    ear.attributes["heard"] = ""
    ear.script = str(tmp_path / "listener.py")
    scene.add(ear)
    level = tmp_path / "level.ron"
    scene.save(str(level))
    seen = []

    def after(ctx):
        if len(seen) == 10:
            ctx.scene["Runner"].animation.play("emotes:Leap", loop=False)
        jog = ctx.scene["Jogger"].animation
        seen.append((dict(ctx.scene["Ear"].attributes.to_dict()), jog.clip,
                     [name for _, _, name in jog.recent_events]))

    at.run_scene(str(level), headless=True, max_frames=170, update=after, fixed_delta=1 / 60)
    attrs, clip, events = seen[-1]
    assert clip == "Run" and "footstep" in events
    # Run is 0.72 s with two footsteps; she starts running after 1 s.
    assert 4 <= attrs["steps"] <= 7, attrs
    assert attrs["heard"] == "Runner:animation:land emotes:Leap"


KEYS_SCRIPT = """
def start(entity, ctx):
    entity.state["jog"] = ctx.ref("Jogger:animation:clip")


def update(entity, ctx):
    if ctx.time.frame == 30:
        ctx.set("Stroller:animation:speed", 2.0)
        for anim in ctx.scan("Spect*:animation"):
            anim.play("emotes:Wave", loop=False)
        ear = ctx.get("Ear:attr")
        ear["runners"] = len(ctx.scan("*:animation"))
        ear["jog"] = entity.state["jog"].value
"""


def test_scripts_reach_other_entities_by_key(tmp_path):
    import shutil

    for folder in ("scripts", "models"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    (tmp_path / "keys.py").write_text(KEYS_SCRIPT)
    scene = build_level.build()
    ear = at.Entity("Ear").add(at.Transform())
    ear.attributes["runners"] = 0
    ear.attributes["jog"] = ""
    ear.script = str(tmp_path / "keys.py")
    scene.add(ear)
    level = tmp_path / "level.ron"
    scene.save(str(level))
    seen = {}

    def after(ctx):
        seen["stroller"] = ctx.scene["Stroller"].animation.speed
        seen["spectator"] = ctx.scene["Spectator"].animation.clip
        seen["ear"] = ctx.scene["Ear"].attributes.to_dict()

    at.run_scene(str(level), headless=True, max_frames=40, update=after, fixed_delta=1 / 60)
    assert seen["stroller"] == 2.0
    assert seen["spectator"] == "emotes:Wave"
    assert seen["ear"] == {"runners": 4, "jog": "Idle"}


def test_entity_names_cannot_hold_colons():
    with pytest.raises(ValueError, match="separates the parts of a key"):
        at.Entity("Player:1")
    e = at.Entity("Player")
    with pytest.raises(ValueError):
        e.name = "a:b"
