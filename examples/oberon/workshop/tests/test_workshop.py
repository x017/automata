"""Tests for the Material Workshop: a room dressed with shared material files.

    pytest examples/oberon/workshop/tests

The project rebuilds from ``build_level.py`` and ``textures.py`` alone, so
the tests rebuild it into a temporary folder and check what Oberon and the
engine will read: valid PNGs, materials whose maps all exist, one shared
file per look, and a scene that names them.
"""

import os
import re
import struct
import zlib

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))


def read_png(path):
    """(width, height, rows of RGB tuples) for the 8-bit RGB PNGs we write."""
    with open(path, "rb") as f:
        data = f.read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    pos, idat, size = 8, b"", None
    while pos < len(data):
        (n,) = struct.unpack(">I", data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + n]
        (crc,) = struct.unpack(">I", data[pos + 8 + n:pos + 12 + n])
        assert crc == zlib.crc32(kind + body), f"{path}: bad {kind!r} checksum"
        if kind == b"IHDR":
            size = struct.unpack(">II", body[:8])
            assert body[8:10] == b"\x08\x02", "8-bit RGB"
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    w, h = size
    raw = zlib.decompress(idat)
    rows = []
    for y in range(h):
        line = raw[y * (w * 3 + 1):(y + 1) * (w * 3 + 1)]
        assert line[0] == 0
        rows.append([tuple(line[1 + 3 * x:4 + 3 * x]) for x in range(w)])
    return w, h, rows


@pytest.fixture(scope="module")
def built(tmp_path_factory):
    folder = str(tmp_path_factory.mktemp("workshop"))
    build_level.write_assets(folder)
    scene = build_level.build(folder)
    path = os.path.join(folder, "level.ron")
    scene.save(path)
    yield folder, scene, path
    at.set_asset_root(None)


def materials_in(text):
    return re.findall(r'material: "([^"]+)"', text)


def test_textures_are_valid_and_reproducible(built):
    folder, _, _ = built
    names = sorted(os.listdir(os.path.join(folder, "textures")))
    assert names == sorted(os.listdir(os.path.join(GAME, "textures")))
    for name in names:
        ours = os.path.join(folder, "textures", name)
        w, h, rows = read_png(ours)
        assert w == h and w in (16, 128), name
        with open(ours, "rb") as a, open(os.path.join(GAME, "textures", name), "rb") as b:
            assert a.read() == b.read(), f"textures/{name} is stale: run build_level.py"
    # Normal maps lean around "straight out" (128, 128, 255).
    _, _, rows = read_png(os.path.join(folder, "textures", "brick_normal.png"))
    mean_b = sum(p[2] for row in rows for p in row) / (128 * 128)
    assert mean_b > 200
    # The roughness-metal maps pack roughness in green and metal in blue.
    _, _, rows = read_png(os.path.join(folder, "textures", "brushed_rm.png"))
    assert all(p[2] == 255 and 40 < p[1] < 140 for row in rows for p in row)
    # The pixel-art block has grass on top and dirt below.
    _, _, rows = read_png(os.path.join(folder, "textures", "grass_block.png"))
    assert rows[0][0][1] > rows[0][0][0] and rows[15][0][0] > rows[15][0][2]


def test_every_material_reads_and_its_maps_exist(built):
    folder, _, _ = built
    for name, fields in build_level.MATERIALS.items():
        path = os.path.join(folder, "materials", f"{name}.mat.ron")
        with open(path, encoding="utf-8") as f:
            text = f.read()
        with open(os.path.join(GAME, "materials", f"{name}.mat.ron"), encoding="utf-8") as f:
            assert f.read() == text, f"materials/{name}.mat.ron is stale"
        assert f'material: "materials/{name}.mat.ron"' in text
        for key, value in fields.items():
            if key.endswith("_map"):
                image = os.path.normpath(os.path.join(folder, "materials", value))
                assert os.path.isfile(image), f"{name}: {value}"
        # The engine accepts it (``material=`` loads it eagerly).
        mesh = at.MeshRenderer.sphere(material=f"materials/{name}.mat.ron")
        assert mesh.material == f"materials/{name}.mat.ron"


def test_looks_are_shared_files(built):
    _, scene, path = built
    with open(path, encoding="utf-8") as f:
        text = f.read()
    used = materials_in(text)
    # One wood file dresses all the crates, one brick file both walls.
    assert used.count("materials/crate.mat.ron") == len(build_level.CRATES)
    assert used.count("materials/brick.mat.ron") == 2
    assert set(used) == {f"materials/{n}.mat.ron" for n in build_level.MATERIALS}
    # Plinths keep their own colour beside the materials.
    assert scene.to_ron().count("Plinth") == len(build_level.SHOWPIECES)
    plinth = text[text.index('"Steel Ball Plinth"'):]
    plinth = plinth[:plinth.index("name: Some(", 10)]
    assert "material" not in plinth and "roughness: Some(0.9)" in plinth


def test_the_shipped_level_matches_the_builder(built):
    _, scene, _ = built
    with open(LEVEL, encoding="utf-8") as f:
        shipped = at.Scene.from_ron(f.read())
    assert materials_in(shipped.to_ron()) == materials_in(scene.to_ron())
    names = re.compile(r'name: Some\("([^"]+)"\)')
    assert names.findall(shipped.to_ron()) == names.findall(scene.to_ron())


def test_showpieces_turn(built):
    from automata import script_api as api

    class Backend(api.Backend):
        def error(self, report):
            raise AssertionError(report["traceback"])

    host = api.ScriptHost(Backend(), api.KeySource())
    script = os.path.join(GAME, "scripts", "turntable.py")
    records = [{"id": 1, "name": "Steel Ball", "script": script, "attrs": {"speed": 0.5},
                "p": (0.0, 1.0, 0.0), "r": (0.0, 0.0, 0.0)}]
    yaw = 0.0
    for n in range(1, 61):
        out = host.frame(records, delta=1 / 60, elapsed=n / 60, frame=n, screen=(1280, 720))
        for w in out.writes:
            if "r" in w:
                records[0]["r"] = w["r"]
                yaw = w["r"][1]
    assert yaw == pytest.approx(0.5, abs=0.02)
