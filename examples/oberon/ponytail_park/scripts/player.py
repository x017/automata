"""player: you. W A S D (or the arrows) walk, hold Shift to run, 1 waves,
2 cheers and 3 leaps.

Moving sets the clip: Idle, Walk or Run. The emotes live in another file,
``models/anims/emotes.glb``, which the model's import settings borrow
(Clip files): its clips are called ``emotes:Wave``, ``emotes:Cheer`` and
``emotes:Leap``.

Animation events: the import settings mark each footfall of Walk and Run
as ``footstep`` and the touchdown of Leap as ``land``. One subscription
to ``*:animation:footstep`` kicks up dust under every runner in the park
(the jogger and the stroller too), and ``on_animation_event`` hears this
runner's own ``land``.

Root motion: her Animation has Root Motion on, so Leap's hips travelling
2.4 m carry the whole entity forward, rather than snapping back when the
clip ends. Walk and Run stay in place, so this script moves her for those.

Nothing here touches the hair or the skirt; their swing comes from the
spring bones in the model's import settings. Press R to put them back at
rest with ``entity.animation.reset_springs()``.
"""

import math

WALK = 1.4
RUN = 4.2
EMOTES = {"1": "emotes:Wave", "2": "emotes:Cheer", "3": "emotes:Leap"}  # Q, E orbit the camera
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


def kick_up_dust(msg):
    """Every runner's footsteps (``<name>:animation:footstep``)."""
    msg.ctx.particles.burst("dust", msg.entity, count=5, size=0.18, speed=0.6, lifetime=0.6)


def start(entity, ctx):
    entity.state["yaw"] = entity.transform.rotation_euler[1]
    entity.state["landings"] = 0
    # A wildcard: footsteps of any entity, not just this one.
    ctx.subscribe("*:animation:footstep", kick_up_dust)


def on_animation_event(entity, ctx, name):
    """This entity's own animation events, by name."""
    if name == "land":
        entity.state["landings"] += 1
        ctx.particles.burst("dust", entity, count=24, size=0.3, speed=1.6)


def update(entity, ctx):
    keys = ctx.input
    dt = ctx.time.delta
    anim = entity.animation
    # Mid-leap the clip is in charge of where she goes.
    leaping = anim.clip == "emotes:Leap" and anim.playing
    forward = keys.axis("s", "w") or keys.axis("arrow_down", "arrow_up")
    right = keys.axis("a", "d") or keys.axis("arrow_left", "arrow_right")
    if leaping:
        forward = right = 0.0

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
    edge = abs(x) > EDGE or abs(z) > EDGE
    # Only write the position when it changes: the leap's root motion moves
    # her too, and a write would put her back where this frame began.
    if length > 0.1 or edge:
        x = min(max(x + vx * dt, -EDGE), EDGE)
        z = min(max(z + vz * dt, -EDGE), EDGE)
        entity.transform.position = (x, y, z)
    if length > 0.1:
        entity.state["yaw"] = turn_towards(entity.state["yaw"], math.atan2(vx, vz), TURN, dt)
        entity.transform.rotation_euler = (0.0, entity.state["yaw"], 0.0)

    for key, emote in EMOTES.items():
        if keys.just_pressed(key) and not leaping:
            anim.play(emote, loop=False, fade=0.15, restart=True)
    # An emote plays to its end unless she moves.
    emoting = anim.clip in EMOTES.values() and anim.playing
    if length > 0.1 or not emoting:
        anim.play(clip_for(math.hypot(vx, vz)), fade=0.2)
    if keys.just_pressed("r"):
        anim.reset_springs()
