"""player: walk (W A S D or the arrows) and run (Shift) through the grass.

She has a Character Controller, which is all the grass needs: every frame
it bends away from the bottom of her collider. Moving sets her clip (Idle,
Walk or Run) and turns her to face the way she goes. Her ponytail and skirt
are spring bones, so they blow in the same wind as the grass.
"""

import math

WALK = 1.4
RUN = 4.2
TURN = 10.0  # how fast she turns to face where she's going
GRAVITY = 20.0
EDGE = 30.0  # stay on the ground


def clip_for(speed):
    """The clip that fits a ground speed in m/s."""
    if speed > (WALK + RUN) / 2:
        return "Run"
    if speed > 0.1:
        return "Walk"
    return "Idle"


def turn_towards(yaw, goal, rate, dt):
    """Turns ``yaw`` towards ``goal`` the short way round (radians)."""
    diff = (goal - yaw + math.pi) % (2 * math.pi) - math.pi
    return yaw + diff * min(1.0, rate * dt)


def heading(ctx, forward, right):
    """W A S D as a ground direction relative to the camera (length <= 1)."""
    fx, fz = 0.0, -1.0
    cam = ctx.camera
    if cam is not None:
        (ex, _, ez), (tx, _, tz) = cam.position, cam.target
        length = math.hypot(tx - ex, tz - ez)
        if length > 1e-4:
            fx, fz = (tx - ex) / length, (tz - ez) / length
    dx, dz = fx * forward - fz * right, fz * forward + fx * right
    length = math.hypot(dx, dz)
    if length > 1.0:
        dx, dz = dx / length, dz / length
    return dx, dz


def start(entity, ctx):
    entity.state["yaw"] = entity.transform.rotation_euler[1]
    entity.state["vy"] = 0.0


def update(entity, ctx):
    keys = ctx.input
    dt = ctx.time.delta
    forward = keys.axis("s", "w") or keys.axis("arrow_down", "arrow_up")
    right = keys.axis("a", "d") or keys.axis("arrow_left", "arrow_right")
    dx, dz = heading(ctx, forward, right)
    pace = RUN if keys.pressed("shift") else WALK
    vx, vz = dx * pace, dz * pace

    # Keep her on the ground.
    x, _, z = entity.transform.position
    if abs(x) > EDGE and x * vx > 0:
        vx = 0.0
    if abs(z) > EDGE and z * vz > 0:
        vz = 0.0

    vy = entity.state["vy"]
    vy = -2.0 if ctx.physics.grounded(entity) else vy - GRAVITY * dt
    entity.state["vy"] = vy
    entity.character_velocity = (vx, vy, vz)

    speed = math.hypot(vx, vz)
    if speed > 0.1:
        entity.state["yaw"] = turn_towards(entity.state["yaw"], math.atan2(vx, vz), TURN, dt)
        entity.transform.rotation_euler = (0.0, entity.state["yaw"], 0.0)
    entity.animation.play(clip_for(speed), fade=0.2)
