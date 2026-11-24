"""UI documents from Oberon's UI editor (``ui/<name>.ui.ron``), bound to
scripts by name through ``ctx.ui.screen(name)``.

Driven the way Oberon's bridge drives scripts: a KeySource fed keys,
buttons and a cursor each frame, on an 800x600 screen.
"""

import os
import time

import pytest

from automata import script_api as api


class Backend(api.Backend):
    def __init__(self, base):
        self.base = str(base)
        self.logs = []

    def base_dir(self):
        return self.base

    def log(self, level, message):
        self.logs.append((level, message))

    def error(self, report):
        raise AssertionError(report["traceback"])


HUD = """// A HUD made in Oberon
(
    design_size: (1280.0, 720.0),
    hud: true,
    nodes: [
        (
            kind: Column,
            name: "stats",
            anchor: "top_left",
            x: 20.0,
            y: 20.0,
            width: Some(300.0),
            gap: 8.0,
            children: [
                (kind: Label, name: "score", text: "Score 0", size: Some(24.0)),
                (kind: Progress, name: "health", value: 80.0, max: 100.0, text: "{value}/{max}"),
            ],
        ),
        (kind: Button, name: "pause", text: "Pause", anchor: "top_right", x: 20.0, y: 20.0,
         width: Some(120.0)),
    ],
)
"""

MENU = """(
    design_size: (800.0, 600.0),
    pause: true,
    theme: (accent: Some((1.0, 0.5, 0.0, 1.0)), text_size: Some(18.0)),
    nodes: [
        (kind: Column, anchor: "center", width: Some(300.0), padding: 20.0, panel: true,
         children: [
            (kind: Label, text: "MENU", align: "center"),
            (kind: Button, name: "play", text: "Play"),
            (kind: Button, name: "quit", text: "Quit"),
            (kind: Slider, name: "volume", value: 0.5, step: 0.1),
            (kind: Toggle, name: "music", text: "Music", value: 1.0),
            (kind: Space, height: Some(10.0)),
            (kind: Row, columns: 2, children: [
                (kind: Button, name: "a", text: "A"),
                (kind: Button, name: "b", text: "B"),
            ]),
        ]),
    ],
)
"""


class Rig:
    def __init__(self, tmp_path, source, docs=(), screen=(800, 600)):
        ui = tmp_path / "ui"
        ui.mkdir(exist_ok=True)
        for name, text in docs:
            path = ui / f"{name}.ui.ron"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        self.path = tmp_path / "game.py"
        self.path.write_text(source, encoding="utf-8")
        self.keys = api.KeySource()
        self.backend = Backend(tmp_path)
        self.host = api.ScriptHost(self.backend, self.keys)
        self.records = [{"id": 1, "name": "Game", "script": str(self.path), "attrs": {}}]
        self.screen = screen
        self.n = 0

    def step(self, pos=None, buttons=(), keys=()):
        self.n += 1
        self.keys.update(keys, buttons, (0.0, 0.0), False, pos)
        self.out = self.host.frame(self.records, delta=1 / 60, elapsed=self.n / 60,
                                   frame=self.n, screen=self.screen)
        self.state = self.host.entities[1].state
        return self.out

    def click(self, pos):
        self.step(pos, ["left"])
        return self.step(pos)

    def press(self, text):
        return self.click(self.at(text))

    def tap(self, *keys):
        self.step(keys=keys)
        return self.step()

    @property
    def ui(self):
        return self.host.ctx.ui

    def texts(self):
        return [i["text"] for i in self.out.hud if i["k"] == "text"]

    def text(self, label):
        return next(i for i in self.out.hud if i["k"] == "text" and i["text"] == label)

    def at(self, label):
        item = self.text(label)
        return (item["x"], item["y"])


# -- reading RON --------------------------------------------------------------


def test_ron_reads_what_rust_writes():
    value = api._ron_parse("""#![enable(implicit_some)]
    // a comment
    (
        name: "a \\"quoted\\" line\\n\\u{1F600}",   /* block */
        raw: r#"C:\\path "x""#,
        n: -3, f: 1.5e2, h: 0xff, tup: (1, 2.0,), list: [Some(1), None, true, false],
        kind: Column, map: {"k": [1]}, unit: (), nested: Some((x: 1.0)),
        suffix: 2u8, ch: 'x',
    )""")
    assert value == {
        "name": 'a "quoted" line\n\U0001F600', "raw": 'C:\\path "x"', "n": -3, "f": 150.0,
        "h": 255, "tup": [1, 2.0], "list": [1, None, True, False], "kind": "Column",
        "map": {"k": [1]}, "unit": [], "nested": {"x": 1.0}, "suffix": 2, "ch": "x",
    }


