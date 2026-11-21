"""Tests for the finished Coin Run project (tutorial: From Oberon to Python).

    pip install pytest
    pytest examples/oberon/coin_run/tests

The first group plays the real scene headlessly. The second loads the
behaviour scripts as ordinary modules and drives them with a fake context,
which is how to test logic that depends on time or input.
"""

import math
import os

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
camera_rail = load_script(os.path.join(GAME, "scripts", "camera_rail.py"))
follow_camera = load_script(os.path.join(GAME, "scripts", "follow_camera.py"))


# --------------------------------------------------------------------------
# The real scene, headless
# --------------------------------------------------------------------------


def test_level_loads_and_runs():
    assert at.run_scene(LEVEL, headless=True, max_frames=120) == 120


@pytest.mark.parametrize("name", sorted(build_level.FILES.values()))
def test_every_mood_loads_and_runs(name):
    assert at.run_scene(os.path.join(GAME, name), headless=True, max_frames=30) == 30


def environment_of(scene):
    entity = next(e for e in scene.entities() if e.name == "Environment")
    return next(c for c in entity.components() if isinstance(c, at.Environment))


def test_moods_set_the_look_from_the_tutorial():
    assert environment_of(build_level.build("dusk")).tonemapping == "agx"
    env = environment_of(at.Scene.load(os.path.join(GAME, "level_night.ron")))
    assert env.exposure == pytest.approx(0.8)
    assert env.fog_start == pytest.approx(2.0)


def test_player_stands_on_the_ground():
    heights = []

    def after(ctx):
        heights.append(ctx.scene["Player"].transform.position[1])

    at.run_scene(LEVEL, headless=True, max_frames=120, update=after)
    assert 0.5 < heights[-1] < 1.5, "the player fell through the floor"


def test_intro_starts_at_the_first_shot():
    seen = []

    def after(ctx):
        seen.append(ctx.scene["Main Camera"].camera_3d.position)

    at.run_scene(LEVEL, headless=True, max_frames=10, update=after)
    first_shot = build_level.SHOTS[0][0]
    assert math.dist(seen[-1], first_shot) < 0.5


def test_collecting_a_coin_scores_it(tmp_path):
    import shutil

    shutil.copytree(os.path.join(GAME, "scripts"), tmp_path / "scripts")
    original = list(build_level.COINS)
    build_level.COINS[0] = (0, 1.0, 0)  # a coin right where the player starts
    try:
        build_level.build().save(str(tmp_path / "level.ron"))
    finally:
        build_level.COINS[:] = original
    scores = []

    def after(ctx):
        scores.append(ctx.scene["Rules"].attributes.get("score"))

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=20, update=after)
    assert scores[-1] == 1


# --------------------------------------------------------------------------
# Script logic with a fake context
# --------------------------------------------------------------------------


class FakeCamera:
    def __init__(self, position=(0.0, 0.0, 0.0)):
        self.position = position
        self.target = (0.0, 0.0, 0.0)
        self.fov = 60.0
        self.followed = None

    def follow(self, entity, offset=(0.0, 3.0, 8.0), smooth=8.0, look_height=0.6):
        self.followed = (tuple(offset), smooth)
        p = entity.transform.position
        self.position = tuple(a + b for a, b in zip(p, offset))
        self.target = (p[0], p[1] + look_height, p[2])

    def look_at(self, point):
        self.target = tuple(point)


class FakeTransform:
    def __init__(self, position):
        self.position = position


class FakeEntity:
    def __init__(self, name, position=(0.0, 0.0, 0.0), attributes=None, camera=None):
        self.name = name
        self.transform = FakeTransform(position)
        self.attributes = attributes or {}
        self.state = {}
        self.camera = camera


class FakeInput:
    def __init__(self, held=(), just=()):
        self.held, self.just = set(held), set(just)

    def pressed(self, key):
        return key in self.held

    def just_pressed(self, key):
        return key in self.just

    def axis(self, negative, positive):
        return float(self.pressed(positive)) - float(self.pressed(negative))


class FakeTime:
    def __init__(self, elapsed=0.0, delta=1 / 60):
        self.elapsed, self.delta = elapsed, delta


class FakeScene(list):
    def __getitem__(self, name):
        if isinstance(name, str):
            return next(e for e in self if e.name == name)
        return list.__getitem__(self, name)


class FakeContext:
    def __init__(self, scene, elapsed=0.0, held=(), just=()):
        self.scene = scene
        self.time = FakeTime(elapsed)
        self.input = FakeInput(held, just)


def rail_world():
    player = FakeEntity("Player", (0.0, 0.9, 0.0))
    shots = [
        FakeEntity("Shot 2", (10.0, 4.0, 0.0), {"look": (0.0, 1.0, 0.0)}),
        FakeEntity("Shot 1", (-10.0, 6.0, 10.0), {"look": (0.0, 1.0, -4.0)}),
        FakeEntity("Shot 3", (0.0, 4.4, 8.0), {"look": (0.0, 1.5, 0.0)}),
    ]
    camera = FakeEntity("Main Camera", camera=FakeCamera())
    return FakeScene([player, *shots, camera]), camera


