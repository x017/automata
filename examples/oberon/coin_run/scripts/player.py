"""player: camera-relative walking and jumping for a Character Controller.

The controls are actions (``ctx.settings.bind``), so a settings screen can
rebind them with ``ctx.settings.rebind("jump", "k")`` and the change is
remembered.
"""

import math

SPEED = 5.0
SPRINT = 1.6
JUMP = 7.0
GRAVITY = 20.0


def start(entity, ctx):
    entity.state["vy"] = 0.0
    controls = ctx.settings
    controls.bind("forward", "w", "arrow_up")
    controls.bind("back", "s", "arrow_down")
    controls.bind("left", "a", "arrow_left")
    controls.bind("right", "d", "arrow_right")
    controls.bind("jump", "space")
    controls.bind("sprint", "shift")


def update(entity, ctx):
    keys = ctx.input
    forward = keys.axis("back", "forward")
    right = keys.axis("left", "right")

    # Walk relative to where the camera looks (flattened onto the ground).
    fx, fz = 0.0, -1.0
    cam = ctx.camera
    if cam is not None:
        (ex, _, ez), (tx, _, tz) = cam.position, cam.target
        length = math.hypot(tx - ex, tz - ez)
        if length > 1e-4:
            fx, fz = (tx - ex) / length, (tz - ez) / length
    rx, rz = -fz, fx
    vx = (fx * forward + rx * right) * SPEED
    vz = (fz * forward + rz * right) * SPEED
    if keys.pressed("sprint"):
        vx, vz = vx * SPRINT, vz * SPRINT

    vy = entity.state["vy"]
    if ctx.physics.grounded(entity):
        vy = JUMP if keys.just_pressed("jump") else -2.0
    else:
        vy -= GRAVITY * ctx.time.delta
    entity.state["vy"] = vy
    entity.character_velocity = (vx, vy, vz)