def test_ron_errors_say_where():
    with pytest.raises(api.RonError, match=r"hud.ui.ron:3:12: expected ',' or '\)'"):
        api._ron_parse("(\n  a: 1,\n  b: (1, 2 3),\n)", "hud.ui.ron")
    with pytest.raises(api.RonError, match="string never ends"):
        api._ron_parse('("abc')


# -- HUD documents -------------------------------------------------------------


def test_a_hud_document_shows_from_the_start_and_binds_by_name(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    hud = ctx.ui.screen("hud")
    entity.state["shown"] = bool(hud)
    if ctx.time.frame >= 2:
        hud.set("score", f"Score {ctx.time.frame}")
        hud.set("health", 25)
    if hud.clicked("pause"):
        entity.state["paused"] = True
""", [("hud", HUD)])
    rig.step()
    assert rig.state["shown"]
    assert "Score 0" in rig.texts() and "80/100" in rig.texts()
    # 800x600 against 1280x720: scale 0.625. The column's top-left is 20
    # units in from the corner; its label (24 units) is centred on its line.
    score = rig.text("Score 0")
    assert score["size"] == pytest.approx(24 * 0.625)
    assert score["y"] == pytest.approx((20 + 12) * 0.625)
    rig.step()
    assert "Score 2" in rig.texts() and "25/100" in rig.texts()
    rig.press("Pause")
    rig.step()
    assert rig.state["paused"]


def test_hiding_a_hud_and_its_widgets(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    hud = ctx.ui.screen("hud")
    if ctx.time.frame == 2:
        hud.set_visible("stats", False)
    if ctx.time.frame == 3:
        hud.visible = False
    entity.state["visible"] = hud.visible
""", [("hud", HUD)])
    rig.step()
    rig.step()
    assert "Score 0" not in rig.texts() and "Pause" in rig.texts()
    rig.step()
    assert rig.texts() == [] and rig.state["visible"] is False


# -- screens --------------------------------------------------------------------


def test_a_menu_document_is_a_screen_with_its_own_options(tmp_path):
    rig = Rig(tmp_path, """
def start(entity, ctx):
    ctx.ui.screens.show("menu")

def update(entity, ctx):
    menu = ctx.ui.screen("menu")
    if menu.clicked("play"):
        ctx.ui.screens.hide("menu")
        entity.state["played"] = ctx.time.frame
    entity.state["paused"] = ctx.ui.screens.paused
    entity.state["modal"] = ctx.ui.screens.modal
""", [("menu", MENU)])
    rig.step()
    assert rig.state["paused"] and rig.state["modal"] == "menu", "options from the file"
    # The document's own design size (800x600) and theme.
    assert rig.ui.design_size is None, "scripts keep theirs"
    title = rig.text("MENU")
    assert title["size"] == pytest.approx(18) and title["x"] == pytest.approx(400)
    rig.press("Play")  # pressed on frame 2, released (clicked) on frame 3
    assert "played" not in rig.state, "scripts hear of a click the frame after"
    rig.step()
    assert rig.state["played"] == 4
    rig.step()
    assert "Play" not in rig.texts()


def test_sliders_and_toggles_report_changes(tmp_path):
    rig = Rig(tmp_path, """
def start(entity, ctx):
    ctx.ui.screens.show("menu")

def update(entity, ctx):
    menu = ctx.ui.screen("menu")
    entity.state.setdefault("log", [])
    for name in ("volume", "music"):
        if menu.changed(name):
            entity.state["log"].append((name, menu.get(name)))
""", [("menu", MENU)])
    rig.step()
    rig.press("Music")
    rig.step()
    assert rig.state["log"] == [("music", False)]
    # Focus the slider with the keys and nudge it.
    rig.tap("arrow_down")
    rig.tap("arrow_down")
    rig.tap("arrow_down")
    rig.tap("arrow_right")
    rig.step()
    assert ("volume", pytest.approx(0.6)) in rig.state["log"]


