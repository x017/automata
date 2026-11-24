"""Tests for Shape Signal: a game whose screens are UI documents.

    pytest examples/oberon/shape_signal/tests

The scripts run the way Oberon's Play runs them: a ScriptHost fed the
scripted entities, keys, mouse buttons and the cursor each frame, with the
project folder as the base the ``ui/*.ui.ron`` documents are found under.
Widgets are found by where their text is drawn, so the tests don't depend
on the exact layout someone gives them in the UI editor.
"""

import os
import re

import pytest

from automata import script_api as api
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])

    def log(self, level, message):
        if level == "error":
            raise AssertionError(message)

    def base_dir(self):
        return GAME


class Play:
    """The scene's scripted entities, stepped one 60 Hz frame at a time."""

    def __init__(self, screen=(1280, 720)):
        self.screen = screen
        self.keys = api.KeySource()
        self.host = api.ScriptHost(Backend(), self.keys)
        game = os.path.join(GAME, "scripts", "game.py")
        shape = os.path.join(GAME, "scripts", "shape.py")
        self.records = [{"id": 1, "name": "Game", "script": game, "attrs": {"seed": 7}}]
        for i, (name, pos, _size, _color, speed) in enumerate(build_level.SHAPES):
            self.records.append({"id": i + 2, "name": name, "script": shape,
                                 "attrs": {"speed": speed}, "p": pos, "r": (0.0, 0.5, 0.0)})
        self.n = 0
        self.cursor = (0, 0)
        self.saved = {}

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
            for key in ("attrs", "p", "r"):
                if key in w:
                    record[key] = w[key]
        if out.settings:
            self.saved.update(out.settings)
        self.out = out
        return out

    def click(self, pos):
        self.step(pos, ["left"])
        self.step(pos)
        return self.step()

    def run(self, frames):
        for _ in range(frames):
            self.step()

    def texts(self):
        return [item["text"] for item in self.out.hud if item["k"] == "text"]

    def at(self, text):
        """Where ``text`` was drawn last frame, in pixels (a button's middle)."""
        item = next(i for i in self.out.hud if i["k"] == "text" and i["text"] == text)
        return (item["x"], item["y"])

    def press(self, text):
        return self.click(self.at(text))

    @property
    def game(self):
        return self.host.entities[1].state

    @property
    def ui(self):
        return self.host.ctx.ui

    def wait_for_signal(self):
        for _ in range(120):
            if self.game["lit"]:
                return self.game["lit"]
            self.step()
        raise AssertionError("no shape lifted")


@pytest.fixture
def play():
    p = Play()
    p.step()  # the first frame draws the title, so the UI knows where it is
    p.step()
    return p


def _numbers_rounded(text):
    return re.sub(r"-?\d+\.\d+(?:e-?\d+)?", lambda m: f"{float(m.group()):.5g}", text.strip())


def test_saved_level_matches_the_build_script():
    with open(LEVEL, encoding="utf-8") as f:
        saved = f.read()
    assert _numbers_rounded(saved) == _numbers_rounded(build_level.build().to_ron())


def test_the_documents_have_the_widgets_the_script_uses(play):
    """Renaming one in the UI editor would break the game: keep them."""
    ui = play.ui
    assert ui.screen("title").widgets == ["best", "hints", "play", "title", "volume",
                                          "volume_text"]
    assert ui.screen("hud").widgets == ["clock", "keys", "lives", "pick_1", "pick_2", "pick_3",
                                        "picks", "score", "streak", "time", "verdict"]
    assert ui.screen("results").widgets == ["again", "best", "detail", "final", "heading",
                                            "title"]
    assert ui.screen("hud").document.hud
    assert ui.screen("title").document.pause


