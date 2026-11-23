"""The fire crackles: now and then a pop throws a handful of extra embers."""

import random


def pop(entity, ctx):
    ctx.particles.emit(entity, random.randint(8, 20))
    entity.attributes["pops"] = entity.attributes.get("pops", 0) + 1
    ctx.after(random.uniform(0.8, 3.0), pop, entity, ctx)  # and again, a while later


def start(entity, ctx):
    entity.attributes["pops"] = 0
    ctx.after(random.uniform(0.5, 2.0), pop, entity, ctx)
