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


RAMP = (
    "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\n"
    "f 1 3 2\nf 4 5 6\nf 1 2 5\nf 1 5 4\nf 2 3 6\nf 2 6 5\nf 1 4 6\nf 1 6 3\n"
)

PROBE = """
def update(entity, ctx):
    for name, x in (("slope", 0.5), ("top", 1.5)):
        hit = ctx.physics.raycast((x, 10, 1), (0, -1, 0), 50, ignore=entity)
        entity.attributes[name] = hit.point[1] if hit else -1.0
    entity.attributes["box_y"] = ctx.scene["Box"].transform.position[1]
"""


def test_mesh_and_convex_hull_colliders(tmp_path):
    """A ramp (y = x) scaled ×2: rays land on the real slope, and a hull
    gives a dynamic body something to stand on. Collider models resolve
    against the scene's folder like mesh renderers do."""
    (tmp_path / "ramp.obj").write_text(RAMP)
    (tmp_path / "probe.py").write_text(PROBE)
    mesh = at.Collider3D.mesh("ramp.obj")
    assert "Mesh" in repr(mesh) and "ramp.obj" in repr(mesh)
    assert "ConvexHull" in repr(at.Collider3D.convex_hull(tmp_path / "ramp.obj"))

    probe = at.Entity("Probe").add(at.Transform(position=(20, 0, 0)))
    probe.script = "probe.py"
    scene = (
        at.Scene()
        .add(
            at.Entity("Ramp")
            .add(at.Transform(scale=(2, 2, 2)))
            .add(mesh.friction(0.9))
        )
        .add(at.Entity("Floor").add(at.Transform(position=(0, -5.5, 0))).add(at.Collider3D.cuboid(40, 1, 40)))
        .add(
            at.Entity("Box")
            .add(at.Transform(position=(-6, -2, 0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.convex_hull("ramp.obj"))
        )
        .add(probe)
    )
    scene.save(str(tmp_path / "level.ron"))
    text = (tmp_path / "level.ron").read_text()
    assert 'Mesh(' in text and 'model: "ramp.obj"' in text

    seen = {}

    def after(ctx):
        seen.update(dict(ctx.scene["Probe"].attributes))

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=120, update=after)
    assert seen["slope"] == pytest.approx(0.5, abs=1e-3)
    assert seen["top"] == pytest.approx(1.5, abs=1e-3)
    # Floor top at y = -5: the hull's flat base rests on it.
    assert seen["box_y"] == pytest.approx(-5.0, abs=0.1)
