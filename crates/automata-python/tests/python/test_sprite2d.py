"""2D from Python: ``at.Sprite`` (clips), ``at.Tilemap`` (auto-tiles,
physics), ``at.CameraFollow2D`` and ``entity.sprite`` in ``run_scene``,
plus the shared script API's bookkeeping."""

import pytest

import automata as at
from automata import script_api as api

LEGEND = {"#": -1, "B": 3}


def hero(script=None):
    e = (
        at.Entity("Player")
        .add(at.Transform(position=(1.5, 3.0, 0.0)))
        .add(
            at.Sprite(
                "hero.png",
                columns=4,
                rows=2,
                clips=[at.SpriteClip("idle", 0, 1, fps=4), at.SpriteClip("run", 4, 7, fps=12)],
                play="idle",
            )
        )
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider2D.cuboid(0.8, 0.9))
        .add(at.CharacterController())
        .add(at.CharacterVelocity((0.0, -5.0, 0.0)))
    )
    e.script = script
    return e


def level(script=None):
    ground = at.Tilemap.from_rows(
        ["..............................",
         "..........BB..................",
         "##############################"],
        legend=LEGEND, tileset="tiles.png", columns=8, rows=4,
        autotiles=[at.AutoTile("Ground", first=16)],
    )
    return (
        at.Scene()
        .add(at.Entity("Level").add(at.Transform()).add(ground))
        .add(hero(script))
        .add(
            at.Entity("Camera")
            .add(at.Transform())
            .add(at.Camera2D(half_height=5.0))
            .add(at.CameraFollow2D("Player", offset=(0.0, 0.0), smoothing=0.0, dead_zone=(0.0, 0.0)))
        )
    )


def test_sprites_and_clips_check_their_values():
    clip = at.SpriteClip("run", 4, 7, fps=12)
    assert (clip.name, clip.first, clip.last, clip.fps, clip.looping) == ("run", 4, 7, 12.0, True)
    assert at.SpriteClip("hit", 3).last == 3
    with pytest.raises(ValueError):
        at.SpriteClip("bad", 5, 2)
    s = at.Sprite("a.png", size=(2, 1), columns=4, rows=2, clips=[clip], play="run", layer=2)
    assert s.size == (2.0, 1.0) and s.layer == 2 and s.clips == [clip] and s.play == "run"
    assert "Sprite(\"a.png\"" in repr(s)
    with pytest.raises(ValueError):
        at.Sprite(play="walk")  # no such clip
    with pytest.raises(ValueError):
        at.Sprite(size=(0, 1))


def test_tilemaps_paint_and_pick_auto_tiles():
    m = at.Tilemap.from_rows(["#.", "##"], legend={"#": -1}, autotiles=[at.AutoTile("G", 16)])
    assert (m.width, m.height, m.filled) == (2, 2, 3)
    # Bottom-left has a neighbour above (1) and right (2).
    assert m.tile_at(0, 0) == 16 + 3
    assert m.tile_at(1, 1) is None
    m2 = m.set(1, 1, 5)
    assert m.get(1, 1) == 0 and m2.get(1, 1) == 5 and m2.tile_at(1, 1) == 4
    assert m2.fill((0, 0), (1, 1), 0).filled == 0
    assert m.resize(3, 1).get(2, 0) == 0
    assert m.cell_at(1.5, 0.2) == (1, 0)


def test_2d_components_are_saved_and_loaded():
    text = level().to_ron()
    for key in ("sprite:", "tilemap:", "camera_follow_2d:", 'cells:'):
        assert key in text
    loaded = at.Scene.from_ron(text)
    comps = [c for e in loaded.entities() for c in e.components()]
    maps = [c for c in comps if isinstance(c, at.Tilemap)]
    assert len(maps) == 1 and maps[0].filled == 32 and maps[0].autotiles[0].first == 16
    sprites = [c for c in comps if isinstance(c, at.Sprite)]
    assert [c.name for c in sprites[0].clips] == ["idle", "run"]
    follows = [c for c in comps if isinstance(c, at.CameraFollow2D)]
    assert follows[0].target == "Player" and follows[0].bounds is None


def test_a_script_runs_its_sprite_on_painted_ground_and_the_camera_follows(tmp_path):
    (tmp_path / "hero.py").write_text(
        "def update(entity, ctx):\n"
        "    s = entity.sprite\n"
        "    f = ctx.time.frame\n"
        "    if f < 30:\n"
        "        entity.character_velocity = (0.0, -5.0, 0.0)\n"
        "        if s.clip:\n"
        "            entity.attributes['idle_clip'] = s.clip\n"
        "    else:\n"
        "        entity.character_velocity = (4.0, -5.0, 0.0)\n"
        "        s.play('run')\n"
        "        s.flip_x = True\n"
        "        entity.attributes['run_frame'] = s.frame\n"
        "        entity.attributes['clips'] = ','.join(s.clips)\n"
        "    entity.attributes['grounded'] = entity.grounded\n"
    )
    (tmp_path / "level.ron").write_text(level("hero.py").to_ron())
    seen = {}

    def after(ctx):
        p = ctx.scene["Player"]
        seen.update(p.attributes)
        seen["player"] = p.transform.position
        seen["camera"] = ctx.scene["Camera"].transform.position
        seen["sprite"] = p.sprite_state

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=90, update=after, fixed_delta=1 / 60)
    assert seen["idle_clip"] == "idle"
    assert seen["grounded"], "stands on the tilemap"
    x, y, _ = seen["player"]
    assert y == pytest.approx(1.45, abs=0.1) and x > 3.0
    assert 4 <= seen["run_frame"] <= 7
    assert seen["clips"] == "idle,run"
    assert seen["sprite"]["clip"] == "run" and seen["sprite"]["fx"] is True
    cx, cy, _ = seen["camera"]
    assert cx == pytest.approx(x, abs=0.2) and cy == pytest.approx(y, abs=0.2)


def test_the_shared_api_queues_sprite_changes():
    e = api.Entity(None, 7)
    e._begin({"name": "Hero", "spr": {"clip": "idle", "frame": 1, "clips": ["idle", "run"], "fx": False}})
    s = e.sprite
    assert s.clip == "idle" and s.frame == 1 and not s.flip_x
    s.play("run")
    s.flip_x = True
    assert s.clip == "run" and s.flip_x
    with pytest.raises(ValueError):
        s.play("swim")
    s.frame = 3
    assert s.clip is None and s.frame == 3
    s.color = (1, 0, 0)
    w = e._writes()
    assert w["spr"] == {"flip_x": True, "frame": 3, "color": [1.0, 0.0, 0.0, 1.0]}
    e._begin({"name": "Rock"})
    assert e.sprite is None
