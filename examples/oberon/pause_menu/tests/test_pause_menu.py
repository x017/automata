"""Tests for the Pause Menu scene: game UI drawn with ``ctx.ui``.

    pytest examples/oberon/pause_menu/tests

The scripts run the way Oberon's Play runs them: a ScriptHost fed the
scripted entities, the mouse buttons and the cursor each frame, on an
800x600 screen.
"""

import os
import re

import pytest

from automata import script_api as api
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))

# Where the menus put things on an 800x600 screen (see scripts/menu.py).
RESUME, SETTINGS, RESET = (400, 258), (400, 322), (400, 386)
SLIDER_Y, TOGGLE, LOW, BACK = 227, (262, 274), (284, 362), (400, 444)
MENU_BUTTON = (716, 42)
GAME_SPOT = (400, 520)


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


class Play:
    """The scene's scripted entities, stepped one 60 Hz frame at a time."""

    def __init__(self):
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
                              screen=(800, 600))
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
    assert play.menu["screen"] == "menu"
    assert "PAUSED" in play.texts() and "Resume" in play.texts()
    assert play.records[1]["r"] == (0.0, 0.5, 0.0), "paused shapes don't turn"
    assert play.records[0]["attrs"]["paused"] is True


def test_resume_plays_and_its_click_never_kicks(play):
    play.click(RESUME)
    assert play.menu["screen"] is None
    play.run(30)
    assert play.menu["kicks"] == 0, "the press began on the Resume button"
    assert play.speed() == pytest.approx(1.2, abs=0.01)
    assert play.records[1]["r"][1] > 0.5, "the cube turns"
    assert "Menu" in play.texts() and "PAUSED" not in play.texts()


def test_a_click_in_the_game_kicks_the_shapes(play):
    play.click(RESUME)
    play.click(GAME_SPOT)
    assert play.menu["kicks"] == 1
    assert play.speed() > 4.5 and play.speed("Slab") > 5.0
    play.run(240)
    assert play.speed() == pytest.approx(1.2, abs=0.1), "they ease back"


def test_the_hud_menu_button_pauses_without_a_kick(play):
    play.click(RESUME)
    play.step()  # widgets take presses where they were last drawn
    play.step(MENU_BUTTON, ["left"])
    play.step(MENU_BUTTON, ["left"])  # held: still the UI's
    play.step(MENU_BUTTON)
    assert play.menu["screen"] == "menu"
    assert play.menu["kicks"] == 0 and play.speed() < 1.3


def test_p_toggles_the_menu(play):
    play.step(keys=["p"])
    play.step()
    assert play.menu["screen"] is None
    play.step(keys=["p"])
    assert play.menu["screen"] == "menu"


def test_reset_kicks_is_disabled_until_there_are_kicks(play):
    play.click(RESET)
    assert play.menu["screen"] == "menu"
    play.click(RESUME)
    play.click(GAME_SPOT)
    play.click(GAME_SPOT)
    play.step(keys=["p"])
    play.step()
    assert "Reset kicks (2)" in play.texts()
    play.click(RESET)
    assert play.menu["kicks"] == 0


def test_settings_change_volume_meter_and_graphics(play):
    play.click(SETTINGS)
    assert play.menu["screen"] == "settings"
    play.step()
    assert "SETTINGS" in play.texts()
    # Drag the volume from full down to the middle of the slider.
    play.step((540, SLIDER_Y), ["left"])
    play.step((400, SLIDER_Y), ["left"])
    play.step((400, SLIDER_Y))
    assert play.host.ctx.settings.volume == pytest.approx(0.5, abs=0.05)
    assert play.applied["volume"] == pytest.approx(0.5, abs=0.05)
    play.click(TOGGLE)
    assert play.host.ctx.settings["spin_meter"] is False
    play.click(LOW)
    assert play.applied["graphics"] == "low"
    play.click(BACK)
    assert play.menu["screen"] == "menu"
    play.click(RESUME)
    play.step()
    assert not any("rad/s" in t for t in play.texts()), "the spin meter is off"
    assert play.menu["kicks"] == 0, "none of those clicks reached the game"
