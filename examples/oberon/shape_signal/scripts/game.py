"""Shape Signal's rules. The screens are UI documents made in Oberon's UI
editor (``ui/title.ui.ron``, ``ui/hud.ui.ron`` and ``ui/results.ui.ron``):
this script doesn't draw anything. It fills the documents in and reacts to
their widgets by the names given in the editor::

    hud = ctx.ui.screen("hud")
    hud.set("score", "SCORE 120")          # a label's text
    hud.set("lives", 2)                     # a progress bar's value
    if hud.clicked("pick_1"): ...           # a button

Open ``ui/hud.ui.ron`` in Oberon to move things around, restyle them or
change their text; as long as the names stay, this script keeps working.
"""

import math
import random

SHAPES = ("Cube", "Tower", "Slab")
ROUND = 45.0  # seconds a round lasts
LIVES = 3
ANSWER = 2.6  # seconds to pick the lifted shape before it counts as missed
PAUSE = 0.45  # seconds between signals


def start(entity, ctx):
    s = entity.state
    s["mode"] = "title"
    s["best"] = int(ctx.settings.get("best", 0))
    s["rng"] = random.Random(entity.attributes.get("seed", 1))
    s.update(score=0, streak=0, right=0, missed=0, lives=LIVES, left=ROUND,
             lit=None, wait=PAUSE, timer=0.0, verdict_for=0.0)
    title = ctx.ui.screen("title")
    title.set("volume", ctx.settings.volume)
    title.set("hints", bool(ctx.settings.get("hints", True)))
    show_title(entity, ctx)


def update(entity, ctx):
    s = entity.state
    if s["mode"] == "title":
        title_screen(entity, ctx)
    elif s["mode"] == "playing":
        play(entity, ctx)
    else:
        results_screen(entity, ctx)
    entity.attributes["lit"] = s["lit"] or ""
    entity.attributes["mode"] = s["mode"]


# -- screens --------------------------------------------------------------------


def show_title(entity, ctx):
    s = entity.state
    s["mode"], s["lit"] = "title", None
    title = ctx.ui.screen("title")
    title.set("best", f"Best {s['best']}")
    title.set("volume_text", f"{round(ctx.settings.volume * 100)}%")
    ctx.ui.screen("hud").visible = False
    ctx.ui.screens.clear()
    ctx.ui.screens.show("title")


def title_screen(entity, ctx):
    title = ctx.ui.screen("title")
    if title.changed("volume"):
        ctx.settings.volume = title.get("volume")
        title.set("volume_text", f"{round(ctx.settings.volume * 100)}%")
    if title.changed("hints"):
        ctx.settings["hints"] = title.get("hints")
    if title.clicked("play"):
        new_round(entity, ctx)


def new_round(entity, ctx):
    s = entity.state
    s.update(mode="playing", score=0, streak=0, right=0, missed=0, lives=LIVES,
             left=ROUND, lit=None, wait=PAUSE, timer=0.0, verdict_for=0.0)
    hud = ctx.ui.screen("hud")
    hud.visible = True
    hud.set_visible("keys", bool(ctx.settings.get("hints", True)))
    hud.set("verdict", "")
    ctx.ui.screens.clear()


def finish(entity, ctx):
    s = entity.state
    s["mode"], s["lit"] = "results", None
    if s["score"] > s["best"]:
        s["best"] = s["score"]
        ctx.settings["best"] = s["score"]
    results = ctx.ui.screen("results")
    results.set("heading", "TIME!" if s["lives"] > 0 else "OUT!")
    results.set("final", f"SCORE {s['score']}")
    results.set("detail", f"{s['right']} right, {s['missed']} missed")
    results.set("best", f"Best {s['best']}")
    ctx.ui.screens.show("results")


def results_screen(entity, ctx):
    results = ctx.ui.screen("results")
    if results.clicked("again"):
        new_round(entity, ctx)
    elif results.clicked("title"):
        show_title(entity, ctx)


# -- the round -------------------------------------------------------------------


def play(entity, ctx):
    s = entity.state
    hud = ctx.ui.screen("hud")
    dt = ctx.time.delta
    s["left"] -= dt
    s["verdict_for"] -= dt
    if s["lit"] is None:
        s["wait"] -= dt
        if s["wait"] <= 0:
            s["lit"] = s["rng"].choice(SHAPES)
            s["timer"] = ANSWER
    else:
        s["timer"] -= dt
        pick = None
        for i in range(len(SHAPES)):
            if hud.clicked(f"pick_{i + 1}") or ctx.input.just_pressed(str(i + 1)):
                pick = SHAPES[i]
        if pick == s["lit"]:
            s["streak"] += 1
            s["right"] += 1
            gained = 10 + 5 * min(s["streak"] - 1, 4) + round(4 * max(s["timer"], 0.0))
            s["score"] += gained
            say(s, hud, f"+{gained}")
            s["lit"], s["wait"] = None, PAUSE
        elif pick is not None or s["timer"] <= 0:
            s["streak"] = 0
            s["missed"] += 1
            s["lives"] -= 1
            say(s, hud, "MISS" if pick is not None else "TOO SLOW")
            s["lit"], s["wait"] = None, PAUSE
    if s["verdict_for"] <= 0:
        hud.set("verdict", "")
    hud.set("score", f"SCORE {s['score']}")
    hud.set("streak", f"Streak x{s['streak']}" if s["streak"] > 1 else "")
    hud.set("time", max(s["left"], 0.0))
    hud.set("clock", f"{math.ceil(max(s['left'], 0.0))} s")
    hud.set("lives", s["lives"])
    if s["left"] <= 0 or s["lives"] <= 0:
        finish(entity, ctx)


def say(s, hud, text):
    hud.set("verdict", text)
    s["verdict_for"] = 0.8
