"""`ctx.input.touch_controls`: on-screen thumbsticks and buttons that hold
keys, so keyboard games play on a touch screen."""

import pytest

from automata import script_api as api


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


SCRIPT = """
def start(entity, ctx):
    entity.state["log"] = []
    ctx.settings.bind("right", "d")
    ctx.settings.bind("left", "a")
    ctx.settings.bind("jump", "space")

def update(entity, ctx):
    i = ctx.input
    entity.state["log"].append({
        "d": i.pressed("d"),
        "right": i.pressed("right"),
        "axis": i.axis("left", "right"),
        "keys_axis": i.axis("a", "d"),
        "jump": (i.just_pressed("jump"), i.pressed("jump"), i.just_released("jump")),
        "space": i.pressed("space"),
        "touches": [t.finger for t in i.touches],
        "mouse": i.mouse_pressed("left"),
        "delta": i.mouse_delta,
        "stick": i.touch_controls.stick("left"),
        "clicked": ctx.ui.button("Menu", 40, 500, 120, 60),
    })
"""


class Fingers(api.KeySource):
    """A touch screen: ``fingers`` are ``(finger, x, y)``; the first one
    down is the primary (the left mouse button)."""

    def __init__(self):
        super().__init__()
        self.now = {}  # finger -> (x, y, start, primary)
        self.ended = {}

    def put(self, fingers):
        old = self.now
        self.now = {}
        for f, x, y in fingers:
            start = old[f][2] if f in old else (x, y)
            primary = old[f][3] if f in old else not any(p for *_, p in old.values())
            self.now[f] = (x, y, start, primary)
        self.ended = {f: v for f, v in old.items() if f not in self.now}
        self.started = {f for f in self.now if f not in old}
        primary = [v for v in self.now.values() if v[3]]
        pos = primary[0][:2] if primary else None
        last = [v for v in self.ended.values() if v[3]]
        self.update((), ["left"] if primary else [], (0.0, 0.0), False,
                    pos or (last[0][:2] if last else None))

    def touches(self):
        out = [(f, x, y, *s, f in self.started, False, p) for f, (x, y, s, p) in self.now.items()]
        out += [(f, x, y, *s, False, True, p) for f, (x, y, s, p) in self.ended.items()]
        return out


CONTROLS = {
    "sticks": [{"side": "left", "keys": ["w", "a", "s", "d"]}],
    "buttons": [{"key": "jump", "label": "Jump"}, {"key": "e", "label": "Use"},
                {"key": "space", "label": "Space"}],
    "show": "always",
}


class Rig:
    def __init__(self, tmp_path, controls=CONTROLS):
        self.src = Fingers()
        path = tmp_path / "pad.py"
        path.write_text(SCRIPT, encoding="utf-8")
        self.host = api.ScriptHost(Backend(), self.src, touch_controls=controls)
        self.records = [{"id": 1, "name": "Pad", "script": str(path), "attrs": {}}]
        self.n = 0
        self.hud = []

    def step(self, *fingers):
        self.n += 1
        self.src.put(fingers)
        out = self.host.frame(self.records, delta=1 / 60, elapsed=self.n / 60, frame=self.n,
                              screen=(800, 600))
        self.hud = out.hud
        return self.host.entities[1].state["log"][-1]

    @property
    def controls(self):
        return self.host.ctx.input.touch_controls


@pytest.fixture(autouse=True)
def desktop(monkeypatch):
    monkeypatch.delenv("AUTOMATA_TOUCH_CONTROLS", raising=False)
    monkeypatch.delenv("ANDROID_PRIVATE", raising=False)


def test_a_stick_holds_keys_and_axis_reads_how_far(tmp_path):
    rig = Rig(tmp_path)
    log = rig.step((0, 150, 450))
    assert log["touches"] == [] and not log["mouse"], "the finger is the stick's"
    assert log["axis"] == 0.0
    r = 0.15 * 600  # the stick's reach on an 800×600 screen
    log = rig.step((0, 150 + r, 450))
    assert log["d"] and log["right"], "a key and the action bound to it"
    assert log["axis"] == pytest.approx(1.0) and log["keys_axis"] == pytest.approx(1.0)
    assert log["stick"] == pytest.approx((1.0, 0.0))
    log = rig.step((0, 150 + r / 2, 450 - r / 2))
    assert log["axis"] == pytest.approx(0.5), "pushed half way: half speed"
    assert log["stick"] == pytest.approx((0.5, 0.5)), "+y is up"
    log = rig.step((0, 150 + 0.3 * r, 450))
    assert log["axis"] == pytest.approx(0.3)
    assert not log["d"], "too gentle to hold the key"
    log = rig.step((0, 150 + 3 * r, 450))
    assert log["stick"] == pytest.approx((1.0, 0.0)), "it stops at its edge"
    log = rig.step()
    assert log["axis"] == 0.0 and not log["d"]


