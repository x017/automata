"""player: camera-relative walking and jumping for a Character Controller."""

import math

SPEED = 5.0
SPRINT = 1.6
JUMP = 7.0
GRAVITY = 20.0


def start(entity, ctx):
    entity.state["vy"] = 0.0


def update(entity, ctx):
    keys = ctx.input
    forward = float(keys.pressed("w") or keys.pressed("arrow_up")) - float(
        keys.pressed("s") or keys.pressed("arrow_down")
    )
    right = float(keys.pressed("d") or keys.pressed("arrow_right")) - float(
        keys.pressed("a") or keys.pressed("arrow_left")
    )

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
    if keys.pressed("shift"):
        vx, vz = vx * SPRINT, vz * SPRINT

    vy = entity.state["vy"]
    if ctx.physics.grounded(entity):
        vy = JUMP if keys.just_pressed("space") else -2.0
    else:
        vy -= GRAVITY * ctx.time.delta
    entity.state["vy"] = vy
    entity.character_velocity = (vx, vy, vz)
