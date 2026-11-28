"""``at.Terrain``: a sculpted, painted heightmap ground."""

import math
import struct
import zlib

import pytest

import automata as at


def hill():
    return at.Terrain(size=32, resolution=33).shape(lambda x, z: 6.0 - 0.02 * (x * x + z * z))


def test_defaults_and_keywords():
    t = at.Terrain()
    assert t.size == pytest.approx(64.0)
    assert t.resolution == 129
    assert t.spacing == pytest.approx(0.5)
    assert t.collide
    assert t.height_range == pytest.approx((0.0, 0.0))
    assert [round(c, 2) for c in t.layers[2].color] == [0.78, 0.68, 0.46]
    layers = [at.TerrainLayer(color=(i / 4, 0.5, 0.5), tile=2 + i) for i in range(4)]
    custom = at.Terrain(size=10, resolution=11, layers=layers, roughness=0.4, collide=False)
    assert custom.layers == layers
    assert not custom.collide
    assert custom.roughness == pytest.approx(0.4)
    with pytest.raises(ValueError):
        at.Terrain(layers=layers[:3])
    with pytest.raises(ValueError):
        at.Terrain(size=-1)


def test_shape_and_heights():
    t = hill()
    assert t.height_at(0, 0) == pytest.approx(6.0)
    assert t.height_at(8, 0) == pytest.approx(6.0 - 1.28, abs=0.05)
    assert len(t.heights) == 33 * 33
    nx, ny, nz = t.normal_at(8, 0)
    assert ny > 0.9 and nx > 0.0  # leans away from the top
    with pytest.raises(ZeroDivisionError):
        at.Terrain(size=4, resolution=3).shape(lambda x, z: 1 / 0)


def test_brushes_return_copies():
    flat = at.Terrain(size=32, resolution=33)
    raised = flat.sculpt("raise", (0, 0), radius=6, amount=2)
    assert flat.height_at(0, 0) == pytest.approx(0.0)
    assert raised.height_at(0, 0) == pytest.approx(2.0, abs=0.01)
    assert raised.height_at(10, 0) == pytest.approx(0.0)
    lowered = raised.sculpt("lower", (0, 0), radius=6, amount=2)
    assert lowered.height_at(0, 0) == pytest.approx(0.0, abs=0.01)
    smoothed = raised.sculpt("smooth", (0, 0), radius=6, amount=1)
    assert smoothed.height_at(0, 0) < raised.height_at(0, 0)
    flattened = raised.sculpt("flatten", (2, 0), radius=8, amount=1, height=0.5)
    assert flattened.height_at(2, 0) == pytest.approx(0.5, abs=0.05)
    noisy = flat.sculpt("noise", (0, 0), radius=10, amount=1)
    assert noisy != flat
    with pytest.raises(ValueError):
        flat.sculpt("dig", (0, 0))
    with pytest.raises(ValueError):
        flat.sculpt("raise", (0, 0), radius=0)


def test_painting_layers():
    t = at.Terrain(size=32, resolution=33)
    assert t.weights_at(0, 0) == pytest.approx((1, 0, 0, 0), abs=0.01)
    sandy = t.paint("sand", (0, 0), radius=4)
    assert sandy.weights_at(0, 0)[2] > 0.9
    assert sandy.weights_at(12, 0) == pytest.approx((1, 0, 0, 0), abs=0.01)
    assert sum(sandy.weights_at(1, 1)) == pytest.approx(1.0, abs=0.01)
    rocky = t.paint(1, (0, 0), radius=4, strength=0.5)
    assert 0.3 < rocky.weights_at(0, 0)[1] < 0.7
    with pytest.raises(ValueError):
        t.paint("lava", (0, 0))


