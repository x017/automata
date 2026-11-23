"""The script conformance suite, through the engine's runner.

Each folder in `tests/script_conformance/` is a scene, its behaviour scripts
and `expect.json`: what the scene's entities look like after `frames`
frames. Oberon's Play mode runs the same folders through its JSON bridge
(`crates/oberon/src/play.rs`, `script_conformance_*`), so both runners are
held to the same results.
"""

import json
import math
import os

import pytest

import automata as at
from automata.scripting import SceneScripts

ROOT = os.path.join(os.path.dirname(__file__), "..", "..", "..", "..", "tests", "script_conformance")
SCENARIOS = sorted(
    d for d in os.listdir(ROOT) if os.path.isfile(os.path.join(ROOT, d, "expect.json"))
)


def observe(view):
    """An entity's state in expect.json's terms."""
    out = {"attributes": dict(view.attributes.items())}
    t = view.transform
    if t is not None:
        out["position"] = t.position
        out["rotation"] = t.rotation_euler
        out["scale"] = t.scale
    c = view.camera_3d
    if c is not None:
        out["camera"] = (*c.position, *c.target, c.fov)
    return out


def mismatches(expected, actual, path="", tol=1e-3):
    """Differences between an expect.json value and an observed one."""
    if isinstance(expected, bool) or isinstance(actual, bool):
        return [] if expected is actual else [f"{path}: expected {expected!r}, got {actual!r}"]
    if isinstance(expected, (int, float)):
        ok = isinstance(actual, (int, float)) and math.isclose(expected, actual, abs_tol=tol)
        return [] if ok else [f"{path}: expected {expected!r}, got {actual!r}"]
    if isinstance(expected, list):
        if not isinstance(actual, (list, tuple)) or len(actual) != len(expected):
            return [f"{path}: expected {expected!r}, got {actual!r}"]
        return [m for i, (e, a) in enumerate(zip(expected, actual)) for m in mismatches(e, a, f"{path}[{i}]")]
    return [] if expected == actual else [f"{path}: expected {expected!r}, got {actual!r}"]


def check(expect, find):
    problems = []
    for name, want in expect["entities"].items():
        view = find(name)
        if want is None:
            if view is not None:
                problems.append(f"{name}: expected gone, still there")
            continue
        if view is None:
            problems.append(f"{name}: missing")
            continue
        got = observe(view)
        for key, value in want.get("attributes", {}).items():
            if key not in got["attributes"]:
                problems.append(f"{name}.attributes[{key!r}]: missing")
            else:
                problems += mismatches(value, got["attributes"][key], f"{name}.attributes[{key!r}]")
        for key in want.get("attributes_absent", []):
            if key in got["attributes"]:
                problems.append(f"{name}.attributes[{key!r}]: expected absent")
        for key in ("position", "rotation", "scale", "camera"):
            if key in want:
                problems += mismatches(want[key], got.get(key), f"{name}.{key}")
    return problems


@pytest.mark.parametrize("name", SCENARIOS)
def test_scenario(name):
    folder = os.path.join(ROOT, name)
    with open(os.path.join(folder, "expect.json"), encoding="utf-8") as f:
        expect = json.load(f)
    frames = expect["frames"]
    game = at.Game(headless=True, fixed_delta=1 / 60, log_filter="error")
    game.load_scene(at.Scene.load(os.path.join(folder, "scene.ron")))
    scripts = SceneScripts(folder)
    result = {}

    def tick(ctx):
        if ctx.time.frame > frames:
            # The state `frames` frames of scripts left behind.
            result["problems"] = check(expect, ctx.scene.find)
            ctx.quit()
            return
        scripts.tick(ctx)

    game.run(tick, max_frames=frames + 5)
    scripts.stop()
    assert "problems" in result, "the scene ran to the end"
    assert not result["problems"], "\n".join(result["problems"])
