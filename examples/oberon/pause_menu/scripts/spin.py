"""Turns the shape at its ``speed`` attribute (radians per second).

A click in the game kicks it faster, and it eases back. It holds still
while the menu is open. Presses on the menu are the UI's, so
``mouse_just_pressed`` doesn't see them.
"""


def start(entity, ctx):
    entity.state["speed"] = entity.attributes.get("speed", 1.0)


def update(entity, ctx):
    if ctx.scene["Menu"].attributes.get("paused"):
        return
    s = entity.state
    if ctx.input.mouse_just_pressed("left"):
        s["speed"] += 4.0
    base = entity.attributes.get("speed", 1.0)
    s["speed"] += (base - s["speed"]) * min(1.0, ctx.time.delta * 1.2)
    entity.transform.rotate((0.0, s["speed"] * ctx.time.delta, 0.0))