def test_buttons_hold_their_key_while_touched(tmp_path):
    rig = Rig(tmp_path)
    corner = (800 - 30 - 0.085 * 600 * 1.2, 600 - 30 - 0.085 * 600 * 1.2)
    rig.step()  # the script binds its actions
    log = rig.step((0, *corner))
    assert log["jump"] == (True, True, False), "the first button: in the corner"
    assert log["space"], "an action holds the keys bound to it"
    assert log["touches"] == [] and not log["mouse"] and not log["clicked"]
    log = rig.step((0, corner[0] - 5, corner[1]))
    assert log["jump"] == (False, True, False)
    log = rig.step()
    assert log["jump"] == (False, False, True)
    # A second finger works one while the first is on the stick.
    rig.step((0, 150, 450))
    log = rig.step((0, 150 + 90, 450), (1, *corner))
    assert log["d"] and log["jump"][0]
    assert log["touches"] == []


def test_a_button_holding_a_key_presses_the_actions_bound_to_it(tmp_path):
    rig = Rig(tmp_path)
    rig.step()
    third = rig.controls._buttons[2]
    log = rig.step((0, *third.center))
    assert log["space"] and log["jump"] == (True, True, False)


def test_a_tap_shorter_than_a_frame_still_presses(tmp_path):
    rig = Rig(tmp_path)
    rig.step()
    src = rig.src
    corner = (800 - 30 - 61.2, 600 - 30 - 61.2)
    # The finger lands and lifts between two frames.
    src.update((), [], (0.0, 0.0), False, corner)
    src.touches = lambda: [(3, *corner, *corner, True, True, False)]
    rig.n += 1
    rig.host.frame(rig.records, delta=1 / 60, elapsed=rig.n / 60, frame=rig.n, screen=(800, 600))
    assert rig.host.entities[1].state["log"][-1]["jump"][0]


def test_other_fingers_and_the_mouse_still_reach_the_game(tmp_path):
    rig = Rig(tmp_path)
    log = rig.step((0, 400, 100))  # the top middle: no control there
    assert log["touches"] == [0] and log["mouse"]
    log = rig.step((0, 400, 100), (1, 150, 450))
    assert log["touches"] == [0], "the second finger is on the stick"
    assert log["mouse"]


def test_the_ui_keeps_fingers_that_land_on_it(tmp_path):
    rig = Rig(tmp_path)
    rig.step()  # the menu button is drawn; the UI now knows where it is
    log = rig.step((0, 100, 530))  # on the button, inside the stick's half
    assert log["stick"] == (0.0, 0.0) and log["axis"] == 0.0
    log = rig.step((0, 150, 530))
    assert log["axis"] == 0.0, "still the UI's while held"
    assert rig.step()["clicked"], "released on it: a click"


def test_a_button_can_hold_a_mouse_button(tmp_path):
    rig = Rig(tmp_path, {"buttons": [{"key": "mouse_left", "label": "Fire"},
                                     {"key": "shoot"}], "show": "always"})
    rig.step()
    rig.host.ctx.settings.bind("shoot", "mouse_right")
    fire, shoot = rig.controls._buttons
    log = rig.step((0, *fire.center))
    assert log["mouse"], "Fire is the left mouse button"
    assert log["touches"] == []
    rig.step()
    rig.step((0, *shoot.center))
    i = rig.host.ctx.input
    assert i.mouse_pressed("right") and i.pressed("shoot") and not i.mouse_pressed("left")


def test_look_drags_move_the_mouse_delta(tmp_path):
    rig = Rig(tmp_path, {"look": "right", "show": "always"})
    rig.step((0, 600, 300))
    log = rig.step((0, 630, 290))
    assert log["delta"] == pytest.approx((30.0, -10.0))
    assert log["touches"] == []
    assert rig.step((0, 630, 290))["delta"] == (0.0, 0.0)
    assert rig.step((0, 400, 300))["delta"] == pytest.approx((-230.0, 10.0))
    log = rig.step((0, 100, 300))
    log = rig.step((1, 100, 300))
    assert log["delta"] == (0.0, 0.0), "the left side doesn't look"


