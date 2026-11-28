"""Tests for the Pause Menu scene: game UI drawn with ``ctx.ui``.

    pytest examples/oberon/pause_menu/tests

The scripts run the way Oberon's Play runs them: a ScriptHost fed the
scripted entities, the keys, the mouse buttons and the cursor each frame,
on an 800x600 screen. Widgets are found by where their text is drawn, so
the tests don't depend on the exact layout.
"""

import os
import re

import pytest

from automata import script_api as api
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))

# A spot in the game with no UI on it, while playing (800x600).
GAME_SPOT = (400, 420)


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


class Play:
    """The scene's scripted entities, stepped one 60 Hz frame at a time."""

    def __init__(self, screen=(800, 600)):
        self.screen = screen
        self.keys = api.KeySource()
        self.host = api.ScriptHost(Backend(), self.keys)
        menu = os.path.join(GAME, "scripts", "menu.py")
        spin = os.path.join(GAME, "scripts", "spin.py")
        self.records = [{"id": 1, "name": "Menu", "script": menu, "attrs": {}}]
        for i, (name, _pos, _size, _color, speed) in enumerate(build_level.SHAPES):
            self.records.append({"id": i + 2, "name": name, "script": spin,
                                 "attrs": {"speed": speed}, "r": (0.0, 0.5, 0.0)})
        self.n = 0
        self.cursor = (0, 0)
        self.applied = {}

    def step(self, pos=None, buttons=(), keys=()):
        if pos is not None:
            self.cursor = pos
        self.n += 1
        self.keys.update(keys, buttons, (0.0, 0.0), False, self.cursor)
        out = self.host.frame(self.records, delta=1 / 60, elapsed=self.n / 60, frame=self.n,
                              screen=self.screen)
        by_id = {r["id"]: r for r in self.records}
        for w in out.writes:
            record = by_id[w["id"]]
            if "attrs" in w:
                record["attrs"] = w["attrs"]
            if "r" in w:
                record["r"] = w["r"]
        if out.settings:
            self.applied.update(out.settings)
        self.out = out
        return out

    def click(self, pos):
        self.step(pos, ["left"])
        return self.step(pos)

    def run(self, frames):
        for _ in range(frames):
            self.step()

    @property
    def menu(self):
        return self.host.entities[1].state

    def speed(self, name="Cube"):
        return self.host.ctx.scene[name].state["speed"]

    def texts(self):
        return [item["text"] for item in self.out.hud if item["k"] == "text"]

    def at(self, text):
        """Where ``text`` was drawn last frame, in pixels (a button's middle)."""
        item = next(i for i in self.out.hud if i["k"] == "text" and i["text"] == text)
        return (item["x"], item["y"])

    def press(self, text):
        """Clicks the widget whose text is ``text``."""
        return self.click(self.at(text))

    def tap(self, *keys):
        self.step(keys=keys)
        return self.step()

    @property
    def screens(self):
        return self.host.ctx.ui.screens.names


@pytest.fixture
def play():
    p = Play()
    p.step()  # the first frame draws the menu, so the UI knows where it is
    return p


def _numbers_rounded(text):
    """Rotations go through f32 sin/cos, whose last digit varies by platform."""
    return re.sub(r"-?\d+\.\d+(?:e-?\d+)?", lambda m: f"{float(m.group()):.5g}", text.strip())


def test_saved_level_matches_the_build_script():
    with open(LEVEL, encoding="utf-8") as f:
        saved = f.read()
    assert _numbers_rounded(saved) == _numbers_rounded(build_level.build().to_ron())


def test_it_starts_paused_on_the_menu_with_the_shapes_still(play):
    play.run(30)
    assert play.screens == ["menu"]
    assert "PAUSED" in play.texts() and "Resume" in play.texts()
    assert play.records[1]["r"] == (0.0, 0.5, 0.0), "paused shapes don't turn"
    assert play.records[0]["attrs"]["paused"] is True


def test_the_menu_is_centred_and_scaled_to_any_window():
    for size, scale in (((800, 600), 0.625), ((1920, 1080), 1.5), ((1280, 720), 1.0)):
        p = Play(size)
        p.step()
        p.step()
        x, _ = p.at("Resume")
        assert x == pytest.approx(size[0] / 2)
        title = next(i for i in p.out.hud if i["k"] == "text" and i["text"] == "PAUSED")
        assert title["size"] == pytest.approx(28 * scale)


def test_resume_plays_and_its_click_never_kicks(play):
    play.press("Resume")
    assert play.screens == []
    play.run(30)
    assert play.menu["kicks"] == 0, "the press began on the Resume button"
    assert play.speed() == pytest.approx(1.2, abs=0.01)
    assert play.records[1]["r"][1] > 0.5, "the cube turns"
    assert "Menu" in play.texts() and "PAUSED" not in play.texts()


