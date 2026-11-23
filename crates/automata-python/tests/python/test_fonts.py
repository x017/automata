"""Custom fonts: `font=` on ctx.hud.text / ctx.screen.text / ctx.ui, with
.ttf / .otf files from the project, checked and measured headless."""

import shutil
from pathlib import Path

import pytest

import automata as at
from automata import script_api as api

PIXEL = (Path(__file__).resolve().parents[4]
         / "examples/oberon/pause_menu/fonts/PressStart2P-Regular.ttf")


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


def host_frame(tmp_path, source, n=1):
    path = tmp_path / "hud.py"
    path.write_text(source, encoding="utf-8")
    host = api.ScriptHost(Backend(), api.KeySource())
    records = [{"id": 1, "name": "Hud", "script": str(path), "attrs": {}}]
    out = None
    for i in range(1, n + 1):
        out = host.frame(records, delta=1 / 60, elapsed=i / 60, frame=i, screen=(800, 600))
    return host, out


def texts(out):
    return [i for i in out.hud if i["k"] == "text"]


def test_hud_text_carries_the_font_with_forward_slashes(tmp_path):
    _, out = host_frame(tmp_path, r"""
def update(entity, ctx):
    ctx.hud.text("plain", 0, 0)
    ctx.hud.text("pixel", 0, 0, font="fonts\\Pixel.TTF")
    ctx.hud.text("round", 0, 0, font=" fonts/Round.otf ")
""")
    items = texts(out)
    assert "font" not in items[0]
    assert items[1]["font"] == "fonts/Pixel.TTF"
    assert items[2]["font"] == "fonts/Round.otf"


@pytest.mark.parametrize("font, error", [
    (5, TypeError), ("", TypeError), (Path("fonts/A.ttf"), TypeError),
    ("fonts/A.png", ValueError), ("fonts/A", ValueError),
])
def test_bad_font_arguments_raise(font, error):
    hud = api.ScriptHost(Backend(), api.KeySource()).ctx.hud
    with pytest.raises(error):
        hud.text("x", 0, 0, font=font)


def test_ui_widgets_use_the_theme_font_unless_told_otherwise(tmp_path):
    _, out = host_frame(tmp_path, """
def update(entity, ctx):
    ctx.ui.label("before", 0, 0)
    ctx.ui.theme.font = "fonts/Pixel.ttf"
    ctx.ui.button("Play", 0, 40, 200, 40)
    ctx.ui.label("own", 0, 100, font="fonts/Other.otf")
    ctx.ui.toggle("Sound", True, 0, 140)
""")
    fonts = {i["text"]: i.get("font") for i in texts(out)}
    assert fonts["before"] is None
    assert fonts["Play"] == "fonts/Pixel.ttf"
    assert fonts["own"] == "fonts/Other.otf"
    assert fonts["Sound"] == "fonts/Pixel.ttf"


def test_the_theme_font_is_checked_when_used(tmp_path):
    host = api.ScriptHost(Backend(), api.KeySource())
    host.ctx.ui.theme.font = "fonts/A.woff"
    with pytest.raises(ValueError):
        host.ctx.ui.label("x", 0, 0)


SCRIPT = """
def update(entity, ctx):
    s = entity.attributes
    s["builtin_w"], s["builtin_h"] = ctx.screen.measure_text("Score 100", 20)
    s["pixel_w"], s["pixel_h"] = ctx.screen.measure_text("Score 100", 20, font="fonts/Pixel.ttf")
    try:
        ctx.screen.measure_text("x", 20, font="fonts/fake.ttf")
        s["fake"] = "measured"
    except ValueError as e:
        s["fake"] = str(e)
    try:
        ctx.screen.measure_text("x", 20, font="fonts/x.png")
    except ValueError as e:
        s["png"] = str(e)
    ctx.hud.text("Score 100", 20, 20, size=20, font="fonts/Pixel.ttf")
    ctx.hud.text("Fake", 20, 60, font="fonts/fake.ttf")
    ctx.hud.text("Gone", 20, 90, font="fonts/gone.ttf")
"""


def test_run_scene_measures_with_project_fonts(tmp_path):
    (tmp_path / "fonts").mkdir()
    shutil.copy(PIXEL, tmp_path / "fonts/Pixel.ttf")
    (tmp_path / "fonts/fake.ttf").write_bytes(b"not a font at all")
    (tmp_path / "hud.py").write_text(SCRIPT, encoding="utf-8")
    hud = at.Entity("Hud").add(at.Transform())
    hud.script = "hud.py"
    at.Scene().add(hud).save(str(tmp_path / "level.ron"))

    seen = {}

    def after(ctx):
        seen.update(dict(ctx.scene["Hud"].attributes))

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=4, update=after)
    # Press Start 2P is monospaced: 9 cells of one em each.
    assert seen["pixel_w"] == pytest.approx(9 * 20, abs=1.0)
    assert seen["pixel_w"] > seen["builtin_w"] * 1.3
    assert seen["pixel_h"] == pytest.approx(20, abs=0.01)
    assert "fake.ttf" in seen["fake"]
    assert ".ttf or .otf" in seen["png"]
    # Drawing with a bad or missing font doesn't stop the game (windowed
    # runs log it once and use the built-in font).
    assert seen["pixel_w"] and seen["fake"] != "measured"
