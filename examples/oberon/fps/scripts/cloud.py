"""cloud: drifts gently up and down, each at its own pace."""

import math
import random


def start(entity, ctx):
    entity.state["t"] = 0.0
    entity.state["speed"] = random.uniform(0.1, 2.0)
    entity.state["pace"] = random.uniform(0.1, 2.0)


def update(entity, ctx):
    s = entity.state
    dt = ctx.time.delta
    s["t"] += dt
    x, y, z = entity.transform.position
    y += math.cos(s["t"] * s["pace"]) * s["speed"] * dt
    entity.transform.position = (x, y, z)
