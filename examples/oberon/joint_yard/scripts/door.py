"""Swings the door open and shut with its hinge motor.

The door's Joint has a motor that pulls to a target angle; this script just
moves the target. Every three seconds (or when you press E) it flips
between open and shut, and the motor swings the door there.
"""

OPEN = 1.4  # radians, about 80°
EVERY = 3.0  # seconds


def start(entity, ctx):
    entity.state["open"] = False
    entity.state["next"] = 1.0


def update(entity, ctx):
    now = ctx.time.elapsed
    if now >= entity.state["next"] or ctx.input.just_pressed("e"):
        entity.state["open"] = not entity.state["open"]
        entity.state["next"] = now + EVERY
        entity.joint.motor_target = OPEN if entity.state["open"] else 0.0
    entity.attributes["angle"] = round(entity.joint.angle, 3)
