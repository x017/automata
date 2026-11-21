"""Custom attributes and prefabs — the Python side of Oberon's inspector.

Run with:  pytest crates/automata-python/tests/python
"""

import pytest

import automata as at


def make_game():
    return at.Game(title="tests", headless=True)


def test_staged_attributes_behave_like_a_dict():
    e = at.Entity("crate")
    attrs = e.attributes
    assert len(attrs) == 0 and "hp" not in attrs
    attrs["hp"] = 3
    attrs["speed"] = 2.5
    attrs["alive"] = True
    attrs["label"] = "box"
    attrs["spawn"] = (1, 2, 3)
    attrs["tint"] = (1.0, 0.5, 0.25, 1.0)
    # Every access sees the same underlying entity.
    assert e.attributes["hp"] == 3 and isinstance(e.attributes["hp"], int)
    assert e.attributes["alive"] is True
    assert e.attributes["spawn"] == (1.0, 2.0, 3.0)
    assert e.attributes["tint"] == (1.0, 0.5, 0.25, 1.0)
    assert sorted(e.attributes) == ["alive", "hp", "label", "spawn", "speed", "tint"]
    assert e.attributes.get("missing", 7) == 7
    assert e.attributes.to_dict()["label"] == "box"
    del e.attributes["label"]
    assert "label" not in e.attributes
    with pytest.raises(KeyError):
        e.attributes["label"]
    with pytest.raises(TypeError):
        e.attributes["bad"] = [1, 2]
    with pytest.raises(TypeError):
        e.attributes["bad"] = object()
    e.attributes = {"only": 1}
    assert e.attributes.keys() == ["only"]
    assert "only" in repr(e.attributes)


def test_attributes_and_scripts_round_trip_through_scene_files(tmp_path):
    e = at.Entity("enemy").add(at.Transform(position=(1.0, 0.0, 0.0)))
    e.attributes.update({"hp": 10, "team": "red", "color": (1.0, 0.0, 0.0, 1.0)})
    e.script = "scripts/enemy.py"
    path = tmp_path / "level.ron"
    at.Scene().add(e).save(str(path))
    text = path.read_text()
    assert "attributes" in text and "scripts/enemy.py" in text

    loaded = at.Scene.load(str(path)).find("enemy")
    assert loaded is not None
    assert loaded.attributes.to_dict() == {
        "color": (1.0, 0.0, 0.0, 1.0),
        "hp": 10,
        "team": "red",
    }
    assert loaded.script == "scripts/enemy.py"


def test_live_attributes_read_and_buffer_writes():
    game = make_game()
    e = at.Entity("player")
    e.attributes["hp"] = 5
    e.script = "player.py"
    game.load_scene(at.Scene().add(e))
    seen = []

    def update(ctx):
        player = ctx.scene["player"]
        seen.append(player.attributes["hp"])
        assert player.script == "player.py"
        player.attributes["hp"] = player.attributes["hp"] - 1
        # Writes are visible to later reads in the same frame…
        assert ctx.scene["player"].attributes["hp"] == seen[-1] - 1
        if ctx.time.frame == 2:
            player.attributes["shield"] = True

    game.run(update, max_frames=4)
    assert seen == [5, 4, 3, 2]


def test_prefabs_instantiate_and_save(tmp_path):
    template = at.Entity("crate").add(at.Transform(position=(0.0, 0.5, 0.0)))
    template.attributes["hp"] = 2
    prefab = at.Prefab(template)
    assert len(prefab) == 1 and "crate" in repr(prefab)

    path = tmp_path / "crate.prefab"
    prefab.save(str(path))
    loaded = at.Prefab.load(str(path))

    a = loaded.instantiate(position=(3.0, 0.5, -1.0), name="crate_a")
    assert a.name == "crate_a"
    assert a.attributes["hp"] == 2
    transform = next(c for c in a.components() if isinstance(c, at.Transform))
    assert transform.position == (3.0, 0.5, -1.0)
    # Instances are independent copies.
    a.attributes["hp"] = 99
    assert loaded.instantiate().attributes["hp"] == 2

    scene = at.Scene().add(loaded).add(a)
    assert len(scene) == 2

    with pytest.raises(ValueError):
        at.Prefab().instantiate()
    with pytest.raises(TypeError):
        at.Prefab(42)


