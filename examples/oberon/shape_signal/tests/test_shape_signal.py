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

    def language(self):
        return self.system

    system = "en-GB"  # the computer's language: the game starts in it


class Play:
    """The scene's scripted entities, stepped one 60 Hz frame at a time."""

    def __init__(self, screen=(1280, 720), system="en-GB", save_dir=None):
        self.screen = screen
        self.keys = api.KeySource()
        backend = Backend()
        backend.system = system
        self.host = api.ScriptHost(backend, self.keys, save_dir=save_dir)
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
    assert ui.screen("title").widgets == ["best", "hints", "language", "play", "title",
                                          "volume", "volume_text"]
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


# -- languages --------------------------------------------------------------------

STRINGS = os.path.join(GAME, "lang", "strings.csv")


def _keys_used():
    """Every key the documents and the script name."""
    used = set()
    for name in os.listdir(os.path.join(GAME, "ui")):
        with open(os.path.join(GAME, "ui", name), encoding="utf-8") as f:
            used |= set(re.findall(r'text: "([a-z]+\.[a-z_0-9]+)"', f.read()))
    with open(os.path.join(GAME, "scripts", "game.py"), encoding="utf-8") as f:
        used |= set(re.findall(r'"((?:title|hud|results)\.[a-z_0-9]+)"', f.read()))
    return used


def test_every_key_is_in_the_table_in_every_language(play):
    tr = play.host.ctx.tr
    assert tr.languages == ("en", "es", "ja")
    assert [tr.name(code) for code in tr.languages] == ["English", "Español", "日本語"]
    used = _keys_used()
    assert {"title.play", "hud.keys", "results.again", "hud.score", "hud.slow"} <= used
    assert sorted(used - set(tr.keys())) == [], "keys the table lacks"
    assert sorted(set(tr.keys()) - used) == [], "rows nothing uses"
    for key in tr.keys():
        for code in tr.languages:
            assert tr(key, language=code, score=1, count=2, seconds=3, name="x", value=1,
                      max=3, right=1, missed=0), f"{key} has no {code} text"


def _font_characters(path):
    """The code points a TrueType font maps (its cmap's format 4 and 12
    subtables), with the standard library only."""
    import struct  # noqa: PLC0415 - only this helper needs it

    with open(path, "rb") as f:
        data = f.read()
    tables = {data[12 + 16 * i:16 + 16 * i]: struct.unpack_from(">I", data, 20 + 16 * i)[0]
              for i in range(struct.unpack_from(">H", data, 4)[0])}
    cmap = tables[b"cmap"]
    found = set()
    for i in range(struct.unpack_from(">H", data, cmap + 2)[0]):
        offset = cmap + struct.unpack_from(">I", data, cmap + 8 + 8 * i)[0]
        kind = struct.unpack_from(">H", data, offset)[0]
        if kind == 4:
            n = struct.unpack_from(">H", data, offset + 6)[0] // 2
            ends = struct.unpack_from(f">{n}H", data, offset + 14)
            starts = struct.unpack_from(f">{n}H", data, offset + 16 + 2 * n)
            found.update(c for a, b in zip(starts, ends) for c in range(a, b + 1) if c != 0xFFFF)
        elif kind == 12:
            for g in range(struct.unpack_from(">I", data, offset + 12)[0]):
                a, b, _ = struct.unpack_from(">3I", data, offset + 16 + 12 * g)
                found.update(range(a, b + 1))
    return found


def test_the_japanese_font_has_every_japanese_character(play):
    tr = play.host.ctx.tr
    assert tr.font("ja") == "fonts/MPLUSRounded1c-ShapeSignal.ttf"
    cmap = _font_characters(os.path.join(GAME, tr.font("ja")))
    assert ord("あ") in cmap and ord("ン") in cmap and ord("A") in cmap
    assert ord("漢") not in cmap, "only the kanji the table uses"
    text = "".join(tr(k, language="ja") for k in tr.keys()) + tr.name("ja")
    assert [c for c in text if c not in "\n" and ord(c) not in cmap] == []


def test_it_starts_in_the_computers_language_if_the_game_has_it():
    p = Play(system="es-MX")
    p.step()
    p.step()
    assert p.host.ctx.settings.language == "es"
    texts = p.texts()
    assert "SEÑAL" in texts and "Jugar" in texts and "Récord 0" in texts
    assert "Idioma: Español" in texts
    assert Play(system="fr-FR").host.ctx.settings.language == "en", "else the first column"


def test_the_language_button_switches_every_screen(play, tmp_path):
    play.press("Language: English")
    assert play.host.ctx.settings.language == "es"
    play.step()
    assert "Idioma: Español" in play.texts()
    play.press("Idioma: Español")
    assert play.host.ctx.settings.language == "ja"
    play.step()
    texts = play.texts()
    assert "シェイプ・シグナル" in texts and "プレイ" in texts and "言語: 日本語" in texts
    assert "ベスト 0" in texts and "キーのヒントを表示" in texts
    # Japanese draws with the table's font where the theme's font lacks it.
    title = next(i for i in play.out.hud if i["k"] == "text" and i["text"] == "シェイプ・シグナル")
    assert title["font"].endswith("PressStart2P-Regular.ttf")
    assert title["fallback"] == "fonts/MPLUSRounded1c-ShapeSignal.ttf"
    play.press("プレイ")
    play.step()
    texts = play.texts()
    assert "スコア 0" in texts and "ライフ 3 / 3" in texts and "45 秒" in texts
    assert "1  キューブ" in texts and "タイム" in texts
    lit = play.wait_for_signal()
    wrong = next(n for n in ("Cube", "Tower", "Slab") if n != lit)
    play.step(keys=[str([s[0] for s in build_level.SHAPES].index(wrong) + 1)])
    play.step()
    play.step()
    assert "ミス" in play.texts()
    play.game["left"] = 0.01
    play.run(3)
    play.step()
    texts = play.texts()
    assert "タイムアップ！" in texts and "せいかい 0・ミス 1" in texts and "もう一度" in texts
    play.press("タイトル")
    play.step()
    play.press("言語: 日本語")
    assert play.host.ctx.settings.language == "en", "it wraps around"


def test_the_chosen_language_is_remembered(tmp_path):
    p = Play(save_dir=tmp_path)
    p.step()
    p.step()
    p.press("Language: English")
    p.host.ctx.settings._flush()
    again = Play(system="en-US", save_dir=tmp_path)
    again.step()
    again.step()
    assert again.host.ctx.settings.language == "es"
    assert "Jugar" in again.texts()
