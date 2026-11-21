"""follow_camera: the finished third-person camera from the tutorial.

Step 1 follows the player, Step 2 orbits with Q / E, Step 3 looks ahead in
the direction of travel, and Step 4 widens the view while sprinting.
"""

import math

DISTANCE = 8.0
HEIGHT = 3.5
SMOOTH = 6.0
TURN = 2.0  # radians per second
LOOK_AHEAD = 0.35  # seconds of travel to lead by
FOV = 60.0
SPRINT_FOV = 72.0


def start(entity, ctx):
    entity.state["yaw"] = 0.0


def orbit_offset(entity, ctx):
    """Step 2: rotate the offset around the player with Q / E."""
    yaw = entity.state["yaw"] + ctx.input.axis("q", "e") * TURN * ctx.time.delta
    entity.state["yaw"] = yaw
    return (DISTANCE * math.sin(yaw), HEIGHT, DISTANCE * math.cos(yaw))


def look_ahead(entity, ctx, player):
    """Step 3: aim a little ahead of where the player is heading."""
    p = player.transform.position
    last = entity.state.get("last", p)
    dt = max(ctx.time.delta, 1e-4)
    vel = [(a - b) / dt for a, b in zip(p, last)]
    entity.state["last"] = p

    lead = entity.state.get("lead", (0.0, 0.0, 0.0))
    goal = (vel[0] * LOOK_AHEAD, 0.0, vel[2] * LOOK_AHEAD)
    k = min(1.0, 4.0 * ctx.time.delta)
    lead = tuple(v + (g - v) * k for v, g in zip(lead, goal))
    entity.state["lead"] = lead
    return (p[0] + lead[0], p[1] + 0.6, p[2] + lead[2])


def sprint_zoom(cam, ctx):
    """Step 4: ease the field of view wider while Shift is held."""
    goal_fov = SPRINT_FOV if ctx.input.pressed("shift") else FOV
    cam.fov += (goal_fov - cam.fov) * min(1.0, 6.0 * ctx.time.delta)


def update(entity, ctx):
    player = ctx.scene["Player"]
    cam = entity.camera
    cam.follow(player, offset=orbit_offset(entity, ctx), smooth=SMOOTH)  # Step 1
    cam.look_at(look_ahead(entity, ctx, player))
    sprint_zoom(cam, ctx)
