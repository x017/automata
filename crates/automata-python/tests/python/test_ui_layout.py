"""`ctx.ui` layout, scaling, screens and keyboard / gamepad focus.

Driven the way Oberon's bridge drives scripts: a KeySource fed keys,
buttons and a cursor each frame.
"""

import pytest

from automata import script_api as api
from automata.scripting import _pad_keys


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


class Rig:
    """A host running one script on an 800x600 screen."""

    def __init__(self, tmp_path, source, screen=(800, 600)):
        self.path = tmp_path / "ui.py"
        self.path.write_text(source, encoding="utf-8")
        self.keys = api.KeySource()
        self.host = api.ScriptHost(Backend(), self.keys)
        self.records = [{"id": 1, "name": "Ui", "script": str(self.path), "attrs": {}}]
        self.screen = screen
        self.n = 0
        self.held = set()

    def step(self, pos=None, buttons=(), keys=()):
        self.n += 1
        self.keys.update(keys, buttons, (0.0, 0.0), False, pos)
        self.out = self.host.frame(self.records, delta=1 / 60, elapsed=self.n / 60,
                                   frame=self.n, screen=self.screen)
        self.state = self.host.entities[1].state
        return self.out

    def tap(self, *keys):
        """Presses and releases keys over two frames."""
        self.step(keys=keys)
        return self.step()

    def click(self, pos):
        self.step(pos, ["left"])
        return self.step(pos)

    @property
    def ui(self):
        return self.host.ctx.ui

    def rects(self):
        return [i for i in self.out.hud if i["k"] == "rect"]

    def text(self, label):
        return next(i for i in self.out.hud if i["k"] == "text" and i["text"] == label)


# -- scaling and anchors ----------------------------------------------------


def test_design_size_scales_ui_units_to_fit_the_screen(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ctx.ui.design_size = (1280, 720)
    entity.state["size"] = ctx.ui.size
    entity.state["scale"] = ctx.ui.scale
    entity.state["clicked"] = ctx.ui.button("Go", 0, 0, 160, 40)
    ctx.ui.label("Hi", 100, 100, size=20)
""")
    rig.step()
    # 800x600 against 1280x720: the width decides (0.625), and the
    # screen is taller in UI units than the design.
    assert rig.state["scale"] == pytest.approx(0.625)
    assert rig.state["size"] == pytest.approx((1280, 960))
    border = rig.rects()[0]
    assert (border["w"], border["h"]) == pytest.approx((100, 25))
    assert rig.text("Hi")["size"] == pytest.approx(12.5)
    # Clicks are in pixels: (90, 20) is inside the scaled button only.
    rig.click((90, 20))
    assert rig.state["clicked"] is True
    rig.step((110, 20), ["left"])
    rig.step((110, 20))
    assert rig.state["clicked"] is False


def test_the_players_ui_scale_multiplies_and_is_saved(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    entity.state["scale"] = ctx.ui.scale
""")
    rig.step()
    assert rig.state["scale"] == 1.0
    settings = rig.host.ctx.settings
    settings.ui_scale = 1.5
    rig.step()
    assert rig.state["scale"] == 1.5
    settings.ui_scale = 9
    assert settings.ui_scale == 3.0
    rig.host.ctx.ui.design_size = (400, 300)
    rig.step()
    assert rig.state["scale"] == pytest.approx(6.0)
    with pytest.raises(ValueError):
        settings.ui_scale = float("nan")


