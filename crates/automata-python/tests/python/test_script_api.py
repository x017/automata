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


def _anim(events, clip="Run"):
    return {"clip": clip, "time": 0.0, "len": 1.0, "playing": True, "speed": 1.0,
            "loop": True, "clips": [clip], "lens": [1.0], "rm": False, "ev": events}


def test_channels_match_exactly_or_with_wildcards():
    host = api.ScriptHost(Backend())
    got = []
    host.ctx.subscribe("Player:hit", lambda m: got.append(("exact", m.channel, m.data)))
    host.ctx.subscribe("*:hit", lambda m: got.append(("any", m.channel, m.data)))
    host.ctx.subscribe("Enemy_?:*", lambda m: got.append(("enemy", m.channel, m.data)))
    assert host.ctx.publish("Player:hit", 5) == 2
    assert host.ctx.publish("Enemy_1:hit", 1) == 2
    assert host.ctx.publish("player:hit") == 1, "names are case-sensitive"
    assert host.ctx.publish("Enemy_12:hit") == 1
    assert got[:2] == [("exact", "Player:hit", 5), ("any", "Player:hit", 5)]
    assert ("enemy", "Enemy_1:hit", 1) in got


def test_subscribing_again_is_harmless_and_cancel_stops_delivery(tmp_path):
    script = tmp_path / "listener.py"
    write(script, (
        "def heard(msg):\n"
        "    msg.data.append('heard')\n"
        "\n"
        "def update(entity, ctx):\n"
        "    ctx.subscribe('bell', heard)\n"
    ))
    host = api.ScriptHost(Backend())
    rec = [{"id": 1, "name": "L", "script": str(script)}]
    for f in range(3):
        host.frame(rec, frame=f)
    log = []
    assert host.ctx.publish("bell", log) == 1 and log == ["heard"]
    (sub,) = host._subs
    assert sub.owner.name == "L" and "heard" in repr(sub)
    assert host.ctx.unsubscribe(sub) == 1 and host.ctx.publish("bell", log) == 0
    # The entity leaving ends its subscriptions.
    host.frame(rec, frame=4)
    assert len(host._subs) == 1
    host.frame([], frame=5)
    assert host._subs == []


def test_subscribers_follow_hot_reloads_and_errors_blame_their_script(tmp_path):
    script = tmp_path / "s.py"
    write(script, (
        "def heard(msg):\n"
        "    msg.data.append(1)\n"
        "\n"
        "def start(entity, ctx):\n"
        "    ctx.subscribe('ping', heard)\n"
    ))
    backend = Backend()
    host = api.ScriptHost(backend, on_error="report", hot_reload=True)
    rec = [{"id": 1, "name": "E", "script": str(script)}]
    host.frame(rec, elapsed=0.0)
    log = []
    host.ctx.publish("ping", log)
    write(script, (
        "def heard(msg):\n"
        "    msg.data.append(2)\n"
        "    1 / 0\n"
        "\n"
        "def start(entity, ctx):\n"
        "    ctx.subscribe('ping', heard)\n"
    ))
    later = time.time() + 5
    os.utime(script, (later, later))
    host.frame(rec, elapsed=1.0)
    host.ctx.publish("ping", log)
    host.ctx.publish("ping", log)
    assert log == [1, 2], "the new version runs, then pauses with its script"
    (report,) = backend.errors
    assert report["file"] == str(script) and report["line"] == 3
    assert "subscriber of 'ping' (E)" in report["msg"]


def test_animation_events_reach_on_animation_event_and_channels(tmp_path):
    script = tmp_path / "steps.py"
    write(script, (
        "def start(entity, ctx):\n"
        "    entity.state['heard'] = []\n"
        "\n"
        "def on_animation_event(entity, ctx, name):\n"
        "    entity.state['heard'].append(name)\n"
    ))
    host = api.ScriptHost(Backend())
    channel = []
    host.ctx.subscribe("*:animation:*", lambda m: channel.append(
        (m.channel, m.entity.name, m.event, m.clip)))

    def frame(events, f):
        host.frame([{"id": 3, "name": "Runner", "script": str(script), "anim": _anim(events)}],
                   frame=f)

    frame([], 0)
    frame([[1, "Run", "footstep"]], 1)
    # Events stay in the record for a while; each is delivered once.
    frame([[1, "Run", "footstep"], [2, "Run", "footstep"], [3, "Run", "end"]], 2)
    frame([[1, "Run", "footstep"], [2, "Run", "footstep"], [3, "Run", "end"]], 3)
    runner = host.ctx.scene["Runner"]
    assert runner.state["heard"] == ["footstep", "footstep", "end"]
    assert channel[0] == ("Runner:animation:footstep", "Runner", "footstep", "Run")
    assert [c[0] for c in channel] == [
        "Runner:animation:footstep", "Runner:animation:footstep", "Runner:animation:end"]
    # A fresh animator (its numbers start over) is noticed.
    frame([[1, "Run", "jump"]], 4)
    assert runner.state["heard"][-1] == "jump"


def test_root_motion_reads_and_writes():
    host = api.ScriptHost(Backend())
    host.frame([{"id": 1, "name": "R", "anim": _anim([])}])
    anim = host.ctx.scene["R"].animation
    assert anim.root_motion is False
    anim.root_motion = True
    assert anim.root_motion is True
    out = host._collect()
    assert out.writes == [{"id": 1, "anim": [["root_motion", True]]}]
