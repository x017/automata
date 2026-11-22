"""The fire crackles: now and then a pop throws a handful of extra embers."""

import random


def start(entity, ctx):
    entity.state["next"] = random.uniform(0.5, 2.0)
    entity.attributes["pops"] = 0


def update(entity, ctx):
    s = entity.state
    s["next"] -= ctx.time.delta
    if s["next"] <= 0.0:
        s["next"] = random.uniform(0.8, 3.0)
        ctx.particles.emit(entity, random.randint(8, 20))
        entity.attributes["pops"] = entity.attributes.get("pops", 0) + 1
