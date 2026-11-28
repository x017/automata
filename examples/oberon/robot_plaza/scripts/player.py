"""player: walk the robot around; its animation graph picks the clips.

W A S D (or the arrows) walk, Shift runs, Space jumps and F waves (even
while walking: waving only moves the arms). Walking works like Coin Run's
player script.

This script never names a clip. It tells the robot's animation graph,
``anims/robot.anim.ron``, what the robot is doing: how fast it goes,
whether it's on the ground, and when it jumps or waves. The graph blends
Idle, Walking and Running by speed, plays Jump and goes back when the
robot lands, and lays Wave over the upper body. Open the graph in Oberon
to see it, and watch it light up while you play.
"""

import math

WALK = 2.2
RUN = 5.0
JUMP = 6.5
GRAVITY = 20.0
TURN = 12.0  # how fast the robot turns to face where it walks


def animate(anim, speed, grounded, jumped, waved):
    """Tells the animation graph what the robot is doing this frame.

    ``speed`` is the ground speed in m/s, ``grounded`` whether the robot
    stands on something, ``jumped`` and ``waved`` whether it took off or
    F was pressed this frame.
    """
    anim.set("speed", speed)
    anim.set("grounded", grounded)
    if jumped:
        anim.trigger("jump")
    if waved:
        anim.trigger("wave")


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
    jumped = grounded and vy > 0.0
    animate(
        entity.animation,
        math.hypot(vx, vz),
        grounded and vy <= 0.0,
        jumped,
        keys.just_pressed("f"),
    )
