"""stroller: walks round and round the fountain.

Walking in a circle leans the ponytail and skirt out sideways, a little:
the spring bones feel the turn as well as the speed.
"""

import math

RADIUS = 3.2
WALK = 1.35  # m/s


def start(entity, ctx):
    x, _, z = entity.transform.position
    entity.state["centre"] = (x, z - RADIUS)
    entity.state["angle"] = 0.0
    entity.animation.play("Walk")


def update(entity, ctx):
    angle = entity.state["angle"] + WALK / RADIUS * ctx.time.delta
    entity.state["angle"] = angle
    cx, cz = entity.state["centre"]
    y = entity.transform.position[1]
    entity.transform.position = (cx + RADIUS * math.sin(angle), y, cz + RADIUS * math.cos(angle))
    # Face along the circle (anticlockwise seen from above).
    entity.transform.rotation_euler = (0.0, angle + math.pi / 2, 0.0)
