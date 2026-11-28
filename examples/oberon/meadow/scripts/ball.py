"""ball: the runner kicks it when she walks into it (or press K nearby).

It's a dynamic body, and moving bodies push grass aside just as characters
do, so it leaves a wake through the meadow as it rolls.
"""

import math

REACH = 0.9  # how close her feet must be, metres
KICK = 3.5  # impulse, newton-seconds


def kick(entity, ctx, runner, force):
    bx, _, bz = entity.transform.position
    rx, _, rz = runner.transform.position
    dx, dz = bx - rx, bz - rz
    length = math.hypot(dx, dz) or 1.0
    ctx.physics.apply_impulse(entity, (dx / length * force, force * 0.25, dz / length * force))


def update(entity, ctx):
    runner = ctx.scene.find("Runner")
    if runner is None:
        return
    bx, _, bz = entity.transform.position
    rx, _, rz = runner.transform.position
    near = math.hypot(bx - rx, bz - rz) < REACH
    was_near = entity.state.get("near", False)
    entity.state["near"] = near
    if (near and not was_near) or (near and ctx.input.just_pressed("k")):
        kick(entity, ctx, runner, KICK)
