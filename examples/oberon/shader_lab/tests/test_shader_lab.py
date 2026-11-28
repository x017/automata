"""Tests for the Shader Lab: custom WGSL material and post effect shaders.

    pytest examples/oberon/shader_lab/tests

The project rebuilds from ``build_level.py`` and the hand-written shaders,
so the tests rebuild it into a temporary folder and check what Oberon and
the engine will read: every shader passes the engine's checks, materials
name their shader and numbers, the scene carries a post effect, and a
headless run draws with them.
"""

import os
import re

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
SHADERS = os.path.join(GAME, "shaders")
build_level = load_script(os.path.join(GAME, "build_level.py"))


@pytest.fixture(scope="module")
def built(tmp_path_factory):
    folder = str(tmp_path_factory.mktemp("shader_lab"))
    build_level.write_assets(folder)
    scene = build_level.build(folder)
    path = os.path.join(folder, "level.ron")
    scene.save(path)
    yield folder, scene, path
    at.set_asset_root(None)


def shader_names():
    return sorted(n[:-5] for n in os.listdir(SHADERS) if n.endswith(".wgsl"))


def test_every_shader_passes_the_engine_checks():
    names = shader_names()
    assert len(names) == 6
    for name in names:
        assert at.check_shader(os.path.join(SHADERS, f"{name}.wgsl")) is None, name


def test_every_shader_documents_its_numbers():
    for name in shader_names():
        with open(os.path.join(SHADERS, f"{name}.wgsl"), encoding="utf-8") as f:
            text = f.read()
        header = text[:text.index("fn ")]
        assert re.search(r"//\s+p0\.", header), f"{name}: say what p0 does"
        post = "fn post(" in text
        assert (name in build_level.POST_EFFECTS) == post, name
        assert (name in build_level.MATERIALS) == (not post), name
        if post:
            assert "discard" not in text, "post effects can't discard"


def test_materials_name_their_shader_and_numbers(built):
    folder, _, _ = built
    for name, fields in build_level.MATERIALS.items():
        path = os.path.join(folder, "materials", f"{name}.mat.ron")
        with open(path, encoding="utf-8") as f:
            text = f.read()
        with open(os.path.join(GAME, "materials", f"{name}.mat.ron"), encoding="utf-8") as f:
            assert f.read() == text, f"materials/{name}.mat.ron is stale: run build_level.py"
        assert f'shader: "../shaders/{name}.wgsl"' in text
        assert len(fields["shader_params"]) <= 8
        assert os.path.isfile(os.path.join(folder, "materials", fields["shader"]))
        # The engine accepts it (``material=`` loads it eagerly).
        mesh = at.MeshRenderer.sphere(material=f"materials/{name}.mat.ron")
        assert mesh.material == f"materials/{name}.mat.ron"


def test_the_scene_wears_the_shaders(built):
    _, scene, path = built
    with open(path, encoding="utf-8") as f:
        text = f.read()
    used = re.findall(r'material: "([^"]+)"', text)
    assert sorted(used) == sorted(f"materials/{n}.mat.ron" for n in build_level.MATERIALS)
    look = next(c for c in scene.find("Look").components() if isinstance(c, at.PostShader))
    assert look.shader == f"shaders/{build_level.LOOK}.wgsl"
    assert look.params == pytest.approx(build_level.POST_EFFECTS[build_level.LOOK])


@pytest.mark.parametrize("look", sorted(build_level.POST_EFFECTS))
def test_each_post_effect_fits_the_component(look):
    params = build_level.POST_EFFECTS[look]
    post = at.PostShader(f"shaders/{look}.wgsl", params=params)
    assert post.params == pytest.approx(params)


def test_the_shipped_level_matches_the_builder(built):
    _, scene, _ = built
    with open(LEVEL, encoding="utf-8") as f:
        shipped = at.Scene.from_ron(f.read())
    names = re.compile(r'name: Some\("([^"]+)"\)')
    assert names.findall(shipped.to_ron()) == names.findall(scene.to_ron())
    assert re.findall(r'shader: "[^"]+"', shipped.to_ron()) == re.findall(
        r'shader: "[^"]+"', scene.to_ron()
    )


def test_a_headless_run_draws_with_the_shaders(built):
    folder, _, path = built
    at.set_asset_root(folder)
    with open(path, encoding="utf-8") as f:
        scene = at.Scene.from_ron(f.read())
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(scene)
    assert game.run(lambda ctx: None, max_frames=3) == 3