def test_disabled_widgets_dont_click(tmp_path):
    rig = Rig(tmp_path, """
def start(entity, ctx):
    ctx.ui.screens.show("menu")

def update(entity, ctx):
    menu = ctx.ui.screen("menu")
    menu.set_enabled("quit", False)
    if menu.clicked("quit"):
        entity.state["quit"] = True
""", [("menu", MENU)])
    rig.step()
    rig.press("Quit")
    rig.step()
    assert "quit" not in rig.state


def test_rows_and_spaces_lay_out_like_the_script_api(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n", [("menu", MENU)])
    rig.ui.screens.show("menu")
    rig.step()
    rig.step()
    a, b = rig.text("A"), rig.text("B")
    # Inside a 300-wide column with 20 padding: two 125-wide cells.
    assert b["x"] - a["x"] == pytest.approx(135)
    assert a["y"] == pytest.approx(b["y"])


# -- mistakes ---------------------------------------------------------------------


def test_misspelt_widgets_and_wrong_values_say_so(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n", [("hud", HUD)])
    rig.step()
    hud = rig.ui.screen("hud")
    with pytest.raises(KeyError, match="no widget named 'helth'.*'health'"):
        hud.set("helth", 3)
    with pytest.raises(TypeError, match="'health' is a progress: set it to a number"):
        hud.set("health", "lots")
    with pytest.raises(TypeError, match="'stats' is a column"):
        hud.set("stats", 1)
    assert hud.widgets == ["health", "pause", "score", "stats"]
    with pytest.raises(LookupError, match="no UI document ui/nope.ui.ron"):
        rig.ui.screen("nope").set("x", 1)


def test_a_broken_document_is_reported_once_and_others_still_draw(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n",
              [("hud", HUD), ("broken", "(hud: true, nodes: [(kind: Buton)])")])
    rig.step()
    rig.step()
    errors = [m for level, m in rig.backend.logs if level == "error"]
    assert len(errors) == 1 and "no node kind 'Buton'" in errors[0] and "'button'" in errors[0]
    assert "Score 0" in rig.texts()


def test_documents_reload_when_their_files_change(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n", [("hud", HUD)])
    rig.step()
    path = tmp_path / "ui" / "hud.ui.ron"
    path.write_text(HUD.replace("Score 0", "Points 0"), encoding="utf-8")
    later = time.time() + 5
    os.utime(path, (later, later))
    (tmp_path / "ui" / "extra.ui.ron").write_text(
        '(hud: true, nodes: [(kind: Label, text: "NEW", x: 5.0, y: 5.0)])', encoding="utf-8")
    rig.ui._check_documents()
    rig.step()
    assert "Points 0" in rig.texts() and "NEW" in rig.texts()


def test_space_needs_a_layout(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    ctx.ui.space(10, 10)\n")
    with pytest.raises(TypeError, match="ui.space only works inside"):
        rig.step()


# -- the same layout as Oberon's UI editor ------------------------------------------

FIXTURES = os.path.join(os.path.dirname(__file__), "..", "..", "..", "oberon", "tests", "fixtures")


@pytest.mark.parametrize("screen", [(800, 600), (1920, 1080), (720, 1280)])
def test_documents_lay_out_where_oberons_editor_shows_them(tmp_path, screen):
    import json

    with open(os.path.join(FIXTURES, "ui_layout.ui.ron"), encoding="utf-8") as f:
        source = f.read()
    with open(os.path.join(FIXTURES, "ui_layout.json"), encoding="utf-8") as f:
        want = json.load(f)[f"{screen[0]}x{screen[1]}"]
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n", [("check", source)], screen)
    pressed = {}
    real = api.Ui._press

    def spy(self, wid, rect, enabled, focusable=True):
        if isinstance(wid[2], tuple) and wid[2][0] == "id":
            doc, key = wid[2][1]
            if doc == "check" and isinstance(key, str):
                s = self.scale
                pressed[key] = [rect[0] * s, rect[1] * s, rect[2] * s, rect[3] * s]
        return real(self, wid, rect, enabled, focusable)

    api.Ui._press = spy
    try:
        rig.step()
        rig.step()  # anchored layouts settle on their first frame; check a steady one
    finally:
        api.Ui._press = real
    doc = rig.ui._document("check")
    got = dict(pressed)
    for name, node in doc.by_name.items():
        if node["kind"] == "label" and node["visible"]:
            item = rig.text(node["text"])
            got[name] = [item["x"], item["y"], item["size"]]
    assert sorted(got) == sorted(want)
    for name, numbers in want.items():
        assert got[name] == pytest.approx(numbers, abs=0.011), name
