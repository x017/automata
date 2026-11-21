"""3D model files: MeshRenderer.model, the asset root, collider offsets."""

import os
import shutil

import pytest

import automata as at

MODELS = os.path.join(os.path.dirname(__file__), "..", "..", "..", "..", "examples", "models")
TREE = os.path.abspath(os.path.join(MODELS, "tree.glb"))
CRATE = os.path.abspath(os.path.join(MODELS, "crate.obj"))


def test_model_meshes_report_their_file_and_bounds():
    tree = at.MeshRenderer.model(TREE)
    assert tree.kind == "model"
    assert tree.path == TREE
    lo, hi = tree.bounds
    assert lo[1] == pytest.approx(0.0, abs=1e-4)
    assert hi[1] == pytest.approx(2.4, abs=1e-3)
    assert "tree.glb" in repr(tree)
    # Primitives have no file or bounds.
    cube = at.MeshRenderer.cube(1.0)
    assert cube.path is None and cube.bounds is None


def test_relative_paths_resolve_against_the_asset_root(tmp_path):
    shutil.copy(CRATE, tmp_path / "crate.obj")
    shutil.copy(os.path.join(MODELS, "crate.mtl"), tmp_path / "crate.mtl")
    (tmp_path / "textures").mkdir()
    shutil.copy(os.path.join(MODELS, "textures", "crate.png"), tmp_path / "textures" / "crate.png")
    previous = at.asset_root()
    try:
        at.set_asset_root(str(tmp_path))
        assert at.asset_root() == str(tmp_path)
        crate = at.MeshRenderer.model("crate.obj", color=(1, 0.5, 0.5, 1))
        assert crate.path == "crate.obj"
        assert crate.color == pytest.approx((1, 0.5, 0.5, 1))
    finally:
        at.set_asset_root(previous)


def test_missing_and_broken_models_raise(tmp_path):
    with pytest.raises(FileNotFoundError):
        at.MeshRenderer.model(str(tmp_path / "nope.glb"))
    bad = tmp_path / "bad.obj"
    bad.write_text("v 0 0 0\nf 1 2 9\n")
    with pytest.raises(ValueError):
        at.MeshRenderer.model(str(bad))


def test_scenes_with_models_save_load_and_run(tmp_path):
    shutil.copy(TREE, tmp_path / "tree.glb")
    collider = at.Collider3D.cuboid(1.5, 2.4, 1.5).offset(0.0, 1.2, 0.0)
    at.Scene().add(
        at.Entity("Tree")
        .add(at.Transform())
        .add(at.MeshRenderer.model(str(tmp_path / "tree.glb")))
        .add(at.RigidBody.fixed())
        .add(collider)
    ).save(str(tmp_path / "level.ron"))
    text = (tmp_path / "level.ron").read_text()
    assert "Model" in text and "offset" in text

    scene = at.Scene.load(str(tmp_path / "level.ron"))
    tree = next(e for e in scene.entities() if e.name == "Tree")
    assert tree is not None
    assert at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=3) == 3


def test_the_sample_scene_runs_and_the_crate_lands():
    # The dynamic crate starts 2.5 m up and comes to rest on the other
    # crate (their colliders are offset to cover the meshes).
    landed = []

    def after(ctx):
        landed.append(ctx.scene["Crate 2"].transform.position[1])

    path = os.path.abspath(os.path.join(MODELS, "scene.ron"))
    at.run_scene(path, headless=True, max_frames=180, update=after)
    assert landed[0] > 2.0
    assert landed[-1] == pytest.approx(0.9, abs=0.15)
