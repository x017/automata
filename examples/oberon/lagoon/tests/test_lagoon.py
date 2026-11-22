"""Tests for the Lagoon example.

    pip install pytest
    pytest examples/oberon/lagoon/tests
"""

import os
import re

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
main = load_script(os.path.join(GAME, "main.py"))
swimmer = load_script(os.path.join(GAME, "scripts", "swimmer.py"))
orbit = load_script(os.path.join(GAME, "scripts", "orbit_camera.py"))
FLOATING = [entry[0] for entry in build_level.FLOATING]
NUMBER = r"-?\d+\.\d+(?:[eE][-+]?\d+)?"


def same_scene_text(a, b):
    """Scene files equal up to float noise in the last digit."""
    if re.split(NUMBER, a) != re.split(NUMBER, b):
        return False
    xs, ys = re.findall(NUMBER, a), re.findall(NUMBER, b)
    return all(abs(float(x) - float(y)) <= 1e-5 for x, y in zip(xs, ys))


def test_level_file_is_up_to_date():
    with open(LEVEL, encoding="utf-8") as f:
        saved = f.read()
    assert same_scene_text(build_level.build().to_ron(), saved), (
        "level.ron is stale: run examples/oberon/lagoon/build_level.py"
    )


@pytest.fixture(scope="module")
def played():
    """Ten seconds of the real scene, headless: every floating thing's
    position each frame, the water height beside the pier and far out,
    the boat's position and the clock."""
    frames = []

    def record(ctx):
        frames.append(
            (
                {name: ctx.scene[name].transform.position for name in FLOATING},
                ctx.physics.water_height(-8.5, 24.0),
                ctx.physics.water_height(500.0, 0.0),
                ctx.scene["Boat"].transform.position,
                ctx.time.elapsed,
                ctx.scene["Swimmer"].transform.position,
            )
        )

    count = at.run_scene(LEVEL, headless=True, max_frames=600, update=record)
    assert count == 600
    return frames


def test_floating_things_ride_the_swell(played):
    settled = played[300:]
    for name in FLOATING:
        ys = [frame[0][name][1] for frame in settled]
        # Afloat: neither sunk to the seabed nor thrown into the air.
        assert -1.0 < min(ys) and max(ys) < 1.0, (name, min(ys), max(ys))
        # Riding the waves, not sitting still.
        assert max(ys) - min(ys) > 0.1, (name, min(ys), max(ys))


def test_floating_things_stay_roughly_where_they_were_put(played):
    first, last = played[0][0], played[-1][0]
    for name in FLOATING:
        dx = last[name][0] - first[name][0]
        dz = last[name][2] - first[name][2]
        assert dx * dx + dz * dz < 2.0**2, (name, dx, dz)


def test_scripts_see_the_moving_surface(played):
    heights = [frame[1] for frame in played]
    assert all(h is not None for h in heights)
    assert max(heights) - min(heights) > 0.1
    assert all(frame[2] is None for frame in played)


def test_the_boat_cruises_its_loop_on_the_waves(played):
    loop = build_level.BOAT_LOOP
    cx, _, cz = loop["center"]
    rx, rz = loop["radius_x"], loop["radius_z"]
    boats = [frame[3] for frame in played]
    travelled = sum(
        ((b[0] - a[0]) ** 2 + (b[2] - a[2]) ** 2) ** 0.5 for a, b in zip(boats, boats[1:])
    )
    # At its speed (headless frames run on the wall clock).
    seconds = played[-1][4] - played[0][4]
    assert travelled == pytest.approx(loop["speed"] * seconds, rel=0.1)
    for x, y, z in boats[60:]:
        # On the loop, and riding the swell at the waterline.
        assert ((x - cx) / rx) ** 2 + ((z - cz) / rz) ** 2 == pytest.approx(1.0, abs=0.02)
        assert -0.6 < y < 0.8
    ys = [b[1] for b in boats[60:]]
    assert max(ys) - min(ys) > 0.1


def test_the_boat_loop_clears_the_floating_things():
    loop = build_level.BOAT_LOOP
    cx, _, cz = loop["center"]
    rx, rz = loop["radius_x"], loop["radius_z"]
    for name, (x, _, z), *_ in build_level.FLOATING:
        # How far inside or outside the ellipse, roughly in metres.
        off = (((x - cx) / rx) ** 2 + ((z - cz) / rz) ** 2) ** 0.5 - 1.0
        assert abs(off) * min(rx, rz) > 2.0, (name, off)


def test_quality_cycle_wraps():
    assert main.next_quality("low") == "medium"
    assert main.next_quality("beautiful") == main.QUALITIES[0]


def test_headless_entry_point_runs_on_every_preset(capsys):
    for quality in ("low", "beautiful"):
        main.main(["--headless", "--graphics", quality])
    assert "ran 300 frames" in capsys.readouterr().out