def test_smoothstep_eases_in_and_out():
    assert camera_rail.smoothstep(0.0) == 0.0
    assert camera_rail.smoothstep(0.5) == 0.5
    assert camera_rail.smoothstep(1.0) == 1.0
    assert camera_rail.smoothstep(0.1) < 0.1  # slow start
    assert camera_rail.smoothstep(2.0) == 1.0  # clamped


def test_the_rail_visits_each_shot_in_name_order_then_ends():
    scene, _ = rail_world()
    keys = camera_rail.shots(FakeContext(scene))
    assert [k[0] for k in keys] == [(-10.0, 6.0, 10.0), (10.0, 4.0, 0.0), (0.0, 4.4, 8.0)]
    step = camera_rail.SHOT_TIME
    assert camera_rail.rail(keys, 0.0) == keys[0]
    assert camera_rail.rail(keys, step) == keys[1]
    assert camera_rail.rail(keys, 2 * step) is None


def test_the_camera_flies_the_intro_then_follows_the_player():
    scene, camera = rail_world()
    camera_rail.start(camera, FakeContext(scene))
    camera_rail.update(camera, FakeContext(scene, elapsed=0.0))
    assert camera.camera.position == (-10.0, 6.0, 10.0)
    assert camera.camera.followed is None

    after = 2 * camera_rail.SHOT_TIME + 0.01
    camera_rail.update(camera, FakeContext(scene, elapsed=after))
    offset, smooth = camera.camera.followed
    assert offset == (0.0, camera_rail.HEIGHT, camera_rail.DISTANCE)
    assert smooth == pytest.approx(1.0)  # lazy at first ...
    camera_rail.update(camera, FakeContext(scene, elapsed=after + camera_rail.BLEND))
    assert camera.camera.followed[1] == pytest.approx(8.0)  # ... normal after BLEND


def test_space_skips_the_intro():
    scene, camera = rail_world()
    camera_rail.start(camera, FakeContext(scene))
    camera_rail.update(camera, FakeContext(scene, elapsed=0.5, just={"space"}))
    assert camera.camera.followed is not None


def test_follow_camera_orbits_with_q_and_e():
    player = FakeEntity("Player", (0.0, 0.9, 0.0))
    camera = FakeEntity("Main Camera", camera=FakeCamera())
    scene = FakeScene([player, camera])
    follow_camera.start(camera, FakeContext(scene))
    for _ in range(30):
        follow_camera.update(camera, FakeContext(scene, held={"e"}))
    assert camera.state["yaw"] == pytest.approx(30 * follow_camera.TURN / 60)
    offset, _ = camera.camera.followed
    assert offset[0] > 0.0  # swung around to the side


def test_follow_camera_looks_ahead_of_a_moving_player():
    player = FakeEntity("Player", (0.0, 0.9, 0.0))
    camera = FakeEntity("Main Camera", camera=FakeCamera())
    scene = FakeScene([player, camera])
    follow_camera.start(camera, FakeContext(scene))
    for i in range(60):
        player.transform.position = (i * 5.0 / 60, 0.9, 0.0)  # running +X at 5 m/s
        follow_camera.update(camera, FakeContext(scene))
    assert camera.camera.target[0] > player.transform.position[0] + 0.5


def test_follow_camera_widens_while_sprinting():
    player = FakeEntity("Player", (0.0, 0.9, 0.0))
    camera = FakeEntity("Main Camera", camera=FakeCamera())
    scene = FakeScene([player, camera])
    follow_camera.start(camera, FakeContext(scene))
    for _ in range(60):
        follow_camera.update(camera, FakeContext(scene, held={"shift"}))
    assert camera.camera.fov == pytest.approx(follow_camera.SPRINT_FOV, abs=0.5)
    for _ in range(60):
        follow_camera.update(camera, FakeContext(scene))
    assert camera.camera.fov == pytest.approx(follow_camera.FOV, abs=0.5)


# --------------------------------------------------------------------------
# The tutorial pages show these exact scripts
# --------------------------------------------------------------------------

TUTORIALS = os.path.join(GAME, "..", "..", "..", "docs", "wiki", "tutorials")


def _python_blocks(page):
    import re

    with open(os.path.join(TUTORIALS, page), encoding="utf-8") as f:
        return re.findall(r"```python\n(.*?)```", f.read(), re.S)


def _body(code):
    import re

    return re.sub(r'^""".*?"""\n', "", code, flags=re.S).strip()


@pytest.mark.parametrize(
    "page, index, script",
    [
        ("coin-collector.md", 0, "coin.py"),
        ("coin-collector.md", 1, "rules.py"),
        ("third-person-camera.md", -1, "follow_camera.py"),
        ("camera-rail.md", 0, "camera_rail.py"),
    ],
)
def test_tutorial_pages_show_the_finished_scripts(page, index, script):
    with open(os.path.join(GAME, "scripts", script), encoding="utf-8") as f:
        assert _body(_python_blocks(page)[index]) == _body(f.read())


def test_the_hot_reload_step_points_at_the_spin_line():
    """coin-collector.md sends the reader to coin.py:22, the `t * 2.5` spin line."""
    with open(os.path.join(GAME, "scripts", "coin.py"), encoding="utf-8") as f:
        line_22 = f.read().splitlines()[21]
    assert "entity.transform" in line_22 and "t * 2.5" in line_22
