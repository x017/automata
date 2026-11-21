"""camera_rail: an intro flyover through the Shot markers, then a follow camera.

Shots are Empty entities named "Shot 1", "Shot 2", ... with a Vec3 attribute
`look` (the point that shot looks at). Move them in Oberon to change the
flyover without touching code. Press Space to skip the intro.
"""

import math

SHOT_TIME = 2.5  # seconds per segment
BLEND = 1.0  # seconds to blend into the follow camera
DISTANCE = 8.0
HEIGHT = 3.5
TURN = 2.0  # radians per second, Q / E orbit after the intro


def smoothstep(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3.0 - 2.0 * t)


def lerp(a, b, t):
    return tuple(x + (y - x) * t for x, y in zip(a, b))


def shots(ctx):
    found = sorted((e for e in ctx.scene if e.name.startswith("Shot ")), key=lambda e: e.name)
    return [(tuple(e.transform.position), tuple(e.attributes["look"])) for e in found]


def rail(keys, time):
    """Position and target `time` seconds into the flyover, or None when done."""
    if len(keys) < 2:
        return None
    segment = int(time // SHOT_TIME)
    if segment >= len(keys) - 1:
        return None
    k = smoothstep((time - segment * SHOT_TIME) / SHOT_TIME)
    (p0, a0), (p1, a1) = keys[segment], keys[segment + 1]
    return lerp(p0, p1, k), lerp(a0, a1, k)


def start(entity, ctx):
    entity.state["keys"] = shots(ctx)
    entity.state["yaw"] = 0.0
    entity.state["done_at"] = None


def update(entity, ctx):
    cam = entity.camera
    t = ctx.time.elapsed

    # Skippable intro: Space jumps straight to the hand-off.
    if entity.state["keys"] and ctx.input.just_pressed("space"):
        entity.state["keys"] = []

    on_rail = rail(entity.state["keys"], t)
    if on_rail is not None:
        cam.position, cam.target = on_rail
        return

    # After the flyover: follow, easing in over BLEND seconds.
    if entity.state["done_at"] is None:
        entity.state["done_at"] = t
    blend = smoothstep((t - entity.state["done_at"]) / BLEND)
    yaw = entity.state["yaw"] + ctx.input.axis("q", "e") * TURN * ctx.time.delta
    entity.state["yaw"] = yaw
    offset = (DISTANCE * math.sin(yaw), HEIGHT, DISTANCE * math.cos(yaw))
    cam.follow(ctx.scene["Player"], offset=offset, smooth=1.0 + 7.0 * blend)