def test_the_swimmer_waits_on_the_pier(played):
    # Nobody pressed Tab: it stands where it was put, on the deck.
    x, y, z = build_level.SWIMMER_AT
    for sx, sy, sz in (frame[5] for frame in played[30:]):
        assert (sx, sz) == pytest.approx((x, z), abs=0.05)
        assert sy == pytest.approx(y, abs=0.15)


class Keys:
    def __init__(self, held=(), tapped=()):
        self.held, self.tapped = set(held), set(tapped)

    def pressed(self, key):
        return key in self.held or key in self.tapped

    def just_pressed(self, key):
        return key in self.tapped


class Thing:
    """Just enough of an entity for the swimmer script."""

    def __init__(self, position):
        self.state, self.attributes = {}, {}
        self.transform = type("T", (), {"position": position})()
        self.character_velocity = (0.0, 0.0, 0.0)


class Ctx:
    """A sea at y = 0 and a camera looking along -z."""

    def __init__(self, keys, grounded=False):
        self.input = keys
        self.time = type("Time", (), {"delta": 1 / 60})()
        self.water = type("W", (), {"height": staticmethod(lambda x, z: 0.0)})()
        self.physics = type("P", (), {"grounded": staticmethod(lambda e: grounded)})()
        self.camera = type("C", (), {"position": (0.0, 2.0, 6.0), "target": (0.0, 0.0, 0.0)})()


def swim(position, keys, frames=1, grounded=False):
    """Runs the swimmer for `frames` steps (moving it by its velocity)."""
    thing = Thing(position)
    swimmer.start(thing, None)
    thing.state["active"] = True
    for _ in range(frames):
        swimmer.update(thing, Ctx(keys, grounded))
        x, y, z = thing.transform.position
        vx, vy, vz = thing.character_velocity
        thing.transform.position = (x + vx / 60, y + vy / 60, z + vz / 60)
    return thing


def test_tab_takes_control_and_hands_it_back():
    thing = Thing((0.0, 2.0, 0.0))
    swimmer.start(thing, None)
    swimmer.update(thing, Ctx(Keys(tapped={"tab"}), grounded=True))
    assert thing.state["active"]
    swimmer.update(thing, Ctx(Keys(held={"w"}), grounded=True))
    assert thing.character_velocity[2] < -1.0  # away from the camera
    swimmer.update(thing, Ctx(Keys(tapped={"tab"}), grounded=True))
    assert not thing.state["active"]
    assert thing.character_velocity[0] == thing.character_velocity[2] == 0.0


def test_space_jumps_off_the_ground():
    thing = swim((0.0, 2.0, 0.0), Keys(tapped={"space"}), grounded=True)
    assert thing.character_velocity[1] == pytest.approx(6.5)


def test_the_swimmer_floats_head_out():
    # Dropped in deep: it bobs back up to floating height and stays there.
    thing = swim((0.0, -2.0, 0.0), Keys(), frames=600)
    assert thing.transform.position[1] == pytest.approx(-swimmer.FLOAT, abs=0.05)
    assert thing.state["swimming"]


def test_c_dives_and_space_swims_back_up():
    thing = swim((0.0, -swimmer.FLOAT, 0.0), Keys(held={"c"}), frames=90)
    assert thing.state["depth"] > 2.0
    down = thing.transform.position[1]
    thing2 = swim((0.0, down, 0.0), Keys(held={"space"}), frames=30)
    assert thing2.transform.position[1] > down + 0.5


class CameraThing:
    def __init__(self):
        self.state = {}
        self.attributes = dict(build_level.CAMERA)
        self.position = None

        class Cam:
            def __init__(cam):
                cam.position = (0.0, 0.0, 0.0)
                cam.looking = None

            def look_at(cam, target):
                cam.looking = target

        self.camera = Cam()


def test_the_camera_follows_the_active_swimmer():
    diver = Thing((build_level.PIER_X, -3.0, 15.0))
    diver.state.update(active=True, depth=2.5)
    cam = CameraThing()

    class Scene:
        def find(self, name):
            return diver if name == "Swimmer" else None

    ctx = Ctx(Keys())
    ctx.scene = Scene()
    ctx.input = type("I", (), {
        "axis": staticmethod(lambda a, b: 0.0),
        "mouse_pressed": staticmethod(lambda b: False),
        "mouse_delta": (0.0, 0.0),
    })()
    orbit.start(cam, ctx)
    for _ in range(120):
        orbit.update(cam, ctx)
    x, y, z = cam.camera.position
    tx, ty, tz = cam.camera.looking
    assert (tx, tz) == pytest.approx((build_level.PIER_X, 15.0), abs=0.1)
    # Close behind it, and under the water with it.
    dist = ((x - tx) ** 2 + (y - ty) ** 2 + (z - tz) ** 2) ** 0.5
    assert dist == pytest.approx(build_level.CAMERA["follow_distance"], abs=0.1)
    assert y < -orbit.UNDER + 0.05