def test_place_puts_boxes_against_the_edges(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n")
    rig.step()
    ui = rig.ui
    assert ui.place("top_left", 100, 40) == (0, 0)
    assert ui.place("bottom_right", 100, 40, margin=20) == (680, 540)
    assert ui.place("center", 100, 40) == (350, 280)
    assert ui.place("Top-Right", 100, 40, margin=(10, 5)) == (690, 5)
    assert ui.place("centre", 0, 0) == (400, 300)
    with pytest.raises(ValueError, match="bottom_left"):
        ui.place("botom_left", 1, 1)


# -- layouts ----------------------------------------------------------------


def test_a_column_stacks_widgets_and_fills_its_width(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ui = ctx.ui
    with ui.column(100, 50, width=300, gap=10):
        ui.label("Title", align="center", size=20)
        entity.state["a"] = ui.button("A")
        entity.state["b"] = ui.button("B", height=60)
        ui.slider(0.5)
""")
    rig.step()
    title = rig.text("Title")
    assert (title["x"], title["y"]) == (250, 60)  # centred in the column, middle of its line
    borders = [r for r in rig.rects() if r["w"] == 300]
    # A: top 80 (50 + 20 + 10), 44 high; B: top 134, 60 high; the slider at 204.
    assert [(r["x"], r["y"], r["h"]) for r in borders[:2]] == [(250, 102, 44), (250, 164, 60)]
    rig.click((250, 170))
    rig.step((250, 170), ["left"])
    rig.step((250, 170))
    assert rig.state["b"] is True and rig.state["a"] is False


def test_rows_split_into_columns_and_nest_in_columns(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ui = ctx.ui
    with ui.column(0, 0, width=430, gap=10):
        ui.button("Top")
        with ui.row(columns=4, gap=10, height=40):
            for name in ("Low", "Medium", "High", "Ultra"):
                ui.button(name)
        ui.button("Bottom")
    with ui.row(500, 0, gap=5):
        ui.button("X", width=50)
        ui.button("Y")
""")
    rig.step()
    cells = [r for r in rig.rects() if r["w"] == 100 and r["h"] == 40]
    assert [r["x"] for r in cells] == [50, 160, 270, 380]
    assert {r["y"] for r in cells} == {74}
    bottom = rig.text("Bottom")
    assert bottom["y"] == 54 + 50 + 22  # after the 40-high row and two gaps
    x, y = rig.text("X"), rig.text("Y")
    assert (x["x"], y["x"]) == (525, 635)  # 50 wide, then a 160 default button


def test_an_anchored_column_with_a_panel_lands_right_on_its_first_frame(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    with ctx.ui.column(width=200, anchor="bottom_right", margin=20, padding=10,
                       panel=True, title="Menu"):
        ctx.ui.button("One")
        ctx.ui.button("Two")
""")
    rig.step()
    rects = rig.rects()
    # The backdrop comes first (under the buttons) and fits them: 10 + 32
    # (title) + 44 + 10 + 44 + 10 high, right against the margins.
    backdrop = rects[0]
    h = 10 + 32 + 44 + 10 + 44 + 10
    assert (backdrop["w"], backdrop["h"]) == (200, h)
    assert backdrop["x"] + 100 == 780 and backdrop["y"] + h / 2 == 580
    two = rig.text("Two")
    assert two["x"] == 680 and two["y"] == 580 - 10 - 22
    rig.step()
    assert rig.text("Two")["y"] == 580 - 10 - 22


def test_an_anchored_column_keeps_its_place_when_another_goes(tmp_path):
    # The HUD column goes away; the centred one keeps its own size and
    # takes a click right where it was drawn, on that very frame.
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ui = ctx.ui
    if not entity.state.get("hide"):
        with ui.column(width=100, anchor="top_left", margin=10):
            ui.label("HUD")
    with ui.column(width=200, anchor="center", gap=10):
        if ui.button("Hide", height=40):
            entity.state["hide"] = True
        if ui.button("Go", height=40):
            entity.state["go"] = True
""")
    rig.step()
    rig.step()
    go = (rig.text("Go")["x"], rig.text("Go")["y"])
    rig.click((rig.text("Hide")["x"], rig.text("Hide")["y"]))
    assert rig.state["hide"]
    rig.click(go)
    assert rig.state.get("go") and (rig.text("Go")["x"], rig.text("Go")["y"]) == go


def test_a_stack_overlaps_its_widgets(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    with ctx.ui.stack(10, 10, width=120, height=40):
        ctx.ui.progress(0.5)
        ctx.ui.label("HP", align="center")
""")
    rig.step()
    assert rig.rects()[0]["w"] == 120 and rig.rects()[0]["h"] == 40
    assert rig.text("HP")["x"] == 70


def test_positions_are_both_or_neither(tmp_path):
    rig = Rig(tmp_path, "def update(entity, ctx):\n    pass\n")
    rig.step()
    ui = rig.ui
    with pytest.raises(TypeError, match="inside ui.column"):
        ui.button("Lost")
    with pytest.raises(TypeError, match="both x and y"):
        ui.button("Half", 10)
    with pytest.raises(TypeError, match="both x and y"):
        ui.column(5)
    with pytest.raises(ValueError):
        ui.row(0, 0, columns=0)


# -- screens and focus --------------------------------------------------------


MENU = """
def start(entity, ctx):
    entity.state.update(clicks=[], enter_seen=0, arrows_seen=0, vol=0.5, music=False)

def update(entity, ctx):
    ui = ctx.ui
    s = entity.state
    if ctx.input.just_pressed("enter"):
        s["enter_seen"] += 1
    if ctx.input.just_pressed("down"):
        s["arrows_seen"] += 1
    if ctx.input.just_pressed("p"):
        ui.screens.show("menu", pause=True)
    if ui.screen("menu"):
        with ui.column(300, 100, width=200):
            for name in ("Resume", "Settings", "Quit"):
                if ui.button(name):
                    s["clicks"].append(name)
            s["vol"] = ui.slider(s["vol"], step=0.1)
            s["music"] = ui.toggle("Music", s["music"])
    ui.screen(None)
    if ui.button("HUD", 10, 10, 100, 40):
        s["clicks"].append("HUD")
"""


def test_arrow_keys_focus_and_enter_activates_inside_a_modal_screen(tmp_path):
    rig = Rig(tmp_path, MENU)
    rig.step()
    rig.tap("arrow_down")
    assert rig.state["arrows_seen"] == 1, "no screen: the arrows are the game's"
    assert not rig.ui.focused
    rig.tap("p")
    assert rig.host.ctx.ui.screens.paused
    rig.tap("arrow_down")  # focuses Resume
    rig.tap("arrow_down")  # Settings
    rig.tap("enter")
    assert rig.state["clicks"] == ["Settings"]
    assert rig.state["enter_seen"] == 0 and rig.state["arrows_seen"] == 1, \
        "keys the UI used are hidden from ctx.input"
    # The focus ring: four thin rects in the theme's focus colour.
    ring = api._linear(rig.ui.theme.focus)
    assert sum(1 for r in rig.rects() if r["c"] == pytest.approx(list(ring))) == 4
    rig.tap("arrow_up")
    rig.tap("arrow_up")  # wraps from Resume to the toggle at the bottom
    rig.tap("space")
    assert rig.state["music"] is True


def test_sliders_take_left_and_right_and_tab_cycles(tmp_path):
    rig = Rig(tmp_path, MENU)
    rig.step()
    rig.host.ctx.ui.screens.show("menu")
    rig.step()
    for _ in range(4):
        rig.tap("tab")  # Resume, Settings, Quit, the slider
    rig.tap("arrow_right")
    rig.tap("arrow_right")
    assert rig.state["vol"] == pytest.approx(0.7)
    rig.tap("arrow_left")
    assert rig.state["vol"] == pytest.approx(0.6)
    rig.step(keys=["shift_left", "tab"])
    rig.step()
    rig.tap("enter")
    assert rig.state["clicks"] == ["Quit"]


def test_holding_a_direction_repeats(tmp_path):
    rig = Rig(tmp_path, MENU)
    rig.step()
    rig.host.ctx.ui.screens.show("menu")
    rig.step()
    for _ in range(4):
        rig.tap("tab")
    for _ in range(60):  # 1 s: one press, then repeats every 0.08 s after 0.4 s
        rig.step(keys=["arrow_right"])
    assert rig.state["vol"] == 1.0


def test_a_modal_screen_blocks_and_covers_widgets_under_it(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ui = ctx.ui
    if ui.button("HUD", 300, 100, 200, 44):
        entity.state["hud"] = True
    if ui.screen("menu"):
        ui.button("Menu", 300, 100, 200, 44)
""")
    rig.host.ctx.ui.screens.show("menu")
    rig.step()
    labels = [i["text"] for i in rig.out.hud if i["k"] == "text"]
    assert labels == ["HUD", "Menu"]
    rig.step((400, 120), ["left"])
    rig.step((400, 120))
    assert "hud" not in rig.state
    # Drawn first but shown last: the menu's items go on top.
    rig2 = Rig(tmp_path, """
def update(entity, ctx):
    if ctx.ui.screen("menu"):
        ctx.ui.button("Menu", 0, 0)
    ctx.ui.screen(None)
    ctx.ui.button("HUD", 0, 0)
    ctx.hud.text("raw", 0, 0)
""")
    rig2.host.ctx.ui.screens.show("menu")
    rig2.step()
    assert [i["text"] for i in rig2.out.hud if i["k"] == "text"] == ["HUD", "raw", "Menu"]


def test_escape_closes_the_top_screen_and_the_script_never_sees_it(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ui = ctx.ui
    if ctx.input.just_pressed("escape"):
        ui.screens.toggle("menu")  # Escape opens the menu; the UI closes it
    for name in ui.screens.names:
        if ui.screen(name):
            ui.button(name, 0, 0)
    entity.state["names"] = ui.screens.names
""")
    rig.step()
    rig.tap("escape")
    assert rig.state["names"] == ["menu"]
    rig.host.ctx.ui.screens.show("settings")
    rig.host.ctx.ui.screens.show("locked", escape=False)
    rig.tap("escape")
    # "locked" stays, so Escape reaches the script, which hides the menu.
    assert rig.state["names"] == ["settings", "locked"]
    rig.host.ctx.ui.screens.hide("locked")
    rig.tap("pad_east")
    assert rig.state["names"] == []
    rig.tap("escape")
    assert rig.state["names"] == ["menu"], "with no screen, Escape is the game's"


def test_screens_api():
    host = api.ScriptHost(Backend(), api.KeySource())
    screens = host.ctx.ui.screens
    assert screens.top is None and not screens.paused
    screens.show("hud", modal=False)
    assert screens.modal is None and "hud" in screens
    screens.show("pause", pause=True)
    screens.show("hud", modal=False)  # moves to the top
    assert screens.names == ["pause", "hud"] and screens.modal == "pause"
    assert screens.paused and screens.top == "hud"
    assert screens.toggle("pause") is False and not screens.paused
    assert screens.back() == "hud" and screens.back() is None
    with pytest.raises(TypeError):
        screens.show("")


def test_a_screen_opened_from_the_keyboard_focuses_its_first_widget(tmp_path):
    rig = Rig(tmp_path, MENU)
    rig.step()
    rig.host.ctx.ui.screens.show("menu")
    rig.step()
    rig.tap("arrow_down")
    assert rig.ui.focused
    rig.host.ctx.ui.screens.hide("menu")
    rig.step()
    assert not rig.ui.focused, "focus goes with its screen"
    rig.tap("p")  # opened with a key, after keys were used: Resume is focused
    rig.step()
    rig.tap("enter")
    assert rig.state["clicks"] == ["Resume"]
    rig.step((5, 5), ["left"])  # a click drops keyboard focus
    rig.step((5, 5))
    assert not rig.ui.focused


def test_gamepads_navigate_and_bind_as_pad_keys(tmp_path):
    rig = Rig(tmp_path, MENU)
    rig.step()
    rig.host.ctx.ui.screens.show("menu")
    rig.step()
    rig.tap("pad_down")
    rig.tap("pad_down")
    rig.tap("pad_south")
    assert rig.state["clicks"] == ["Settings"]
    settings = rig.host.ctx.settings
    settings.bind("pause", "p", "pad_start")
    assert settings.keys("pause") == ("p", "pad_start")
    rig.keys.update(["pad_start"])
    assert rig.host.ctx.input.just_pressed("pause")
    assert rig.host.ctx.input.any_just_pressed() == "pad_start"
    with pytest.raises(ValueError, match="pad_south"):
        settings.bind("jump", "pad_a")


class FakePad:
    def __init__(self, buttons, stick=(0.0, 0.0)):
        self.buttons, self.stick = buttons, stick

    def pressed(self, name):
        return name in self.buttons

    def left_stick(self):
        return self.stick


class FakePads:
    def __init__(self, *pads):
        self.pads = pads

    def ids(self):
        return list(range(len(self.pads)))

    def get(self, i):
        return self.pads[i]


def test_engine_gamepads_become_pad_keys():
    assert _pad_keys(None) == frozenset()
    pads = FakePads(FakePad({"south", "start"}), FakePad(set(), stick=(0.9, -0.7)))
    assert _pad_keys(pads) == {"pad_south", "pad_start", "pad_right", "pad_down"}
    assert _pad_keys(FakePads(FakePad({"dpad_up", "left_trigger2"}))) == {"pad_up", "pad_lt"}
