"""Tweens, timers and easing curves (`ctx.tween`, `ctx.after`, ...) in the
shared script layer, driven with dict records the way Oberon's bridge
drives it."""

import os
import time

import pytest

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


class World:
    """A host plus records that take each frame's writes, like a driver."""

    def __init__(self, records, on_error="raise", hot_reload=False):
        self.backend = Backend()
        self.host = api.ScriptHost(self.backend, on_error=on_error, hot_reload=hot_reload,
                                   internal_files=[__file__])
        self.records = {r["id"]: dict(r) for r in records}
        self.frame_no = 0
        self.elapsed = 0.0

    def step(self, dt=0.25, frames=1):
        out = None
        for _ in range(frames):
            self.frame_no += 1
            self.elapsed += dt
            out = self.host.frame(list(self.records.values()), delta=dt,
                                  elapsed=self.elapsed, frame=self.frame_no)
            for w in out.writes:
                if w.get("despawn"):
                    self.records.pop(w["id"], None)
                    continue
                rec = self.records[w["id"]]
                for key in ("p", "r", "s"):
                    if key in w:
                        rec[key] = list(w[key])
                if "attrs" in w:
                    rec["attrs"] = dict(w["attrs"])
        return out

    def get(self, eid, key):
        return self.records[eid].get(key)

    @property
    def ctx(self):
        return self.host.ctx


def scripted(tmp_path, body, extra=(), **kw):
    """A world with entity 1 ("Box") running a script made of ``body``."""
    script = tmp_path / "box.py"
    write(script, body)
    rec = {"id": 1, "name": "Box", "script": str(script), "p": [0, 0, 0], "r": [0, 0, 0],
           "s": [1, 1, 1], "attrs": {"score": 0, "glow": 0.0}}
    return World([rec, *extra], **kw)


def close(a, b, tol=1e-6):
    return all(abs(x - y) < tol for x, y in zip(a, b))


# --- easing ---------------------------------------------------------------


def test_there_are_31_curves_from_0_to_1():
    assert len(api.EASINGS) == 31
    for name, fn in api.EASINGS.items():
        assert fn(0.0) == pytest.approx(0.0, abs=1e-9), name
        assert fn(1.0) == pytest.approx(1.0, abs=1e-9), name


def test_curves_have_their_shapes():
    assert api.ease("linear", 0.3) == pytest.approx(0.3)
    assert api.ease("out_cubic", 0.5) == pytest.approx(0.875)
    assert api.ease("in_quad", 0.5) == pytest.approx(0.25)
    assert api.ease("in_out_sine", 0.5) == pytest.approx(0.5)
    # back and elastic overshoot; bounce stays in [0, 1].
    assert min(api.EASINGS["in_back"](i / 100) for i in range(101)) < -0.05
    assert max(api.EASINGS["out_back"](i / 100) for i in range(101)) > 1.05
    assert max(api.EASINGS["out_elastic"](i / 100) for i in range(101)) > 1.1
    bounce = [api.EASINGS["out_bounce"](i / 100) for i in range(101)]
    assert min(bounce) >= 0.0 and max(bounce) <= 1.0 + 1e-9
    # in_ and out_ mirror each other.
    for family in ("quad", "cubic", "expo", "circ", "back", "elastic", "bounce", "sine"):
        f_in, f_out = api.EASINGS[f"in_{family}"], api.EASINGS[f"out_{family}"]
        for t in (0.1, 0.35, 0.8):
            assert f_out(t) == pytest.approx(1.0 - f_in(1.0 - t), abs=1e-9), family
    # ease() clamps t.
    assert api.ease("out_cubic", 2.0) == 1.0 and api.ease("in_cubic", -1.0) == 0.0


def test_curve_names_are_forgiving_and_typos_get_suggestions():
    for name in ("easeOutCubic", "ease-out-cubic", "OutCubic", "ease_out_cubic", "out_cubic"):
        assert api.easing(name) is api.EASINGS["out_cubic"], name
    assert api.easing("InOutQuad") is api.EASINGS["in_out_quad"]
    assert api.easing("inout_quad") is api.EASINGS["in_out_quad"]
    with pytest.raises(ValueError, match="did you mean 'out_bounce'"):
        api.easing("out_bounse")
    square = lambda t: t * t  # noqa: E731
    assert api.easing(square) is square
    with pytest.raises(TypeError):
        api.easing(3)


# --- tweens -----------------------------------------------------------------


