"""The shared behaviour-script layer (`automata.script_api`) on its own.

The same module runs Oberon's Play mode (as `_automata_script_api.py` next
to the bridge) and `run_scene`; these tests drive it with plain dict records,
the way the bridge does.
"""

import os
import time

from automata import script_api as api


class Backend(api.Backend):
    def __init__(self):
        self.errors = []
        self.logs = []

    def error(self, report):
        self.errors.append(report)

    def log(self, level, message):
        self.logs.append((level, message))


def write(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def test_key_aliases_and_edges():
    keys = api.KeySource()
    inp = api.Input(keys)
    keys.update(["shift_right", "arrow_left"])
    assert inp.pressed("shift") and inp.pressed("Shift")
    assert inp.just_pressed("left")
    assert inp.axis("left", "right") == -1.0
    keys.update(["shift_right"])
    assert inp.just_released("left") and not inp.just_pressed("shift")


def test_writes_are_shared_and_returned(tmp_path):
    script = tmp_path / "hit.py"
    write(script, "def update(entity, ctx):\n    ctx.scene['P'].attributes['hp'] -= 1\n")
    host = api.ScriptHost(Backend())
    records = [{"id": 1, "name": "P", "attrs": {"hp": 10}}] + [
        {"id": 10 + i, "name": f"D{i}", "script": str(script)} for i in range(3)
    ]
    out = host.frame(records, delta=0.016, frame=1)
    assert out.writes == [{"id": 1, "attrs": {"hp": 7}}]


def test_errors_report_once_without_runner_frames(tmp_path):
    script = tmp_path / "bad.py"
    write(script, "def update(entity, ctx):\n    1 / 0\n")
    backend = Backend()
    host = api.ScriptHost(backend, on_error="report", internal_files=[__file__])
    for f in range(3):
        host.frame([{"id": 1, "name": "E", "script": str(script)}], frame=f + 1)
    assert len(backend.errors) == 1
    report = backend.errors[0]
    assert report["line"] == 2 and report["file"] == str(script)
    assert "ZeroDivisionError" in report["traceback"]
    assert "script_api.py" not in report["traceback"]


def test_hot_reload_picks_up_edits(tmp_path):
    script = tmp_path / "s.py"
    write(script, "def update(entity, ctx):\n    entity.attributes['v'] = 1\n")
    host = api.ScriptHost(Backend(), hot_reload=True)
    rec = [{"id": 1, "name": "E", "script": str(script)}]
    assert host.frame(rec, elapsed=0.0).writes[0]["attrs"] == {"v": 1}
    write(script, "def update(entity, ctx):\n    entity.attributes['v'] = 2\n")
    later = time.time() + 5
    os.utime(script, (later, later))
    assert host.frame(rec, elapsed=1.0).writes[0]["attrs"] == {"v": 2}


def test_engine_extras_explain_themselves(tmp_path):
    script = tmp_path / "q.py"
    write(script, "def update(entity, ctx):\n    ctx.quit()\n")
    backend = Backend()
    host = api.ScriptHost(backend, on_error="report")
    host.frame([{"id": 1, "name": "E", "script": str(script)}])
    assert "ctx.quit isn't available here" in backend.errors[0]["msg"]


def test_hud_text_items_and_validation():
    hud = api.Hud()
    hud.text("100%", 10, 20, size=26, shadow=True)
    hud.text("Hi", 5, 5, align="Centre", outline=(1, 0, 0, 1), shadow=False)
    first, second = hud._items
    assert first == {"k": "text", "text": "100%", "x": 10.0, "y": 20.0, "size": 26.0,
                     "c": [1.0, 1.0, 1.0, 1.0], "align": "left", "shadow": [0.0, 0.0, 0.0, 0.6]}
    assert second["align"] == "center" and second["outline"] == [1.0, 0.0, 0.0, 1.0]
    assert "shadow" not in second
    for bad in ({"align": "middle"}, {"size": 0}, {"size": float("nan")}):
        try:
            hud.text("x", 0, 0, **bad)
        except ValueError:
            continue
        raise AssertionError(f"accepted {bad}")
