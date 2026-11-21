"""The benchmark's behaviour script: one copy runs on every scripted entity.

Each frame it reads two attributes, moves its entity in a small circle and
casts a ray down at the ground: the three things almost every gameplay
script does. Walkers (the physics scenes) move through their character
controller instead of teleporting, so physics has real work to do.
"""

import math


def update(entity, ctx):
    attrs = entity.attributes
    speed = attrs["speed"]
    t = ctx.time.elapsed * speed + attrs["phase"]
    if attrs.get("walker"):
        entity.character_velocity = (math.cos(t) * 2.0, -1.0, math.sin(t) * 2.0)
    else:
        x, y, z = entity.transform.position
        step = speed * ctx.time.delta
        entity.transform.position = (x + math.cos(t) * step, y, z + math.sin(t) * step)
    hit = ctx.physics.raycast(entity.transform.position, (0.0, -1.0, 0.0), 10.0, ignore=entity)
    if hit is not None:
        attrs["height"] = hit.distance
