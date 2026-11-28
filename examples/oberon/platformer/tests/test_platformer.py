"""Tests for the 2D platformer example.

    pip install pytest
    pytest examples/oberon/platformer/tests

They play the real scene headlessly and move the player around from the
``update`` callback (as if it had run and jumped there), which is how to
test pick-ups and enemies without a keyboard.
"""

import math
import os
import re

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
build_art = load_script(os.path.join(GAME, "build_art.py"))
player_script = load_script(os.path.join(GAME, "scripts", "player.py"))

NUMBER = r"-?\d+\.\d+(?:[eE][-+]?\d+)?"
SPAWN = (2.5, 2.5)  # the "@" cell's centre


def same_scene_text(a, b):
    """Scene files equal up to float noise in the last digit."""
    if re.split(NUMBER, a) != re.split(NUMBER, b):
        return False
    xs, ys = re.findall(NUMBER, a), re.findall(NUMBER, b)
    return all(abs(float(x) - float(y)) <= 1e-5 for x, y in zip(xs, ys))


def play(frames, at_frame=None):
    """Runs the level for ``frames`` frames; ``at_frame(n, ctx)`` may move
    things. Returns what the player, camera and sky did each frame."""
    seen = []

    def after(ctx):
        if at_frame is not None:
            at_frame(ctx.time.frame, ctx)
        p = ctx.scene["Player"]
        seen.append(
            {
                "player": p.transform.position,
                "attrs": dict(p.attributes),
                "sprite": p.sprite_state,
                "camera": ctx.scene["Camera"].transform.position,
                "sky": ctx.scene["Sky"].transform.position,
                "names": {e.name for e in ctx.scene},
                "slime": ctx.scene.find("Slime 1") and ctx.scene["Slime 1"].transform.position,
            }
        )

    at.run_scene(LEVEL, headless=True, max_frames=frames, fixed_delta=1 / 60, update=after)
    return seen


def teleport(frame, name, offset=(0.0, 0.0)):
    """At ``frame``, puts the player on the entity ``name`` (plus ``offset``)."""

    def at_frame(n, ctx):
        if n == frame:
            x, y, _ = ctx.scene[name].transform.position
            ctx.scene["Player"].transform.position = (x + offset[0], y + offset[1], 0.0)

    return at_frame


def test_the_saved_level_matches_build_level(tmp_path):
    path = tmp_path / "fresh.ron"
    build_level.build().save(str(path))
    with open(LEVEL, encoding="utf-8") as saved:
        assert same_scene_text(path.read_text(encoding="utf-8"), saved.read()), (
            "level.ron is stale: run build_level.py"
        )


def test_the_art_matches_build_art(tmp_path):
    for name, draw in [
        ("tiles.png", build_art.tileset),
        ("hero.png", build_art.hero_sheet),
        ("coin.png", build_art.coin_sheet),
        ("slime.png", build_art.slime_sheet),
        ("flag.png", build_art.flag_sheet),
        ("sky.png", build_art.sky),
    ]:
        fresh = tmp_path / name
        draw().save(str(fresh))
        with open(os.path.join(GAME, "sprites", name), "rb") as f:
            assert fresh.read_bytes() == f.read(), f"sprites/{name} is stale: run build_art.py"


def test_every_map_row_is_as_wide_and_things_stand_on_something():
    width = len(build_level.MAP[0])
    assert all(len(row) == width for row in build_level.MAP)
    solid = set(build_level.SOLID)
    for r, row in enumerate(build_level.MAP[:-1]):
        for c, ch in enumerate(row):
            if ch in "bfg>s@":
                assert build_level.MAP[r + 1][c] in solid, f"{ch!r} at column {c} floats"


