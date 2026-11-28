"""Navigation from Python: ``at.NavAgent`` / ``at.Navigation`` /
``at.NavLink``, ``entity.nav`` and ``ctx.nav`` in ``run_scene``, and the
pure-Python ``NavGraph`` Oberon's bridge answers ``ctx.nav`` with."""

import math

import pytest

import automata as at
from automata import script_api as api


def level(script=None, character=False):
    """A floor with a wall across the middle; the drone starts behind it."""
    drone = (
        at.Entity("Drone")
        .add(at.Transform(position=(0.0, 1.2, -4.0)))
        .add(at.NavAgent(speed=4.0, hover=1.2, stop_distance=0.3))
    )
    if character:
        drone = (
            at.Entity("Drone")
            .add(at.Transform(position=(0.0, 1.0, -4.0)))
            .add(at.RigidBody.kinematic_position())
            .add(at.Collider3D.capsule(0.5, 0.3))
            .add(at.CharacterController())
            .add(at.NavAgent(speed=4.0, stop_distance=0.3))
        )
    drone.script = script
    return (
        at.Scene()
        .add(
            at.Entity("Floor")
            .add(at.Transform(position=(0.0, -0.5, 0.0)))
            .add(at.Collider3D.cuboid(16.0, 1.0, 16.0))
        )
        .add(
            at.Entity("Wall")
            .add(at.Transform(position=(-1.0, 1.0, 0.0)))
            .add(at.Collider3D.cuboid(10.0, 2.0, 0.4))
        )
        .add(at.Entity("Goal").add(at.Transform(position=(0.0, 0.0, 4.0))))
        .add(at.Entity("Navigation").add(at.Navigation(agent_radius=0.4, jump_distance=0.0)))
        .add(drone)
    )


def test_nav_components_are_saved():
    agent = at.NavAgent(speed=4.0, hover=1.2)
    assert (agent.speed, agent.hover, agent.radius, agent.avoid) == pytest.approx((4.0, 1.2, 0.4, True))
    assert agent == at.NavAgent(speed=4.0, hover=1.2) and agent != at.NavAgent()
    nav = at.Navigation(file="level.nav.ron", agent_radius=0.5)
    assert nav.file == "level.nav.ron"
    assert (nav.agent_radius, nav.max_slope) == pytest.approx((0.5, 45.0))
    link = at.NavLink("Top", two_way=False)
    assert (link.to, link.two_way) == ("Top", False)
    with pytest.raises(ValueError):
        at.NavAgent(speed=-1.0)
    with pytest.raises(ValueError):
        at.Navigation(cell_size=math.nan)
    text = (
        at.Scene()
        .add(at.Entity("a").add(at.Transform()).add(agent))
        .add(at.Entity("Navigation").add(nav))
        .add(at.Entity("b").add(at.Transform()).add(link))
        .to_ron()
    )
    assert "nav_agent:" in text and "navigation:" in text and "nav_link:" in text
    loaded = at.Scene.from_ron(text)
    comps = [c for e in loaded.entities() for c in e.components()]
    assert [c for c in comps if isinstance(c, at.NavAgent)] == [agent]
    assert [c for c in comps if isinstance(c, at.Navigation)] == [nav]
    assert [c for c in comps if isinstance(c, at.NavLink)] == [link]


def test_an_agent_goes_round_the_wall_to_its_target(tmp_path):
    (tmp_path / "drone.py").write_text(
        "def update(entity, ctx):\n"
        "    if ctx.time.frame == 1:\n"
        "        entity.nav.go('Goal')\n"
        "        p = ctx.nav.path(entity, ctx.scene['Goal'])\n"
        "        entity.attributes['corners'] = len(p)\n"
        "        entity.attributes['ready'] = ctx.nav.ready\n"
        "        entity.attributes['length'] = ctx.nav.path_length(entity, (0, 0, 4))\n"
        "    entity.attributes['arrived'] = entity.nav.arrived\n"
        "    entity.attributes['moving'] = entity.nav.moving\n"
    )
    (tmp_path / "level.ron").write_text(level("drone.py").to_ron())
    seen = {"x": []}

    def after(ctx):
        d = ctx.scene["Drone"]
        seen.update(d.attributes)
        seen["x"].append(d.transform.position[0])
        seen["at"] = d.transform.position

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=360, update=after)
    assert seen["ready"]
    assert seen["corners"] >= 3, "it turns the wall's end"
    assert 9.0 < seen["length"] < 14.0
    assert seen["arrived"] and not seen["moving"]
    x, y, z = seen["at"]
    assert math.hypot(x, z - 4.0) < 0.5
    assert y == pytest.approx(1.2, abs=0.1), "it hovers over the floor"
    assert max(seen["x"]) > 3.9, "it went round the wall's open end"


