"""Ragdolls from Python: ``at.Ragdoll`` and ``entity.ragdoll`` in
``run_scene`` (going limp, pushes, getting back up), plus the shared
script API's bookkeeping."""

import math
from pathlib import Path

import pytest

import automata as at
from automata import script_api as api

ROBOT = Path(__file__).resolve().parents[4] / "examples/oberon/robot_plaza/models/robot.glb"


def level(script=None, ragdoll=None):
    robot = (
        at.Entity("Robot")
        .add(at.Transform(position=(0.0, 0.89, 0.0)))
        .add(at.MeshRenderer.model(str(ROBOT)))
        .add(at.Animation("Idle"))
        .add(ragdoll or at.Ragdoll())
    )
    robot.script = script
    return (
        at.Scene()
        .add(
            at.Entity("Floor")
            .add(at.Transform(position=(0.0, -0.5, 0.0)))
            .add(at.Collider3D.cuboid(30.0, 1.0, 30.0))
        )
        .add(robot)
    )


def test_the_ragdoll_component_is_saved():
    r = at.Ragdoll(mass=80.0, stiffness=0.25, blend_time=1.0)
    assert (r.active, r.mass, r.stiffness, r.blend_time) == pytest.approx((False, 80.0, 0.25, 1.0))
    assert r == at.Ragdoll(mass=80.0, stiffness=0.25, blend_time=1.0) and r != at.Ragdoll()
    assert "Ragdoll(active=False" in repr(r)
    for bad in ({"mass": 0.0}, {"stiffness": 2.0}, {"blend_time": math.nan}):
        with pytest.raises(ValueError):
            at.Ragdoll(**bad)
    text = level(ragdoll=r).to_ron()
    assert "ragdoll:" in text
    loaded = at.Scene.from_ron(text)
    comps = [c for e in loaded.entities() for c in e.components()]
    assert [c for c in comps if isinstance(c, at.Ragdoll)] == [r]


def test_a_script_knocks_the_robot_down_and_stands_it_back_up(tmp_path):
    (tmp_path / "robot.py").write_text(
        "def update(entity, ctx):\n"
        "    rag = entity.ragdoll\n"
        "    f = ctx.time.frame\n"
        "    if f == 5:\n"
        "        rag.on(push=(160.0, 0.0, 0.0))\n"
        "        entity.attributes['switched'] = rag.active\n"
        "    if rag.limp:\n"
        "        entity.attributes['was_limp'] = True\n"
        "        entity.attributes['hips_x'] = rag.hips[0]\n"
        "        if rag.resting and f > 60:\n"
        "            rag.off()\n"
        "            entity.attributes['off_at'] = f\n"
        "    if rag.blending:\n"
        "        entity.attributes['blended'] = True\n"
        "    entity.attributes['weight'] = rag.weight\n"
    )
    (tmp_path / "level.ron").write_text(level("robot.py").to_ron())
    seen = {}

    def after(ctx):
        r = ctx.scene["Robot"]
        seen.update(r.attributes)
        seen["at"] = r.transform.position

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=420, update=after, fixed_delta=1 / 60)
    assert seen["switched"]
    assert seen["was_limp"], "it went limp"
    assert seen.get("off_at"), "it came to rest and was switched off"
    assert seen["blended"], "the animation blended back in"
    assert seen["weight"] == pytest.approx(0.0)
    x, y, z = seen["at"]
    assert x > 0.3, "the push knocked it along, and it stood up where it fell"
    assert x == pytest.approx(seen["hips_x"], abs=0.1)
    assert y == pytest.approx(0.89, abs=0.05), "standing on the floor again"


def test_the_shared_api_queues_ragdoll_changes():
    host = api.ScriptHost.__new__(api.ScriptHost)
    e = api.Entity(host, 7)
    e._begin({"name": "Bot", "rag": {"on": False, "mass": 70.0, "stiff": 0.0, "blend": 0.5,
                                     "limp": False, "w": 0.0, "rest": False, "hips": None}})
    rag = e.ragdoll
    assert rag is not None and not rag.limp and rag.hips is None and not rag.blending
    rag.on(push=(1, 2, 3), bone="Head")
    rag.push((0, 0, -4))
    rag.stiffness = 0.5
    assert rag.active and rag.stiffness == 0.5
    w = e._writes()
    assert w["rag"] == {
        "on": True,
        "push": [[[1.0, 2.0, 3.0], "Head"], [[0.0, 0.0, -4.0], None]],
        "stiffness": 0.5,
    }
    with pytest.raises(ValueError):
        rag.mass = -1
    with pytest.raises(ValueError):
        rag.push((math.inf, 0, 0))
    e._begin({"name": "Plain"})
    assert e.ragdoll is None