def test_a_tween_moves_the_position_and_stops(tmp_path):
    w = scripted(tmp_path, (
        "def start(entity, ctx):\n"
        "    entity.state['t'] = ctx.tween(entity, 'position', (0, 3, 0), 1.0, ease='linear')\n"
    ))
    w.step()  # start() makes it; it moves from the next frame
    assert w.get(1, "p") == [0, 0, 0]
    ys = []
    for _ in range(5):
        w.step()
        ys.append(w.get(1, "p")[1])
    assert ys == pytest.approx([0.75, 1.5, 2.25, 3.0, 3.0])
    tween = w.host.entities[1].state["t"]
    assert tween.done and tween.progress == 1.0
    assert w.step().writes == []  # nothing more to write


def test_ease_shapes_the_motion_and_whole_numbers_stay_whole(tmp_path):
    w = scripted(tmp_path, (
        "def start(entity, ctx):\n"
        "    ctx.tween(entity, 'scale', (2, 2, 2), 1.0)\n"  # out_cubic by default
        "    ctx.tween(entity, 'score', 100, 1.0, ease='linear')\n"
    ))
    w.step()
    w.step(0.5)
    assert w.get(1, "s") == pytest.approx([1.875] * 3)
    score = w.get(1, "attrs")["score"]
    assert score == 50 and isinstance(score, int)


def test_fields_by_key_and_errors_at_the_call(tmp_path):
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0], "attrs": {"hp": 3.0, "name": "x"}}])
    w.step()
    ctx = w.ctx
    t = ctx.tween("Box", "attr:hp", 0.0, 1.0)
    assert t.key == "Box:attr:hp"
    assert ctx.tween("Box", "transform:rotation_euler", (0, 1, 0), 1.0).key == \
        "Box:transform:rotation"
    with pytest.raises(KeyError, match="did you mean 'position'"):
        ctx.tween("Box", "positon", (0, 1, 0), 1.0)
    with pytest.raises(KeyError, match="no entity is called 'Bx'"):
        ctx.tween("Bx", "position", (0, 1, 0), 1.0)
    with pytest.raises(TypeError, match="numbers"):
        ctx.tween("Box", "name", "y", 1.0)
    with pytest.raises(ValueError, match="same shape"):
        ctx.tween("Box", "position", (1, 2), 1.0)
    with pytest.raises(TypeError, match="tween_value"):
        ctx.tween(5, "position", (1, 2, 3), 1.0)
    with pytest.raises(ValueError, match="duration"):
        ctx.tween("Box", "position", (1, 2, 3), -1.0)
    with pytest.raises(ValueError, match="no easing curve"):
        ctx.tween("Box", "position", (1, 2, 3), 1.0, ease="wobbly")
    with pytest.raises(ValueError, match="loops forever"):
        ctx.tween("Box", "position", (1, 2, 3), 0.0, loop=True)


def test_read_only_fields_cant_be_tweened(tmp_path):
    rec = {"id": 1, "name": "Robot", "anim": {"clip": "Run", "time": 0.0, "len": 1.0,
                                             "playing": True, "speed": 1.0, "loop": True,
                                             "clips": ["Run"], "lens": [1.0]}}
    w = World([rec])
    w.step()
    with pytest.raises(AttributeError, match="read-only"):
        w.ctx.tween("Robot", "animation:duration", 2.0, 1.0)
    assert w.ctx.tween("Robot", "animation:speed", 0.0, 1.0).key == "Robot:animation:speed"


def test_delay_start_and_on_update(tmp_path):
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}])
    w.step()
    seen = []
    w.ctx.tween("Box", "position", (4, 0, 0), 1.0, ease="linear", start=(2, 0, 0),
                delay=0.5, on_update=seen.append)
    w.step()  # 0.25 s: still waiting
    assert w.get(1, "p") == [0, 0, 0] and seen == []
    w.step()  # 0.5 s: starts, at its start
    assert w.get(1, "p") == pytest.approx([2, 0, 0])
    w.step()
    assert w.get(1, "p") == pytest.approx([2.5, 0, 0])
    assert [v[0] for v in seen] == pytest.approx([2.0, 2.5])


def test_without_start_it_begins_where_the_value_is_after_the_delay():
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}])
    w.step()
    w.ctx.tween("Box", "position", (10, 0, 0), 1.0, ease="linear", delay=0.5)
    w.records[1]["p"] = [6, 0, 0]  # something else moved it meanwhile
    w.step(frames=2)
    assert w.get(1, "p") == pytest.approx([6, 0, 0])
    w.step(0.5)
    assert w.get(1, "p") == pytest.approx([8, 0, 0])


