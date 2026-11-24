"""Material files (``.mat.ron``) on Python-built meshes (ROADMAP D5)."""

from pathlib import Path

import pytest

import automata as at

BRICK = """// A material.
(
    color: (0.9, 0.5, 0.4, 1.0),
    roughness: 0.85,
    base_map: "../textures/brick.png",
    tiling: (3.0, 1.5),
)
"""


@pytest.fixture
def project(tmp_path: Path) -> Path:
    (tmp_path / "materials").mkdir()
    (tmp_path / "materials" / "brick.mat.ron").write_text(BRICK, encoding="utf-8")
    (tmp_path / "materials" / "typo.mat.ron").write_text("(colour: (1, 1, 1, 1))", encoding="utf-8")
    at.set_asset_root(str(tmp_path))
    yield tmp_path
    at.set_asset_root(None)


def test_every_constructor_takes_a_material(project: Path) -> None:
    for make in (at.MeshRenderer.cube, at.MeshRenderer.sphere, at.MeshRenderer.quad):
        mesh = make(material="materials/brick.mat.ron")
        assert mesh.material == "materials/brick.mat.ron"
    assert at.MeshRenderer.cube().material is None


def test_a_material_is_saved_with_the_scene_and_loads_back(project: Path) -> None:
    wall = at.MeshRenderer.cube(2.0, material="materials/brick.mat.ron")
    plain = at.MeshRenderer.cube(2.0)
    scene = (
        at.Scene()
        .add(at.Entity("wall").add(at.Transform()).add(wall))
        .add(at.Entity("box").add(at.Transform()).add(plain))
    )
    text = scene.to_ron()
    assert text.count('material: "materials/brick.mat.ron"') == 1, text
    back = at.Scene.from_ron(text)
    assert back.to_ron() == text


def test_bad_materials_fail_early_with_the_reason(project: Path) -> None:
    with pytest.raises(FileNotFoundError, match="ghost.mat.ron"):
        at.MeshRenderer.cube(material="materials/ghost.mat.ron")
    with pytest.raises(ValueError, match="colour"):
        at.MeshRenderer.cube(material="materials/typo.mat.ron")
    with pytest.raises(ValueError, match=r"\.mat\.ron"):
        at.MeshRenderer.cube(material="textures/brick.png")
    # A missing texture isn't an error: the material draws without it.
    assert at.MeshRenderer.sphere(material="materials/brick.mat.ron").kind == "sphere"


def test_scene_files_with_materials_load_headless(project: Path) -> None:
    level = project / "level.ron"
    level.write_text(
        at.Scene()
        .add(
            at.Entity("wall")
            .add(at.Transform())
            .add(at.MeshRenderer.cube(material="materials/brick.mat.ron"))
        )
        .to_ron(),
        encoding="utf-8",
    )
    scene = at.Scene.load(str(level))
    assert 'material: "materials/brick.mat.ron"' in scene.to_ron()
