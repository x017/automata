"""`ctx.input.touches`: fingers on a touch screen, with the left mouse
button standing in as finger 0 on a computer (in Oberon and in the engine)."""

from automata import script_api as api
from automata.scripting import _EngineKeys


class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])


SCRIPT = """
def start(entity, ctx):
    entity.state["log"] = []

def update(entity, ctx):
    entity.state["log"].append([
        (t.finger, t.position, t.start, t.just_started, t.just_ended, t.primary, t.moved)
        for t in ctx.input.touches
    ])
    if ctx.ui.button("Pause", 0, 0, 100, 50):
        entity.state["paused"] = True
"""


class Rig:
    """A host running SCRIPT, stepped with a mouse (as Oberon does)."""

    def __init__(self, tmp_path, source=None):
        self.keys = source or api.KeySource()
        path = tmp_path / "touch.py"
        path.write_text(SCRIPT, encoding="utf-8")
        self.host = api.ScriptHost(Backend(), self.keys)
        self.records = [{"id": 1, "name": "Pad", "script": str(path), "attrs": {}}]
        self.n = 0

    def step(self, pos=None, buttons=()):
        self.n += 1
        if isinstance(self.keys, api.KeySource):
            self.keys.update((), buttons, (0.0, 0.0), False, pos)
        self.host.frame(self.records, delta=1 / 60, elapsed=self.n / 60, frame=self.n,
                        screen=(800, 600))
        return self.host.entities[1].state["log"][-1]


def test_the_left_mouse_button_is_finger_zero(tmp_path):
    rig = Rig(tmp_path)
    assert rig.step((300, 300)) == []
    assert rig.step((300, 300), ["right"]) == [], "only the left button"
    assert rig.step((300, 300), ["left"]) == [
        (0, (300.0, 300.0), (300.0, 300.0), True, False, True, (0.0, 0.0))]
    assert rig.step((340, 290), ["left"]) == [
        (0, (340.0, 290.0), (300.0, 300.0), False, False, True, (40.0, -10.0))]
    # Dragged outside the view: it stays where it was last seen.
    assert rig.step(None, ["left"])[0][1] == (340.0, 290.0)
    assert rig.step((350, 280)) == [
        (0, (350.0, 280.0), (300.0, 300.0), False, True, True, (50.0, -20.0))]
    assert rig.step((350, 280)) == [], "gone the frame after it lifts"


def test_a_finger_that_lands_on_the_ui_belongs_to_the_ui(tmp_path):
    rig = Rig(tmp_path)
    rig.step((50, 25))  # the button is drawn; the UI now knows where it is
    assert rig.step((50, 25), ["left"]) == []
    assert rig.step((400, 25), ["left"]) == [], "still the UI's while held"
    assert rig.step((50, 25)) == [], "nor is its release"
    assert rig.host.entities[1].state.get("paused") is True, "released on the button: a click"
    # Back on the game: the next press is a finger again.
    assert len(rig.step((400, 300), ["left"])) == 1


class FakeEngineInput:
    """The parts of the native ``_core.Input`` the engine source reads (its
    ``Touch`` objects have the same fields as ``api.Touch``)."""

    def __init__(self):
        self.touches = []
        self.cursor = None
        self.gamepads = None
        self.mouse_delta = (0.0, 0.0)

    def pressed(self, _key):
        return False

    just_pressed = just_released = pressed

    def mouse_pressed(self, _button):
        return False

    mouse_just_pressed = mouse_just_released = mouse_pressed


class FakeScreen:
    cursor_locked = False


def test_engine_fingers_come_from_the_native_input(tmp_path):
    keys = _EngineKeys()
    raw = FakeEngineInput()
    keys.begin(type("Ctx", (), {"input": raw, "screen": FakeScreen()})())
    rig = Rig(tmp_path, keys)
    raw.touches = [api.Touch(0, (120, 400), (100, 410), primary=True),
                   api.Touch(1, (700, 500), (700, 500), just_started=True)]
    log = rig.step()
    assert [t[0] for t in log] == [0, 1]
    assert log[0][6] == (20.0, -10.0)
    assert log[1][3] and not log[1][5], "the second finger isn't the mouse"


def test_touch_reads_well():
    t = api.Touch(2, (10, 20.5), (0, 0), just_started=True)
    assert repr(t) == "Touch(finger=2, position=(10, 20.5) started)"
    assert t.moved == (10.0, 20.5)
    assert not t.just_ended and not t.primary