def test_prefabs_spawn_at_runtime():
    game = make_game()
    bullet = at.Entity("bullet").add(at.Transform())
    bullet.attributes["damage"] = 4
    prefab = at.Prefab(bullet)
    seen = []

    def update(ctx):
        if ctx.time.frame == 1:
            ctx.scene.spawn(prefab, position=(0.0, 5.0, 0.0))
        elif ctx.time.frame == 2:
            b = ctx.scene["bullet"]
            seen.append((b.transform.position, b.attributes["damage"]))

    game.run(update, max_frames=3)
    assert seen == [((0.0, 5.0, 0.0), 4)]


SCRIPT = '''
def start(entity, ctx):
    entity.state["ticks"] = 0
    print("started", entity.name)


def update(entity, ctx):
    entity.state["ticks"] += 1
    entity.attributes["ticks"] = entity.state["ticks"]
    x, y, z = entity.transform.position
    entity.transform.position = (x + 1.0, y, z)
    assert ctx.scene["Hero"] is entity
    assert ctx.physics.grounded(entity) is False
'''


def test_run_scene_runs_behaviour_scripts(tmp_path, capsys):
    (tmp_path / "scripts").mkdir()
    (tmp_path / "scripts" / "hero.py").write_text(SCRIPT)
    hero = at.Entity("Hero").add(at.Transform())
    hero.script = "scripts/hero.py"
    at.Scene().add(hero).add(at.Entity("Prop").add(at.Transform())).save(
        str(tmp_path / "level.ron")
    )
    seen = []

    def after(ctx):
        h = ctx.scene["Hero"]
        seen.append((h.attributes.get("ticks"), h.transform.position[0]))

    frames = at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=3, update=after)
    assert frames == 3
    # Later reads in the same frame already see the scripts' writes.
    assert seen == [(1, 1.0), (2, 2.0), (3, 3.0)]
    assert "started Hero" in capsys.readouterr().out


def test_run_scene_propagates_script_errors(tmp_path):
    (tmp_path / "bad.py").write_text("def update(entity, ctx):\n    raise RuntimeError('boom')\n")
    e = at.Entity("E").add(at.Transform())
    e.script = "bad.py"
    at.Scene().add(e).save(str(tmp_path / "s.ron"))
    with pytest.raises(RuntimeError, match="boom"):
        at.run_scene(str(tmp_path / "s.ron"), headless=True, max_frames=2)


CAMERA_SCRIPT = """
def update(entity, ctx):
    assert entity.camera is not None
    assert ctx.camera is not None
    ctx.camera.follow(ctx.scene["Hero"], offset=(0, 2, 5), smooth=0)
    ctx.camera.fov = 45
"""


def test_run_scene_scripts_drive_the_camera(tmp_path):
    (tmp_path / "cam.py").write_text(CAMERA_SCRIPT)
    cam = at.Entity("Main Camera").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0)))
    cam.script = "cam.py"
    hero = at.Entity("Hero").add(at.Transform(position=(3, 1, 0)))
    at.Scene().add(cam).add(hero).save(str(tmp_path / "level.ron"))
    seen = []

    def after(ctx):
        c = ctx.scene["Main Camera"].camera_3d
        seen.append((c.position, c.target, c.fov))

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=3, update=after)
    position, target, fov = seen[-1]
    assert position == pytest.approx((3.0, 3.0, 5.0))
    assert target == pytest.approx((3.0, 1.6, 0.0))
    assert fov == pytest.approx(45.0)


def test_coin_run_example_plays_headless(tmp_path):
    """The tutorial project (examples/oberon/coin_run) runs end to end."""
    import importlib.util
    import pathlib
    import shutil

    src = pathlib.Path(__file__).resolve().parents[4] / "examples" / "oberon" / "coin_run"
    shutil.copytree(src / "scripts", tmp_path / "scripts")
    spec = importlib.util.spec_from_file_location("coin_run_build", src / "build_level.py")
    build = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(build)
    build.COINS[0] = (0, 1.0, 0)  # a coin right where the player starts
    build.build().save(str(tmp_path / "level.ron"))
    scores = []

    def after(ctx):
        scores.append(ctx.scene["Rules"].attributes.get("score"))

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=20, update=after)
    assert scores[-1] == 1