def test_yoyo_loops_and_on_done():
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}])
    w.step()
    done = []
    t = w.ctx.tween("Box", "position", (0, 1, 0), 1.0, ease="linear", yoyo=True, loop=2,
                    on_done=lambda: done.append(w.frame_no))
    ys = []
    for _ in range(9):
        w.step()
        ys.append(w.get(1, "p")[1])
    assert ys == pytest.approx([0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25, 0.0, 0.0])
    assert done == [9] and t.done  # once, on the frame it ended


def test_looping_forever_restarts_from_the_start():
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}])
    w.step()
    t = w.ctx.tween("Box", "position", (0, 1, 0), 1.0, ease="linear", loop=True)
    ys = []
    for _ in range(6):
        w.step()
        ys.append(w.get(1, "p")[1])
    assert ys == pytest.approx([0.25, 0.5, 0.75, 0.0, 0.25, 0.5])
    assert t.active and t.progress == pytest.approx(0.5)
    t.cancel()
    w.step()
    assert w.get(1, "p")[1] == pytest.approx(0.5)  # stays where it stopped


def test_finish_jumps_to_the_end_and_a_new_tween_replaces_the_old():
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0], "s": [1, 1, 1]}])
    w.step()
    ctx = w.ctx
    done = []
    first = ctx.tween("Box", "position", (5, 0, 0), 1.0, on_done=lambda: done.append(1))
    w.step()
    second = ctx.tween("Box", "position", (0, 0, 9), 1.0, ease="linear")
    assert first.done and not second.done
    other = ctx.tween("Box", "scale", (2, 2, 2), 1.0)
    w.step()
    assert not other.done  # a different field isn't replaced
    second.finish()  # (from a script, this write applies with the frame's)
    assert second.done and second.value == pytest.approx((0, 0, 9))
    assert w.host.entities[1].transform.position == pytest.approx((0, 0, 9))
    assert done == []  # cancelled ones don't call on_done


def test_tween_value_for_your_own_numbers():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    fade = w.ctx.tween_value(1.0, 0.0, 1.0, ease="linear")
    color = w.ctx.tween_value((1, 0, 0), (0, 0, 1), 0.5, ease="linear")
    assert fade.value == 1.0
    w.step()
    assert fade.value == pytest.approx(0.75)
    assert color.value == pytest.approx((0.5, 0, 0.5))
    w.step(frames=4)
    assert fade.value == pytest.approx(0.0) and fade.done
    with pytest.raises(ValueError, match="same shape"):
        w.ctx.tween_value(1.0, (0, 1), 1.0)


def test_a_tween_ends_when_its_entity_goes(tmp_path):
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}, {"id": 2, "name": "Other"}])
    w.step()
    t = w.ctx.tween("Box", "position", (0, 9, 0), 1.0)
    w.step()
    del w.records[1]
    w.step()
    assert t.done and w.host._timers == []


def test_coin_pops_then_despawns(tmp_path):
    w = scripted(tmp_path, (
        "def update(entity, ctx):\n"
        "    if ctx.time.frame == 2:\n"
        "        ctx.tween(entity, 'scale', (0, 0, 0), 0.5, ease='in_back',\n"
        "                  on_done=entity.despawn)\n"
    ))
    w.step(frames=3)
    assert 1 in w.records
    w.step()
    assert 1 not in w.records


# --- timers ---------------------------------------------------------------


def test_after_fires_once_on_time():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    calls = []
    t = w.ctx.after(0.6, calls.append, "ding")
    w.step(0.25, frames=2)
    assert calls == [] and t.remaining == pytest.approx(0.1)
    w.step(0.25)
    assert calls == ["ding"] and t.done and t.count == 1
    w.step(0.25, frames=4)
    assert calls == ["ding"]


def test_after_zero_fires_next_frame():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    calls = []
    w.ctx.after(0.0, calls.append, 1)
    assert calls == []
    w.step(0.0)
    assert calls == [1]


def test_every_repeats_counts_and_cancels():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    ticks = []
    t = w.ctx.every(0.5, lambda: ticks.append(w.frame_no))
    w.step(0.25, frames=8)
    assert ticks == [3, 5, 7, 9] and t.count == 4
    t.cancel()
    w.step(0.25, frames=4)
    assert len(ticks) == 4
    limited = []
    w.ctx.every(0.25, limited.append, "x", times=3)
    w.step(0.25, frames=6)
    assert limited == ["x"] * 3