def test_the_player_stands_on_the_ground_idle_and_the_camera_stays_in_the_level():
    seen = play(60)
    last = seen[-1]
    x, y, _ = last["player"]
    assert (x, y) == pytest.approx(SPAWN, abs=0.1)
    assert last["sprite"]["clip"] == "idle"
    assert last["attrs"]["coins"] == 0
    cx, cy, _ = last["camera"]
    # The view is 10 cells tall and 16:9: held inside the level's bounds.
    assert cx == pytest.approx(5.0 * 16 / 9, abs=0.01) and cy == pytest.approx(5.0, abs=0.01)
    # The sky keeps most of the camera's movement.
    sx, _, _ = last["sky"]
    assert sx == pytest.approx(32.0 + (cx - 32.0) * 0.85, abs=0.01)


def test_touching_a_coin_collects_it():
    seen = play(15, teleport(10, "Coin 1"))
    assert seen[-1]["attrs"]["coins"] == 1
    assert "Coin 1" not in seen[-1]["names"] and "Coin 2" in seen[-1]["names"]


def test_walking_into_a_slime_sends_the_player_back_to_the_start():
    seen = play(15, teleport(10, "Slime 1", (0.4, 0.0)))
    x, y, _ = seen[-1]["player"]
    assert (x, y) == pytest.approx(SPAWN, abs=0.3)
    assert seen[-1]["attrs"]["deaths"] == 1
    assert "Slime 1" in seen[-1]["names"]


def test_landing_on_a_slime_squashes_it_and_bounces():
    seen = play(24, teleport(10, "Slime 1", (0.0, 0.6)))
    assert "Slime 1" not in seen[-1]["names"]
    assert seen[-1]["attrs"]["deaths"] == 0
    # Bounced: higher than where it landed, and (once it has been off the
    # ground longer than the coyote time) showing the jump frame.
    landed = 3.5 + 0.6  # where it was put
    assert max(f["player"][1] for f in seen) > landed + 1.0, "bounced up"
    assert seen[-1]["sprite"]["clip"] == "jump"


def test_falling_into_a_pit_respawns():
    def at_frame(n, ctx):
        if n == 10:
            ctx.scene["Player"].transform.position = (28.5, 2.0, 0.0)  # over the first pit

    seen = play(90, at_frame)
    assert any(f["player"][1] < 0 for f in seen), "it fell"
    x, y, _ = seen[-1]["player"]
    assert (x, y) == pytest.approx(SPAWN, abs=0.3)
    assert seen[-1]["attrs"]["deaths"] == 1


def test_reaching_the_flag_wins():
    seen = play(15, teleport(10, "Flag", (0.0, -0.5)))
    assert seen[-1]["attrs"]["won"] is True


def test_slimes_patrol_their_ledge():
    seen = play(240)
    xs = [f["slime"][0] for f in seen if f["slime"]]
    home = 25.5
    assert min(xs) == pytest.approx(home - build_level.SLIME_RANGE, abs=0.05)
    assert max(xs) == pytest.approx(home + build_level.SLIME_RANGE, abs=0.05)


def test_jumps_are_about_three_cells_high():
    # v^2 / 2g: high enough for every step in the map, not more.
    height = player_script.JUMP**2 / (2 * player_script.GRAVITY)
    assert 3.0 < height < 3.6
    airtime = 2 * player_script.JUMP / player_script.GRAVITY
    assert player_script.SPEED * airtime > 4, "wide enough to clear the pits"
    assert math.isclose(player_script.CUT, 0.45)


TUTORIAL = os.path.join(GAME, "..", "..", "..", "docs", "wiki", "tutorials", "platformer.md")


def test_the_tutorial_quotes_the_scripts_as_they_are():
    """Every ``# scripts/x.py`` block in platformer.md appears verbatim in
    that file."""
    with open(TUTORIAL, encoding="utf-8") as f:
        blocks = re.findall(r"```python\n# (scripts/\w+\.py)\n(.*?)```", f.read(), re.S)
    assert len(blocks) >= 7
    for script, excerpt in blocks:
        with open(os.path.join(GAME, script), encoding="utf-8") as f:
            source = f.read()
        assert excerpt in source, f"{script} no longer contains:\n{excerpt}"