def test_a_character_agent_walks_through_its_controller(tmp_path):
    (tmp_path / "walker.py").write_text(
        "def update(entity, ctx):\n"
        "    entity.nav.go((0.0, 0.0, 4.0))\n"
        "    entity.attributes['speed'] = entity.nav.speed\n"
    )
    (tmp_path / "level.ron").write_text(level("walker.py", character=True).to_ron())
    seen = {}

    def after(ctx):
        seen["at"] = ctx.scene["Drone"].transform.position
        seen["v"] = ctx.scene["Drone"].character_velocity

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=400, update=after)
    x, y, z = seen["at"]
    assert math.hypot(x, z - 4.0) < 0.6
    assert y == pytest.approx(1.0, abs=0.2), "it stayed on the floor"


def test_bad_nav_arguments_raise():
    host = api.ScriptHost.__new__(api.ScriptHost)
    nav = api.Nav(host)
    with pytest.raises(TypeError):
        nav.path("nowhere", (0, 0, 0))
    with pytest.raises(ValueError):
        nav.path((math.inf, 0, 0), (0, 0, 0))


def square_graph():
    """Two 2×2 squares side by side at x 0..2 and 2..4, then a third
    beyond a gap, reached by a one-way link."""
    polys = [
        [0.0, 0.0, 2.0, 2.0, 0.0, 0.0, 0.0],
        [2.0, 0.0, 4.0, 2.0, 0.0, 0.0, 0.0],
        [6.0, 0.0, 8.0, 2.0, -1.0, 0.0, 0.0],
    ]
    edges = [
        [[1, 2.0, 0.0, 0.0, 2.0, 0.0, 2.0, False]],
        [[0, 2.0, 0.0, 0.0, 2.0, 0.0, 2.0, False], [2, 3.9, 0.0, 1.0, 6.1, -1.0, 1.0, True]],
        [],
    ]
    return api.NavGraph({"polys": polys, "edges": edges})


def test_python_nav_graphs_find_paths_and_links():
    g = square_graph()
    assert g.locate((1.0, 0.5, 1.0))[0] == 0
    assert g.locate((1.0, 9.0, 1.0)) is None, "far above the floor"
    assert g.nearest((-1.0, 0.0, 1.0))[0] == pytest.approx(0.0, abs=0.01)
    straight = g.path((0.5, 0.0, 1.0), (3.5, 0.0, 1.0))
    assert [p[:3] for p in straight] == [(0.5, 0.0, 1.0), (3.5, 0.0, 1.0)]
    jump = g.path((0.5, 0.0, 1.0), (7.0, -1.0, 1.0))
    assert [p[3] for p in jump] == [False, False, True, False]
    assert jump[1][:3] == (3.9, 0.0, 1.0) and jump[2][:3] == (6.1, -1.0, 1.0)
    assert g.path((7.0, -1.0, 1.0), (0.5, 0.0, 1.0)) is None, "links go one way"
    back = g.path((7.0, -1.0, 1.0), (0.5, 0.0, 1.0), partial=True)
    assert back[-1][0] >= 6.0, "the nearest it can get"


def test_python_funnels_hug_corners():
    # An L: along x, then up z.
    polys = [
        [0.0, 0.0, 4.0, 1.0, 0.0, 0.0, 0.0],
        [3.0, 1.0, 4.0, 5.0, 0.0, 0.0, 0.0],
    ]
    edges = [
        [[1, 3.0, 0.0, 1.0, 4.0, 0.0, 1.0, False]],
        [[0, 3.0, 0.0, 1.0, 4.0, 0.0, 1.0, False]],
    ]
    g = api.NavGraph({"polys": polys, "edges": edges})
    p = g.path((0.5, 0.0, 0.5), (3.5, 0.0, 4.5))
    assert [q[:3] for q in p] == [(0.5, 0.0, 0.5), (3.0, 0.0, 1.0), (3.5, 0.0, 4.5)]