def test_auto_paint_follows_the_shape():
    island = (
        at.Terrain(size=64, resolution=65)
        .shape(lambda x, z: 8.0 - 0.012 * (x * x + z * z))
        .sculpt("raise", (12, 0), radius=4, amount=6)  # a steep knob
        .auto_paint(water_level=0.0, beach=1.0, cliff=35)
    )
    # Around the waterline (r ≈ 25.8 m) it's sand; on top, grass.
    shore = island.weights_at(25.5, 0)
    assert shore[2] > 0.5, shore
    assert island.weights_at(0, -4)[0] > 0.5
    # The knob's flanks are rock.
    assert max(island.weights_at(14.2, 0)[1], island.weights_at(9.8, 0)[1]) > 0.4


def png(width, height, rows):
    """A tiny 8-bit greyscale PNG."""
    raw = b"".join(b"\0" + bytes(r) for r in rows)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def test_heightmaps_load_from_images(tmp_path):
    # Left half black, right half white.
    rows = [[0] * 8 + [255] * 8 for _ in range(16)]
    (tmp_path / "hm.png").write_bytes(png(16, 16, rows))
    t = at.Terrain(size=16, resolution=17).load_heightmap(str(tmp_path / "hm.png"), low=-2, high=10)
    assert t.height_at(-7, 0) == pytest.approx(-2.0, abs=0.01)
    assert t.height_at(7, 0) == pytest.approx(10.0, abs=0.01)
    with pytest.raises(ValueError):
        t.load_heightmap(str(tmp_path / "missing.png"))
    finer = t.with_resolution(33)
    assert finer.resolution == 33
    assert finer.height_at(7, 0) == pytest.approx(10.0, abs=0.01)


def test_terrains_round_trip_through_scene_files(tmp_path):
    t = hill().paint("dirt", (3, 3), radius=5)
    scene = at.Scene().add(at.Entity("ground").add(at.Transform()).add(t))
    text = scene.to_ron()
    assert "terrain" in text and len(text) < 20_000
    path = tmp_path / "island.ron"
    scene.save(str(path))
    (loaded,) = [
        c for c in at.Scene.load(str(path)).find("ground").components() if isinstance(c, at.Terrain)
    ]
    assert loaded.resolution == 33
    for a, b in zip(loaded.heights, t.heights):
        assert a == pytest.approx(b, abs=1e-3)
    assert loaded.weights_at(3, 3) == pytest.approx(t.weights_at(3, 3), abs=0.01)


def test_bodies_and_rays_meet_the_ground_with_grass_on_it(tmp_path):
    scene = (
        at.Scene()
        .add(at.Entity("ground").add(at.Transform(position=(0, -1, 0))).add(hill()))
        .add(at.Entity("meadow").add(at.Transform()).add(at.Grass(area=(6, 6))))
        .add(
            at.Entity("ball")
            .add(at.Transform(position=(0, 12, 0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.ball(0.5))
        )
    )
    seen = {}

    def update(ctx):
        hit = ctx.physics.raycast((4, 30, 0), (0, -1, 0), 100)
        seen["ray"] = hit[1][1] if hit else None  # (entity, point, normal, distance)
        seen["ball"] = ctx.scene["ball"].transform.position[1]

    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(scene)
    game.run(update, max_frames=150)
    assert seen["ray"] == pytest.approx(-1.0 + 6.0 - 0.02 * 16, abs=0.01)
    assert seen["ball"] == pytest.approx(-1.0 + 6.0 + 0.5, abs=0.1)
    assert math.isfinite(seen["ball"])
    assert at.Grass().follow_terrain and at.Scatter().follow_terrain


def test_foliage_can_keep_to_its_own_plane():
    grass = at.Grass(area=(4, 4), follow_terrain=False)
    scatter = at.Scatter(area=(4, 4), follow_terrain=False)
    assert not grass.follow_terrain and not scatter.follow_terrain
    assert grass != at.Grass(area=(4, 4))
    text = at.Scene().add(at.Entity("g").add(at.Transform()).add(grass)).to_ron()
    assert "follow_terrain: false" in text
