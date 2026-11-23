"""player: walk the robot around and pick the clip that fits what it's doing.

W A S D (or the arrows) walk, Shift runs, Space jumps and F waves. Walking
works like Coin Run's player script. On top of that, ``choose_clip`` looks
at what the robot is doing each frame and ``entity.animation.play`` blends
to the matching clip. Asking for the clip that's already playing does
nothing, so it's safe to call every frame.
"""

import math

WALK = 2.2
RUN = 5.0
JUMP = 6.5
GRAVITY = 20.0
TURN = 12.0  # how fast the robot turns to face where it walks

# How long each kind of change takes to blend in, in seconds.
FADES = {"Idle": 0.3, "Walking": 0.2, "Running": 0.2, "Jump": 0.1, "Wave": 0.2}


def choose_clip(grounded, speed, current, current_playing, wave_pressed):
    """The clip the robot should be playing.

    ``speed`` is the ground speed in m/s. ``current`` is the clip playing
    now; ``current_playing`` is False once a one-shot (Jump, Wave) ends.
    Returns ``(clip, loop)``.
    """
    if not grounded:
        return "Jump", False
    if speed > (WALK + RUN) / 2:
        return "Running", True
    if speed > 0.2:
        return "Walking", True
    if wave_pressed or (current == "Wave" and current_playing):
        return "Wave", False
    return "Idle", True


def turn_towards(yaw, goal, rate, dt):
    """Turns ``yaw`` towards ``goal`` the short way round (radians)."""
    diff = (goal - yaw + math.pi) % (2 * math.pi) - math.pi
    return yaw + diff * min(1.0, rate * dt)


def start(entity, ctx):
    entity.state["vy"] = 0.0
    entity.state["yaw"] = entity.transform.rotation_euler[1]


def update(entity, ctx):
    keys = ctx.input
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
    rx, rz = -fz, fx
    dx, dz = fx * forward + rx * right, fz * forward + rz * right
    length = math.hypot(dx, dz)
    if length > 1.0:
        dx, dz = dx / length, dz / length
    pace = RUN if keys.pressed("shift") else WALK
    vx, vz = dx * pace, dz * pace

    grounded = ctx.physics.grounded(entity)
    vy = entity.state["vy"]
    if grounded:
        vy = JUMP if keys.just_pressed("space") else -2.0
    else:
        vy -= GRAVITY * ctx.time.delta
    entity.state["vy"] = vy
    entity.character_velocity = (vx, vy, vz)

    # Face the way we're walking. The model looks down +Z, so yaw 0 is +Z.
    if length > 0.1:
        goal = math.atan2(vx, vz)
        entity.state["yaw"] = turn_towards(entity.state["yaw"], goal, TURN, ctx.time.delta)
        entity.transform.rotation_euler = (0.0, entity.state["yaw"], 0.0)

    # Jumping counts from the take-off frame, before the ground lets go.
    anim = entity.animation
    clip, loop = choose_clip(
        grounded and vy <= 0.0,
        math.hypot(vx, vz),
        anim.clip,
        anim.playing,
        keys.just_pressed("f"),
    )
    anim.play(clip, loop=loop, fade=FADES[clip])
