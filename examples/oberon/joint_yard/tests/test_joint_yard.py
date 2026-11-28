"""Tests for the Joint Yard scene: a swinging door, a rope bridge, a chain.

    pytest examples/oberon/joint_yard/tests
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


def joints(scene):
    return {
        e.name: c for e in scene.entities() for c in e.components() if isinstance(c, at.Joint)
    }


def _numbers_rounded(text):
    """Rotations go through f32 sin/cos, whose last digit varies by platform."""
    return re.sub(r"-?\d+\.\d+(?:e-?\d+)?", lambda m: f"{float(m.group()):.5g}", text.strip())


def test_saved_level_matches_the_build_script():
    with open(LEVEL, encoding="utf-8") as f:
        saved = f.read()
    assert _numbers_rounded(saved) == _numbers_rounded(build_level.build().to_ron())


def test_every_joint_names_an_entity_in_the_scene():
    scene = at.Scene.load(LEVEL)
    names = {e.name for e in scene.entities()}
    found = joints(scene)
    assert found["Door"].kind == "hinge" and found["Door"].motor is not None
    assert found["Sign"].kind == "fixed" and found["Sign"].break_force > 0
    assert found["Lantern"].kind == "spring"
    assert sum(j.kind == "ball" for j in found.values()) == build_level.LINKS + 1
    for name, joint in found.items():
        assert joint.connected in names, f"{name} joins a missing {joint.connected!r}"


@pytest.fixture(scope="module")
def run():
    """Ten seconds of the yard, sampled every frame."""
    frames = []

    def after(ctx):
        s = ctx.scene
        frames.append(
            {
                "t": ctx.time.elapsed,
                "door": s["Door"].attributes.get("angle", 0.0),
                "crate": s["Crate"].transform.position,
                "planks": [s[f"Plank {i}"].transform.position for i in range(1, 11)],
                "links": [s[f"Link {i}"].transform.position
                          for i in range(1, build_level.LINKS + 1)],
                "ball": s["Wrecking Ball"].transform.position,
                "sign": s["Sign"].transform.position,
                "snapped": s["Sign"].attributes.get("snapped", False),
                "lamp": s["Lantern"].transform.position,
            }
        )

    n = at.run_scene(LEVEL, headless=True, max_frames=600, update=after, fixed_delta=1 / 60)
    assert n == 600
    return frames


def test_the_door_swings_open_and_shut(run):
    angles = [f["door"] for f in run]
    opened = max(angles)
    assert opened == pytest.approx(1.4, abs=0.05), "the motor pulls it to its target"
    first_open = next(i for i, a in enumerate(angles) if a > 1.35)
    later = angles[first_open:]
    assert min(later) == pytest.approx(0.0, abs=0.05), "and back shut"
    assert all(-1.71 <= a <= 1.71 for a in angles), "inside its limits"


def test_the_rope_bridge_holds_the_crate(run):
    end = run[-1]
    x0, x1 = build_level.BRIDGE_ENDS
    crate = end["crate"]
    assert x0 < crate[0] < x1 and abs(crate[2]) < 0.6, "landed on the bridge"
    assert crate[1] > build_level.BRIDGE_TOP - 1.0, "and the bridge held it up"
    # The planks stay linked edge to edge, each ~0.6 m from the next.
    planks = end["planks"]
    for a, b in zip(planks, planks[1:]):
        assert math.dist(a, b) == pytest.approx(0.6, abs=0.05)
    assert planks[0][0] > x0 and planks[-1][0] < x1


def test_the_chain_swings_down_and_snaps_the_sign(run):
    top = build_level.CHAIN_TOP
    for f in run:
        links = f["links"]
        assert math.dist(links[0], top) == pytest.approx(build_level.LINK / 2, abs=0.05)
        for a, b in zip(links, links[1:]):
            assert math.dist(a, b) == pytest.approx(build_level.LINK, abs=0.06)
    lowest = min(f["ball"][1] for f in run)
    assert lowest < top[1] - build_level.LINK * build_level.LINKS, "swung down"
    snapped_at = next(f["t"] for f in run if f["snapped"])
    assert snapped_at < 3.0
    assert run[-1]["sign"][1] < 0.5, "the sign fell to the ground"


def test_the_lantern_bobs_on_its_spring(run):
    heights = [f["lamp"][1] for f in run[60:]]
    assert max(heights) - min(heights) > 0.05, "it bobs"
    assert 3.0 < heights[-1] < 5.5, "and hangs below the beam"
