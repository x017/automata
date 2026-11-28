"""coin: spins, bobs, and is collected when the player touches it.

Attributes (Inspector): value (Int) — how many points this coin is worth.
"""

import math

PICKUP_RADIUS = 1.1


def start(entity, ctx):
    entity.state["home"] = entity.transform.position
    entity.attributes["collected"] = False


def update(entity, ctx):
    if entity.attributes.get("collected"):
        return
    hx, hy, hz = entity.state["home"]
    t = ctx.time.elapsed
    entity.transform.position = (hx, hy + 0.15 * math.sin(t * 3.0 + hx), hz)
    entity.transform.rotation_euler = (0.0, t * 2.5, 0.0)

    px, py, pz = ctx.scene["Player"].transform.position
    if math.dist((px, py, pz), (hx, hy, hz)) < PICKUP_RADIUS:
        entity.attributes["collected"] = True
        entity.transform.position = (hx, -100.0, hz)  # out of sight
        print(f"+{entity.attributes.get('value', 1)}")