def test_they_show_on_touch_screens_unless_told(tmp_path, monkeypatch):
    rig = Rig(tmp_path, {**CONTROLS, "show": "auto"})
    log = rig.step((0, 150, 450))
    assert not rig.controls.visible, "a computer: the mouse is the mouse"
    assert log["touches"] == [0] and log["mouse"]
    assert not any(item.get("path", "").startswith("builtin:") for item in rig.hud)
    monkeypatch.setenv("AUTOMATA_TOUCH_CONTROLS", "1")
    rig.step()
    assert rig.controls.visible
    log = rig.step((0, 150, 450))
    assert log["touches"] == []
    assert any(item.get("path") == "builtin:ring" for item in rig.hud)
    assert any(item.get("text") == "Jump" for item in rig.hud)
    monkeypatch.setenv("AUTOMATA_TOUCH_CONTROLS", "0")
    assert not rig.controls.visible
    monkeypatch.setenv("ANDROID_PRIVATE", "/data/app")
    monkeypatch.delenv("AUTOMATA_TOUCH_CONTROLS")
    assert rig.controls.visible, "Android"
    rig.controls.visible = False
    assert not rig.controls.visible, "a script's choice wins"
    log = rig.step((0, 150, 450))
    assert log["touches"] == [0]


def test_hiding_them_lets_go_of_their_keys(tmp_path):
    rig = Rig(tmp_path)
    rig.step((0, 150, 450))
    assert rig.step((0, 240, 450))["d"]
    rig.controls.visible = False
    log = rig.step((0, 240, 450))
    assert not log["d"] and log["axis"] == 0.0


def test_scripts_can_set_them_up(tmp_path):
    rig = Rig(tmp_path, None)
    assert not rig.controls.configured and not rig.controls.visible
    rig.controls.set(sticks=[{"side": "right", "keys": ["a", "d"]}], show="always")
    rig.step((0, 600, 450))
    log = rig.step((0, 600 - 90, 450))
    assert log["keys_axis"] == pytest.approx(-1.0)
    assert rig.controls.stick("right") == pytest.approx((-1.0, 0.0))
    assert "visible=True" in repr(rig.controls)


@pytest.mark.parametrize("kwargs, message", [
    ({"sticks": [{"keys": ["w", "a", "s"]}]}, "four keys"),
    ({"sticks": [{"keys": ["a", "d"]}, {"keys": ["w", "a", "s", "d"]}]}, "already a stick"),
    ({"sticks": [{"side": "up", "keys": ["a", "d"]}]}, '"left" or "right"'),
    ({"buttons": [{"label": "Jump"}]}, "needs a key"),
    ({"buttons": [{"key": str(i)} for i in range(7)]}, "up to 6"),
    ({"sticks": [{"side": "right", "keys": ["a", "d"]}], "look": "right"}, "share"),
    ({"show": "sometimes"}, '"auto", "always" or "never"'),
])
def test_bad_settings_say_what_is_wrong(kwargs, message):
    controls = api.TouchControls(None)
    with pytest.raises(ValueError, match=message):
        controls.set(**kwargs)


def test_the_player_sets_them_up_from_game_json(tmp_path, monkeypatch):
    import json

    at = pytest.importorskip("automata")
    (tmp_path / "game.py").write_text(
        "import json\n"
        "def update(entity, ctx):\n"
        "    t = ctx.input.touch_controls\n"
        "    with open('seen.json', 'w') as f:\n"
        "        json.dump([t.configured, t.visible, repr(t)], f)\n")
    entity = at.Entity("Pad").add(at.Transform())
    entity.script = "game.py"
    at.Scene().add(entity).save(str(tmp_path / "level.ron"))
    (tmp_path / "game.json").write_text(json.dumps({
        "name": "Touch Test", "start_scene": "level.ron", "resolution": [320, 240],
        "touch_controls": {"sticks": [{"side": "left", "keys": ["w", "a", "s", "d"]}],
                           "buttons": [{"key": "space", "label": "Jump"}],
                           "look": None, "show": "always"},
    }))
    monkeypatch.setenv("AUTOMATA_SAVE_DIR", str(tmp_path / "saves"))
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(api.sys, "path", list(api.sys.path))
    from automata import player

    player.main([str(tmp_path), "--headless", "--frames", "2"])
    configured, visible, text = json.loads((tmp_path / "seen.json").read_text())
    assert configured and visible
    assert "sticks=1, buttons=1" in text
