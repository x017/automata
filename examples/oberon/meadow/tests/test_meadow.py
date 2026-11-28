"""Tests for Meadow, the grass and foliage example.

    pytest examples/oberon/meadow/tests

How grass sways and bends is tested in Rust (crates/automata-renderer's
grass tests and crates/automata-physics/src/grass.rs). These check the
level, the painted foliage and the scripts.
"""

import math
import os

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
player = load_script(os.path.join(GAME, "scripts", "player.py"))
make_foliage = load_script(os.path.join(GAME, "make_foliage.py"))


def component(scene, name, kind):
    entity = scene.find(name)
    assert entity is not None, name
    return next(c for c in entity.components() if isinstance(c, kind))


def test_meadow_level_matches_build_level():
    with open(LEVEL) as f:
        assert f.read() == build_level.build().to_ron()


def test_the_foliage_models_match_make_foliage():
    for name, data in make_foliage.build().items():
        with open(os.path.join(GAME, "models", name), "rb") as f:
            assert f.read() == data, f"models/{name} is stale: run make_foliage.py"


def test_the_path_is_cut_out_of_the_meadow_and_patches_painted_past_it():
    painted = component(build_level.build(), "Meadow", at.Grass)
    square = at.Grass(area=painted.area, density=painted.density, seed=painted.seed)
    steps = len(list(build_level.path_points()))
    assert painted.strokes == steps + 3
    # Erasing the path takes blades away; the patches then add some back.
    path_only = square
    for x, z in build_level.path_points():
        path_only = path_only.erase((x, 0.0, z), radius=1.1)
    assert path_only.blade_count() < square.blade_count() * 0.97
    assert painted.blade_count() > path_only.blade_count()


def test_flowers_rocks_and_trees_are_scattered():
    scene = build_level.build()
    assert component(scene, "Rocks", at.Scatter).count() > 10
    assert component(scene, "Trees", at.Scatter).count() > 20
    for name in ("Poppies", "Daisies"):
        assert component(scene, name, at.Scatter).count() > 20, name
    assert component(scene, "Wildflowers", at.Grass).blade_count() > 1_000


def test_the_wind_blows():
    env = component(at.Scene.load(LEVEL), "Environment", at.Environment)
    assert env.wind == pytest.approx(build_level.WIND)
    assert env.wind_direction == pytest.approx(build_level.WIND_DIRECTION)


def test_the_runner_walks_through_the_grass_and_kicks_the_ball():
    """Drives her (after her script) toward the ball, as W would."""
    seen = []
    bx, _, bz = (2.0, 0.35, 4.0)
    sx, _, sz = build_level.START
    length = math.hypot(bx - sx, bz - sz)
    toward = ((bx - sx) / length * player.WALK, -2.0, (bz - sz) / length * player.WALK)

    def after(ctx):
        runner, ball = ctx.scene["Runner"], ctx.scene["Ball"]
        runner.character_velocity = toward
        seen.append((runner.transform.position, ball.transform.position))

    at.run_scene(LEVEL, headless=True, max_frames=240, update=after, fixed_delta=1 / 60)
    (start, ball_start), (end, ball_end) = seen[0], seen[-1]
    assert abs(end[1]) < 0.15, f"she stays on the ground: {end}"
    assert math.dist(start, end) > 2.0, "she walked"
    assert math.dist(ball_start, ball_end) > 0.5, "the ball was kicked"


@pytest.mark.parametrize(("speed", "clip"), [(0.0, "Idle"), (1.4, "Walk"), (4.2, "Run")])
def test_the_player_picks_the_clip_for_her_speed(speed, clip):
    assert player.clip_for(speed) == clip


def test_the_level_runs_on_every_preset():
    for quality in ("low", "high"):
        frames = at.run_scene(LEVEL, headless=True, max_frames=5, graphics=quality)
        assert frames == 5


def test_the_runner_is_ponytail_parks():
    """The model is a copy; regenerate it there and copy it here."""
    theirs = os.path.join(GAME, "..", "ponytail_park", "models", "runner.glb")
    with open(os.path.join(GAME, "models", "runner.glb"), "rb") as a, open(theirs, "rb") as b:
        assert a.read() == b.read()
