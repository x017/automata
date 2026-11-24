"""Tests for Lamplight Lane: a street built from linked prefab instances.

    pytest examples/oberon/lamplight_lane/tests

The project rebuilds from ``build_level.py`` alone, so the tests rebuild it
into a temporary folder, then check the links: instances that follow their
prefab files, a variant that stores only its changes, and an override that
survives an edit to the prefab.
"""

import os
import re

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))


@pytest.fixture()
def built(tmp_path):
    folder = str(tmp_path)
    path = build_level.write(folder)
    return folder, path


def links(text):
    """``[(path, overrides)]`` for every prefab link in a scene file."""
    out = []
    for block in re.findall(r"prefab: Some\(\((.*?)\)\),\n        \)", text, re.S):
        path = re.search(r'path: "([^"]+)"', block).group(1)
        overrides = re.findall(r'"([a-z_.]+)"', block.split("overrides:")[1]) if "overrides:" in block else []
        out.append((path, overrides))
    return out


def load(folder, path):
    at.set_asset_root(folder)
    try:
        return {e.name: e for e in at.Scene.load(path).entities()}
    finally:
        at.set_asset_root(None)


def mesh(entity):
    return next(c for c in entity.components() if isinstance(c, at.MeshRenderer))


def test_every_prop_is_a_linked_instance(built):
    folder, path = built
    with open(path) as f:
        found = links(f.read())
    paths = [p for p, _ in found]
    # Five lamps of four entities, ten fence panels, four crates.
    assert len(found) == 5 * 4 + build_level.FENCE_COUNT + 4
    assert paths.count("prefabs/lamp_post.prefab") == 12
    assert paths.count("prefabs/lamp_post_warm.prefab") == 8
    assert paths.count("prefabs/fence.prefab") == build_level.FENCE_COUNT
    assert paths.count("prefabs/crate.prefab") == 4
    # Only the Heavy Crate changes anything.
    changed = [ov for _, ov in found if ov]
    assert changed == [["mesh_visual.color", "attributes.hp"]]
    for p in set(paths):
        assert os.path.isfile(os.path.join(folder, p)), p


def test_the_warm_lamp_is_a_variant_of_the_base(built):
    folder, _ = built
    with open(os.path.join(folder, "prefabs", "lamp_post_warm.prefab")) as f:
        found = links(f.read())
    assert [p for p, _ in found] == ["prefabs/lamp_post.prefab"] * 4
    post, arm, globe, light = (ov for _, ov in found)
    assert post == arm == [], "the post and arm are the base's"
    assert "mesh_visual.surface.emissive" in globe
    assert light == ["point_light.color", "point_light.intensity"]


def test_the_fence_runs_along_a_line(built):
    folder, path = built
    scene = load(folder, path)
    xs = []
    for i in range(build_level.FENCE_COUNT):
        t = next(c for c in scene[f"Fence {i + 1}"].components() if isinstance(c, at.Transform))
        xs.append(t.position[0])
        assert t.position[2] == pytest.approx(-6.6)
    steps = [b - a for a, b in zip(xs, xs[1:])]
    assert steps == pytest.approx([build_level.FENCE_STEP] * (build_level.FENCE_COUNT - 1))


def test_editing_a_prefab_reaches_every_instance_but_keeps_overrides(built):
    folder, path = built
    crate_file = os.path.join(folder, "prefabs", "crate.prefab")
    edited = build_level.crate()
    edited.attributes["hp"] = 5
    edited.attributes["flammable"] = True
    at.Prefab(edited).save(crate_file)
    # The base lamp's post turns brass: the warm variant follows too.
    lamp = build_level.lamp_post()
    lamp[0].add(at.MeshRenderer.cube(size=1.0, color=(0.7, 0.55, 0.2, 1.0), metallic=1.0))
    at.Prefab(lamp).save(os.path.join(folder, "prefabs", "lamp_post.prefab"))

    scene = load(folder, path)
    assert scene["Crate 1"].attributes["hp"] == 5
    assert scene["Crate 1"].attributes["flammable"] is True
    assert scene["Heavy Crate"].attributes["hp"] == 10, "its override stays"
    assert scene["Heavy Crate"].attributes["flammable"] is True
    assert mesh(scene["Heavy Crate"]).color == pytest.approx(build_level.HEAVY)
    assert mesh(scene["Lamp 1"]).color == pytest.approx((0.7, 0.55, 0.2, 1.0))
    assert mesh(scene["Warm Lamp 2"]).color == pytest.approx((0.7, 0.55, 0.2, 1.0))


def test_the_checked_in_level_matches_a_fresh_build(built):
    _, path = built
    with open(path) as f:
        fresh = links(f.read())
    with open(LEVEL) as f:
        assert links(f.read()) == fresh
    for name in ("lamp_post", "lamp_post_warm", "fence", "crate"):
        with open(os.path.join(GAME, "prefabs", f"{name}.prefab")) as f:
            checked_in = f.read()
        with open(os.path.join(os.path.dirname(path), "prefabs", f"{name}.prefab")) as f:
            assert f.read() == checked_in, name