def test_a_click_in_the_game_kicks_the_shapes(play):
    play.press("Resume")
    play.step()  # the menu's shade was there last frame
    play.click(GAME_SPOT)
    assert play.menu["kicks"] == 1
    assert play.speed() > 4.5 and play.speed("Slab") > 5.0
    play.run(240)
    assert play.speed() == pytest.approx(1.2, abs=0.1), "they ease back"


def test_the_hud_menu_button_pauses_without_a_kick(play):
    play.press("Resume")
    play.step()  # widgets take presses where they were last drawn
    menu = play.at("Menu")
    play.step(menu, ["left"])
    play.step(menu, ["left"])  # held: still the UI's
    play.step(menu)
    assert play.screens == ["menu"]
    assert play.menu["kicks"] == 0 and play.speed() < 1.3


def test_the_hud_doesnt_react_under_the_menu(play):
    play.step()
    play.press("Menu")  # the HUD's Menu button, under the modal menu
    assert play.screens == ["menu"] and play.menu["kicks"] == 0


def test_p_and_escape_open_and_close_the_menu(play):
    play.tap("p")
    assert play.screens == []
    play.tap("p")
    assert play.screens == ["menu"]
    play.tap("escape")  # the UI closes the top screen...
    assert play.screens == []
    play.tap("escape")  # ...and with none open, Escape is the game's: it opens the menu
    assert play.screens == ["menu"]


def test_keys_and_a_gamepad_drive_the_menus(play):
    play.tap("arrow_down")  # focuses Resume
    play.tap("arrow_down")  # Settings
    play.tap("enter")
    assert play.screens == ["menu", "settings"]
    play.step()  # opened from the keyboard: it focuses its first widget, the volume
    play.tap("arrow_left")
    play.tap("arrow_left")
    assert play.host.ctx.settings.volume == pytest.approx(0.9)
    play.tap("pad_east")  # B: back to the menu
    assert play.screens == ["menu"]
    play.tap("pad_up")  # wraps from the top to the last enabled button: Settings
    play.tap("pad_up")  # Resume
    play.tap("pad_south")
    assert play.screens == [] and play.menu["kicks"] == 0
    play.tap("pad_start")
    assert play.screens == ["menu"]


def test_reset_kicks_is_disabled_until_there_are_kicks(play):
    play.step()
    play.press("Reset kicks (0)")
    assert play.screens == ["menu"]
    play.press("Resume")
    play.step()
    play.click(GAME_SPOT)
    play.click(GAME_SPOT)
    play.tap("p")
    play.step()
    assert "Reset kicks (2)" in play.texts()
    play.press("Reset kicks (2)")
    assert play.menu["kicks"] == 0


def test_settings_change_volume_meter_graphics_and_ui_size(play):
    play.press("Settings")
    assert play.screens == ["menu", "settings"]
    play.step()
    assert "SETTINGS" in play.texts()
    # Drag the volume from full down to the middle of the slider, which
    # sits between the "Volume" row and the toggle.
    left, label_y = play.at("Volume")
    right, _ = play.at("100%")
    _, toggle_y = play.at("Spin meter")
    y = (label_y + toggle_y) / 2
    play.step((right - 2, y), ["left"])
    play.step(((left + right) / 2, y), ["left"])
    play.step(((left + right) / 2, y))
    assert play.host.ctx.settings.volume == pytest.approx(0.5, abs=0.05)
    assert play.applied["volume"] == pytest.approx(0.5, abs=0.05)
    play.press("Spin meter")
    assert play.host.ctx.settings["spin_meter"] is False
    play.press("Low")
    assert play.applied["graphics"] == "low"
    play.press("Large")
    assert play.host.ctx.settings.ui_scale == 1.25
    assert play.host.ctx.ui.scale == pytest.approx(0.625 * 1.25)
    play.step()
    play.press("Back")
    assert play.screens == ["menu"]
    play.step()
    play.press("Resume")
    play.step()
    assert not any("rad/s" in t for t in play.texts()), "the spin meter is off"
    assert play.menu["kicks"] == 0, "none of those clicks reached the game"


def test_titles_and_hud_numbers_use_the_projects_pixel_font(play):
    def fonts():
        return {i["text"]: i.get("font") for i in play.out.hud if i["k"] == "text"}

    pixel = "fonts/PressStart2P-Regular.ttf"
    assert os.path.isfile(os.path.join(GAME, pixel)), "the font ships with the scene"
    assert fonts()["PAUSED"] == pixel
    assert fonts()["Resume"] is None, "buttons keep the built-in font"
    play.press("Resume")
    play.step()
    assert fonts()["KICKS 0"] == pixel and fonts()["SPIN"] == pixel