def test_it_starts_on_the_title_with_the_hud_hidden(play):
    assert play.ui.screens.names == ["title"]
    assert play.ui.screens.paused
    texts = play.texts()
    assert "SHAPE SIGNAL" in texts and "Play" in texts and "Best 0" in texts
    assert "SCORE 0" not in texts, "the HUD waits for the round"
    play.run(30)
    assert play.records[1]["r"] == (0.0, 0.5, 0.0), "paused shapes don't turn"


def test_the_title_scales_its_design_to_any_window():
    for size, scale in (((800, 600), 0.625), ((1920, 1080), 1.5), ((720, 1280), 0.5625)):
        p = Play(size)
        p.step()
        p.step()
        x, _ = p.at("Play")
        assert x == pytest.approx(size[0] / 2)
        title = next(i for i in p.out.hud if i["k"] == "text" and i["text"] == "SHAPE SIGNAL")
        assert title["size"] == pytest.approx(28 * scale)
        assert title["font"].endswith("PressStart2P-Regular.ttf")


def test_the_volume_slider_and_hints_toggle_go_to_settings(play):
    play.ui.screen("title").set("volume", 0.8)
    x, y = play.at("Key hints on the HUD")
    play.click((x - 20, y))
    assert play.host.ctx.settings.get("hints") is False
    assert play.ui.screen("title").get("hints") is False
    play.press("Play")
    assert play.ui.screens.names == []
    play.step()
    assert "Pick the shape that lifts: click its button or press 1, 2 or 3" not in play.texts()


def test_play_starts_a_round_and_the_hud_shows_it(play):
    play.press("Play")
    assert play.game["mode"] == "playing"
    assert play.ui.screens.names == []
    play.step()
    texts = play.texts()
    assert "SCORE 0" in texts and "3 / 3 lives" in texts and "45 s" in texts
    assert "1  Cube" in texts and "SHAPE SIGNAL" not in texts


def test_picking_the_lifted_shape_scores_and_lifts_it(play):
    play.press("Play")
    lit = play.wait_for_signal()
    play.run(30)
    i = [s[0] for s in build_level.SHAPES].index(lit)
    assert play.records[i + 1]["p"][1] > build_level.SHAPES[i][1][1] + 0.5, "it lifts"
    play.press(f"{i + 1}  {lit}")
    assert play.game["score"] > 10
    assert play.game["right"] == 1
    play.step()
    assert f"SCORE {play.game['score']}" in play.texts()
    assert f"+{play.game['score']}" in play.texts()


def test_keys_pick_too_and_a_wrong_pick_costs_a_life(play):
    play.press("Play")
    lit = play.wait_for_signal()
    wrong = next(n for n in ("Cube", "Tower", "Slab") if n != lit)
    key = str([s[0] for s in build_level.SHAPES].index(wrong) + 1)
    play.step(keys=[key])
    play.step()
    assert play.game["lives"] == 2 and play.game["missed"] == 1
    play.step()
    assert "MISS" in play.texts() and "2 / 3 lives" in play.texts()


def test_losing_every_life_ends_on_the_results_screen(play):
    play.press("Play")
    for _ in range(3):
        play.wait_for_signal()
        play.run(int(60 * 2.7))  # too slow
    play.step()
    assert play.game["mode"] == "results"
    assert play.ui.screens.names == ["results"]
    play.step()
    texts = play.texts()
    assert "OUT!" in texts and "0 right, 3 missed" in texts and "Play again" in texts
    play.press("Title")
    assert play.ui.screens.names == ["title"]
    play.press("Play")
    assert play.game["mode"] == "playing" and play.game["lives"] == 3


def test_the_best_score_is_kept_in_settings(play):
    play.press("Play")
    lit = play.wait_for_signal()
    play.step(keys=[str([s[0] for s in build_level.SHAPES].index(lit) + 1)])
    play.step()
    score = play.game["score"]
    play.game["left"] = 0.01
    play.run(3)
    assert play.game["mode"] == "results"
    play.step()
    assert "TIME!" in play.texts() and f"Best {score}" in play.texts()
    assert play.host.ctx.settings.get("best") == score
