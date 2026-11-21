"""Oberon's bridge (``crates/oberon/src/bridge.py``) answers ray casts from a
grid of the frame's colliders once a scene has many. The grid must give
exactly the hits that testing every collider gives."""

import importlib.util
import math
import os
import random
import sys

import automata.script_api as script_api

BRIDGE = os.path.join(
    os.path.dirname(__file__), "..", "..", "..", "oberon", "src", "bridge.py"
)


def load_bridge():
    sys.modules.setdefault("_automata_script_api", script_api)
    spec = importlib.util.spec_from_file_location("oberon_bridge_under_test", BRIDGE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def brute_force(bridge, shapes, o, d, max_distance, ignore_id, meshes):
    best = None
    for s in shapes:
        if s[0] == ignore_id:
            continue
        hit = bridge._hit_shape(s, o, d, meshes)
        if hit is not None and hit[0] <= max_distance and (best is None or hit[0] < best[1]):
            best = (s[0], hit[0])
    return best


def random_scene(rng, count):
    shapes = [[0, "box", 0.0, -0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 60.0, 0.5, 60.0]]  # a big ground
    for i in range(1, count):
        c = [rng.uniform(-30, 30), rng.uniform(0, 6), rng.uniform(-30, 30)]
        axis = [rng.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(a * a for a in axis)) or 1.0
        half = rng.uniform(0, math.pi)
        q = [a / n * math.sin(half) for a in axis] + [math.cos(half)]
        kind = rng.choice(("ball", "box", "capsule"))
        if kind == "ball":
            shapes.append([i, "ball", *c, rng.uniform(0.2, 1.5)])
        elif kind == "box":
            shapes.append([i, "box", *c, *q, *(rng.uniform(0.1, 2.0) for _ in range(3))])
        else:
            shapes.append([i, "capsule", *c, *q, rng.uniform(0.1, 1.0), rng.uniform(0.1, 0.6)])
    return shapes


def test_the_grid_finds_the_same_hits_as_testing_everything():
    bridge = load_bridge()
    rng = random.Random(7)
    for count in (10, 300):
        shapes = random_scene(rng, count)
        index = bridge.ShapeIndex(shapes)
        assert (index.cells is None) == (count < bridge.ShapeIndex.GRID_FROM)
        for _ in range(400):
            o = (rng.uniform(-35, 35), rng.uniform(-1, 8), rng.uniform(-35, 35))
            d = [rng.uniform(-1, 1) for _ in range(3)]
            n = math.sqrt(sum(v * v for v in d))
            d = tuple(v / n for v in d)
            max_distance = rng.choice((5.0, 40.0, 100.0))
            ignore = rng.choice((None, rng.randrange(count)))
            got = index.raycast(o, d, max_distance, ignore)
            want = brute_force(bridge, shapes, o, d, max_distance, ignore, None)
            if want is None:
                assert got is None
            else:
                assert got is not None and math.isclose(got[3], want[1], abs_tol=1e-9)


def test_axis_aligned_rays_walk_the_grid():
    bridge = load_bridge()
    shapes = [[i, "ball", float(i * 2), 0.0, 0.0, 0.5] for i in range(50)]
    index = bridge.ShapeIndex(shapes)
    hit = bridge.raycast_shapes(shapes, (-5.0, 0.0, 0.0), (1, 0, 0), 100.0, index=index)
    assert hit[0] == 0 and math.isclose(hit[3], 4.5)
    hit = bridge.raycast_shapes(shapes, (200.0, 0.0, 0.0), (-1, 0, 0), 150.0, ignore_id=49, index=index)
    assert hit[0] == 48 and math.isclose(hit[3], 200.0 - 96.0 - 0.5)
    assert bridge.raycast_shapes(shapes, (0.0, 5.0, 0.0), (1, 0, 0), 100.0, index=index) is None
