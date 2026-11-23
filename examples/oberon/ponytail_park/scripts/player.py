"""player: you. W A S D (or the arrows) walk, hold Shift to run.

Moving sets the clip: Idle, Walk or Run. Nothing here touches the hair or
the skirt; their swing comes from the spring bones in the model's import
settings. Press R to put them back at rest with
``entity.animation.reset_springs()``.
"""

import math

WALK = 1.4
RUN = 4.2
TURN = 10.0  # how fast she turns to face where she's going
EDGE = 14.0  # stay on the grass


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


def start(entity, ctx):
    entity.state["yaw"] = entity.transform.rotation_euler[1]


def update(entity, ctx):
    keys = ctx.input
    dt = ctx.time.delta
    forward = keys.axis("s", "w") or keys.axis("arrow_down", "arrow_up")
    right = keys.axis("a", "d") or keys.axis("arrow_left", "arrow_right")

    # Walk relative to where the camera looks (flattened onto the ground).
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
    pace = RUN if keys.pressed("shift") else WALK
    vx, vz = dx * pace, dz * pace

    x, y, z = entity.transform.position
    x = min(max(x + vx * dt, -EDGE), EDGE)
    z = min(max(z + vz * dt, -EDGE), EDGE)
    entity.transform.position = (x, y, z)
    if length > 0.1:
        entity.state["yaw"] = turn_towards(entity.state["yaw"], math.atan2(vx, vz), TURN, dt)
        entity.transform.rotation_euler = (0.0, entity.state["yaw"], 0.0)

    anim = entity.animation
    anim.play(clip_for(math.hypot(vx, vz)), fade=0.2)
    if keys.just_pressed("r"):
        anim.reset_springs()