def test_every_catches_up_but_not_forever():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    ticks = []
    w.ctx.every(0.1, ticks.append, 1)
    w.step(0.35)
    assert len(ticks) == 3  # three periods passed
    ticks.clear()
    w.step(30.0)  # a long hitch
    assert len(ticks) == 8


def test_pause_holds_a_timer():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    calls = []
    t = w.ctx.after(0.5, calls.append, 1)
    t.pause()
    w.step(frames=4)
    assert calls == [] and t.remaining == pytest.approx(0.5)
    t.resume()
    w.step(frames=2)
    assert calls == [1]


def test_bad_timer_arguments():
    w = World([{"id": 1, "name": "Box"}])
    w.step()
    with pytest.raises(ValueError, match="above 0"):
        w.ctx.every(0.0, print)
    with pytest.raises(ValueError):
        w.ctx.after(-1.0, print)
    with pytest.raises(TypeError, match="function"):
        w.ctx.after(1.0, "not callable")
    with pytest.raises(ValueError, match="times"):
        w.ctx.every(1.0, print, times=0)


def test_a_script_cancels_its_timer_from_the_callback(tmp_path):
    w = scripted(tmp_path, (
        "def tick(entity):\n"
        "    entity.attributes['score'] += 1\n"
        "    if entity.attributes['score'] == 3:\n"
        "        entity.state['timer'].cancel()\n"
        "\n"
        "def start(entity, ctx):\n"
        "    entity.state['timer'] = ctx.every(0.25, tick, entity)\n"
    ))
    w.step(frames=10)
    assert w.get(1, "attrs")["score"] == 3


def test_timers_end_with_the_entity_whose_script_made_them(tmp_path):
    w = scripted(tmp_path, (
        "def start(entity, ctx):\n"
        "    ctx.every(0.25, lambda: ctx.scene['Counter'].attributes.update(n=1))\n"
        "    ctx.tween('Counter', 'position', (5, 0, 0), 4.0)\n"
    ), extra=[{"id": 2, "name": "Counter", "p": [0, 0, 0], "attrs": {"n": 0}}])
    w.step(frames=2)
    assert w.get(2, "attrs")["n"] == 1
    assert len(w.host._timers) == 2
    del w.records[1]
    w.step()
    assert w.host._timers == []


def test_a_failing_callback_reports_and_pauses_its_script(tmp_path):
    w = scripted(tmp_path, (
        "def boom():\n"
        "    raise RuntimeError('kaboom')\n"
        "\n"
        "def start(entity, ctx):\n"
        "    ctx.every(0.25, boom)\n"
    ), on_error="report")
    w.step(frames=6)
    assert len(w.backend.errors) == 1
    report = w.backend.errors[0]
    assert "kaboom" in report["msg"] and "every (Box)" in report["msg"]
    assert report["line"] == 2


def test_timer_callbacks_follow_hot_reloads(tmp_path):
    w = scripted(tmp_path, (
        "def tick(entity):\n"
        "    entity.attributes['score'] = 1\n"
        "\n"
        "def start(entity, ctx):\n"
        "    ctx.every(0.25, tick, entity)\n"
    ), hot_reload=True)
    w.step(frames=2)
    assert w.get(1, "attrs")["score"] == 1
    write(tmp_path / "box.py", (
        "def tick(entity):\n"
        "    entity.attributes['score'] = 2\n"
        "\n"
        "def start(entity, ctx):\n"
        "    ctx.every(0.25, tick, entity)\n"
    ))
    later = time.time() + 5
    os.utime(tmp_path / "box.py", (later, later))
    w.step(frames=3)
    assert w.get(1, "attrs")["score"] == 2
    assert len(w.host._timers) == 1  # start() didn't run again


def test_stop_ends_everything():
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}])
    w.step()
    t = w.ctx.after(1.0, print)
    tw = w.ctx.tween("Box", "position", (1, 1, 1), 1.0)
    w.host.stop()
    assert t.done and tw.done and w.host._timers == []


def test_reprs():
    w = World([{"id": 1, "name": "Box", "p": [0, 0, 0]}])
    w.step()
    assert repr(w.ctx.after(1.0, print)).startswith("Timer(after 1s, print")
    assert "Box:transform:position" in repr(w.ctx.tween("Box", "position", (1, 1, 1), 1.0))
