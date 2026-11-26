"""Attachments and IK from Python: ``at.Attach``, ``Animation(foot_ik=)``,
``view.attach`` / ``detach`` and ``view.animation.look_at`` / ``reach``."""

import math
from pathlib import Path

import pytest

import automata as at

ROBOT = Path(__file__).resolve().parents[4] / "examples/oberon/robot_plaza/models/robot.glb"


def robot(name="bot", position=(0.0, 0.0, 0.0), foot_ik=False):
    return (
        at.Entity(name)
        .add(at.Transform(position=position))
        .add(at.MeshRenderer.model(str(ROBOT)))
        .add(at.Animation("Idle", foot_ik=foot_ik))
    )


def hat():
    return (
        at.Entity("hat")
        .add(at.Transform(position=(0.0, 0.5, 0.0), scale=(0.3, 0.3, 0.3)))
        .add(at.MeshRenderer.cube(size=1.0))
        .add(at.Attach("bot", "Head"))
    )


def test_attach_and_foot_ik_are_saved():
    attach = at.Attach("bot", "Head")
    assert (attach.to, attach.bone) == ("bot", "Head")
    assert at.Attach("bot").bone == ""
    assert attach == at.Attach("bot", "Head") and attach != at.Attach("bot")
    assert at.Animation("Idle", foot_ik=True).foot_ik
    assert not at.Animation("Idle").foot_ik
    text = at.Scene().add(robot(foot_ik=True)).add(hat()).to_ron()
    assert "foot_ik: true" in text and "attach:" in text
    loaded = at.Scene.from_ron(text)
    components = [c for e in loaded.entities() for c in e.components()]
    (got,) = [c for c in components if isinstance(c, at.Attach)]
    assert (got.to, got.bone) == ("bot", "Head")
    (anim,) = [c for c in components if isinstance(c, at.Animation)]
    assert anim.foot_ik


def test_a_hat_follows_the_head_until_detached():
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(at.Scene().add(robot()).add(hat()))
    seen = {}

    def update(ctx):
        bot, h, frame = ctx.scene["bot"], ctx.scene["hat"], ctx.time.frame
        if frame == 2:
            seen["on"] = (h.attached_to, h.transform.position)
            bot.transform.position = (3.0, 0.0, 0.0)
        if frame == 4:
            seen["moved"] = h.transform.position
            h.detach()
        if frame == 6:
            seen["off"] = (h.attached_to, h.transform.position)
            bot.transform.position = (-3.0, 0.0, 0.0)
        if frame == 8:
            seen["left"] = h.transform.position
            h.attach("bot", "Palm2.R", at.Transform(position=(0.0, 0.1, 0.0)))
        if frame == 10:
            seen["hand"] = h.attached_to

    game.run(update, max_frames=11)
    attached, (x, y, z) = seen["on"]
    assert attached == ("bot", "Head")
    assert abs(x) < 0.3 and y > 0.5, "above the robot's middle"
    assert seen["moved"][0] == pytest.approx(x + 3.0, abs=0.05), "it went with the robot"
    assert seen["off"][0] is None
    assert seen["left"] == pytest.approx(seen["off"][1]), "let go, it stays put"
    assert seen["hand"] == ("bot", "Palm2.R")


def test_heads_look_and_hands_reach():
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(at.Scene().add(robot(foot_ik=True)).add(robot("friend", (2.0, 0.0, 2.0))))
    seen = {}

    def update(ctx):
        anim, frame = ctx.scene["bot"].animation, ctx.time.frame
        if frame == 1:
            seen["start"] = (anim.foot_ik, anim.looking, anim.reaching)
            with pytest.raises(TypeError):
                anim.look_at(3.5)
            with pytest.raises(ValueError):
                anim.reach("Palm2.R", (math.inf, 0.0, 0.0))
            anim.look_at("friend")
            anim.reach("Palm2.R", (0.5, 1.0, 0.5), weight=0.5)
            anim.foot_ik = False
        if frame == 3:
            seen["set"] = (anim.foot_ik, anim.looking, anim.reaching)
            anim.look_at(None)
            anim.reach("Palm2.R", None)
        if frame == 5:
            seen["stopped"] = (anim.looking, anim.reaching)

    game.run(update, max_frames=6)
    assert seen["start"] == (True, False, [])
    assert seen["set"] == (False, True, ["Palm2.R"])
    assert seen["stopped"] == (False, [])
