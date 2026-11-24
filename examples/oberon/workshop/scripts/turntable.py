"""Turns a showpiece slowly about its vertical axis, so its material
catches the light from every side. ``speed`` (radians a second) is an
attribute set in the Inspector."""


def update(entity, ctx):
    speed = entity.attributes.get("speed", 0.4)
    entity.transform.rotate((0.0, speed * ctx.time.delta, 0.0))
