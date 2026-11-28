"""`ctx.ui`: immediate-mode widgets drawn from scripts, and the clicks they
keep from the game.

Driven the way Oberon's bridge drives scripts: a KeySource fed buttons and
a cursor position each frame.
"""

import pytest

from automata import script_api as api


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


def write(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


class Rig:
    """A host running one script, stepped with a mouse."""

    def __init__(self, tmp_path, source):
        self.path = tmp_path / "menu.py"
        write(self.path, source)
        self.keys = api.KeySource()
        self.host = api.ScriptHost(Backend(), self.keys)
        self.records = [{"id": 1, "name": "Menu", "script": str(self.path), "attrs": {}}]
        self.n = 0

    def step(self, pos=None, buttons=(), keys=()):
        self.n += 1
        self.keys.update(keys, buttons, (0.0, 0.0), False, pos)
        out = self.host.frame(self.records, delta=1 / 60, elapsed=self.n / 60, frame=self.n,
                              screen=(800, 600))
        self.state = self.host.entities[1].state
        return out

    def click(self, pos):
        self.step(pos, ["left"])
        return self.step(pos)


BUTTON = """
def start(entity, ctx):
    entity.state.update(clicks=0, fired=0, presses=[])

def update(entity, ctx):
    if ctx.ui.button("Play", 100, 100, 200, 50):
        entity.state["clicks"] += 1
    if ctx.input.mouse_just_pressed("left"):
        entity.state["fired"] += 1
    entity.state["presses"].append(ctx.input.mouse_pressed("left"))
    entity.state["over"] = ctx.input.mouse_over_ui
"""


def test_a_button_clicks_once_on_release_and_keeps_the_click(tmp_path):
    rig = Rig(tmp_path, BUTTON)
    rig.step((10, 10))  # the button is drawn; the UI now knows where it is
    rig.step((150, 120), ["left"])
    assert rig.state["clicks"] == 0, "a click is a press and a release"
    rig.step((150, 120))
    assert rig.state["clicks"] == 1
    assert rig.state["fired"] == 0, "the game never saw the press"
    assert not any(rig.state["presses"])
    assert rig.state["over"]
    # Off the button, the game gets the mouse back.
    rig.step((600, 400), ["left"])
    assert rig.state["fired"] == 1 and rig.state["presses"][-1]
    assert rig.state["clicks"] == 1 and not rig.state["over"]


def test_pressing_then_sliding_off_cancels_and_outside_presses_dont_click(tmp_path):
    rig = Rig(tmp_path, BUTTON)
    rig.step((10, 10))
    rig.step((150, 120), ["left"])
    rig.step((500, 500), ["left"])  # dragged off while held
    rig.step((500, 500))
    assert rig.state["clicks"] == 0
    # A press that began outside is the game's, even released on the button.
    rig.step((500, 500), ["left"])
    rig.step((150, 120), ["left"])
    rig.step((150, 120))
    assert rig.state["clicks"] == 0 and rig.state["fired"] == 1


def test_the_button_draws_its_states(tmp_path):
    rig = Rig(tmp_path, BUTTON)
    rest = rig.step((10, 10)).hud
    hover = rig.step((150, 120)).hud
    held = rig.step((150, 120), ["left"]).hud
    fill = [i for i in rest if i["k"] == "rect"][1]["c"]
    assert [i for i in hover if i["k"] == "rect"][1]["c"] != fill, "hover shows"
    assert [i for i in held if i["k"] == "rect"][1]["c"] != fill, "and pressing"
    text = [i for i in rest if i["k"] == "text"][0]
    assert text["text"] == "Play" and text["align"] == "center"
    assert (text["x"], text["y"]) == (200, 125), "centred on the button"
    box = [i for i in rest if i["k"] == "rect"][0]
    assert (box["x"], box["y"], box["w"], box["h"]) == (200, 125, 200, 50), "x, y is top-left"


def test_disabled_buttons_never_click(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    if ctx.ui.button("Locked", 0, 0, 100, 40, enabled=False):
        entity.state["clicked"] = True
""")
    rig.step((50, 20))
    rig.click((50, 20))
    assert "clicked" not in rig.state


TOGGLE_SLIDER = """
def start(entity, ctx):
    entity.state.update(music=True, volume=0.5, steps=3)

def update(entity, ctx):
    s = entity.state
    s["music"] = ctx.ui.toggle("Music", s["music"], 20, 20)
    s["volume"] = ctx.ui.slider(s["volume"], 20, 80, 220, 24)
    s["steps"] = ctx.ui.slider(s["steps"], 20, 140, 220, 24, min=0, max=10, step=1, id="steps")
"""


def test_toggles_flip_when_clicked_on_the_box_or_text(tmp_path):
    rig = Rig(tmp_path, TOGGLE_SLIDER)
    rig.step()
    rig.click((30, 30))
    assert rig.state["music"] is False
    rig.click((80, 30))  # on the word "Music"
    assert rig.state["music"] is True


def test_sliders_follow_a_drag_even_off_the_track(tmp_path):
    rig = Rig(tmp_path, TOGGLE_SLIDER)
    rig.step()
    knob = 24 * 0.6
    left = 20 + knob / 2
    span = 220 - knob
    rig.step((left + span * 0.25, 90), ["left"])
    assert rig.state["volume"] == pytest.approx(0.25)
    rig.step((left + span * 0.8, 300), ["left"])  # dragged below the track
    assert rig.state["volume"] == pytest.approx(0.8)
    rig.step((2000, 300), ["left"])
    assert rig.state["volume"] == 1.0, "clamped"
    rig.step((2000, 300))
    rig.step((left, 300))  # released: moving no longer drags
    assert rig.state["volume"] == 1.0
    # Steps round to whole numbers.
    rig.step((left + span * 0.62, 150), ["left"])
    assert rig.state["steps"] == 6


def test_progress_labels_and_panels(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ctx.ui.panel(0, 0, 300, 200, title="Status")
    ctx.ui.progress(30, 10, 40, 200, 20, max=40, text=True)
    ctx.ui.label("Two\\nlines", 10, 100, size=10)
    if ctx.input.mouse_just_pressed("left"):
        entity.state["fired"] = True
""")
    out = rig.step((150, 100))
    texts = {i["text"]: i for i in out.hud if i["k"] == "text"}
    assert set(texts) == {"Status", "75%", "Two\nlines"}
    assert texts["Two\nlines"]["y"] == 110, "top at y: two 10 px lines centre 10 px lower"
    fill = [i for i in out.hud if i["k"] == "rect"][4]
    assert fill["w"] == pytest.approx((200 - 4) * 0.75)
    rig.click((150, 100))
    assert "fired" not in rig.state, "clicks on a panel are the UI's"


def test_ids_tell_apart_widgets_that_share_a_label(tmp_path):
    rig = Rig(tmp_path, """
def start(entity, ctx):
    entity.state["got"] = []

def update(entity, ctx):
    for i in range(3):
        if ctx.ui.button("Buy", 10, 10 + 60 * i, 100, 50):
            entity.state["got"].append(i)
""")
    rig.step()
    rig.click((50, 135))
    assert rig.state["got"] == [2]


def test_theme_colours_apply(tmp_path):
    rig = Rig(tmp_path, """
def update(entity, ctx):
    ctx.ui.theme.accent = (1, 0, 0, 1)
    ctx.ui.progress(1, 0, 0)
""")
    out = rig.step()
    assert [i for i in out.hud if i["k"] == "rect"][-1]["c"] == [1.0, 0.0, 0.0, 1.0]


def test_mouse_position_and_slider_validation():
    keys = api.KeySource()
    inp = api.Input(keys)
    assert inp.mouse_position is None
    keys.update([], [], (0, 0), False, (12, 34.5))
    assert inp.mouse_position == (12.0, 34.5)
    host = api.ScriptHost(Backend(), keys)
    with pytest.raises(ValueError):
        host.ctx.ui.slider(0, 0, 0, min=1, max=1)
