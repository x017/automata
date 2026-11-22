"""swimmer: walk the island, jump off the pier, swim and dive.

Press Tab to take control (Tab again hands the camera back to the island
orbit). Then W / A / S / D move (relative to the camera), Shift runs and
Space jumps. In the water the swimmer floats with their head out: hold C
(or Ctrl) to dive, Space to swim back up. Hit the water hard and it
splashes; dive under and the view turns blue-green, with sunlight
dancing on the sea floor.

Attributes (Inspector -> Attributes):
    speed       walking speed, m/s
    swim_speed  swimming speed, m/s
    jump        jump speed, m/s
"""

import math

RUN = 1.8  # Shift: this much faster
GRAVITY = 18.0  # m/s^2 in the air
FLOAT = 0.55  # the centre floats this far under the surface (head out)
DIVE = 2.2  # m/s straight up or down in the water
WATER_DRAG = 4.0  # how quickly the water takes over the vertical speed


def start(entity, ctx):
    s = entity.state
    s["vy"] = 0.0
    s["active"] = False
    s["swimming"] = False
    s["depth"] = None


def wish(entity, ctx):
    """The direction W / A / S / D ask for, flattened, relative to the camera."""
    keys = ctx.input
    forward = float(keys.pressed("w")) - float(keys.pressed("s"))
    right = float(keys.pressed("d")) - float(keys.pressed("a"))
    fx, fz = 0.0, -1.0
    cam = ctx.camera
    if cam is not None:
        (ex, _, ez), (tx, _, tz) = cam.position, cam.target
        length = math.hypot(tx - ex, tz - ez)
        if length > 1e-4:
            fx, fz = (tx - ex) / length, (tz - ez) / length
    rx, rz = -fz, fx
    x, z = fx * forward + rx * right, fz * forward + rz * right
    length = math.hypot(x, z)
    if length > 1.0:
        x, z = x / length, z / length
    return x, z


def depth_of(entity, ctx):
    """How far the swimmer's centre is under the water surface (None: no water)."""
    x, y, z = entity.transform.position
    h = ctx.water.height(x, z)
    return None if h is None else h - y


def update(entity, ctx):
    s = entity.state
    a = entity.attributes
    keys = ctx.input
    dt = ctx.time.delta
    if keys.just_pressed("tab"):
        s["active"] = not s["active"]
    active = s["active"]

    mx, mz = wish(entity, ctx) if active else (0.0, 0.0)
    depth = depth_of(entity, ctx)
    # In the water once it's up to the chest.
    swimming = depth is not None and depth > FLOAT - 0.15
    vy = s["vy"]
    if swimming:
        speed = float(a.get("swim_speed", 2.2))
        up = active and keys.pressed("space")
        down = active and (keys.pressed("c") or keys.pressed("ctrl"))
        if down:
            target = -DIVE
        elif up:
            target = DIVE
        else:
            # Buoyancy: back up to floating height.
            target = max(-2.0, min(2.5, (depth - FLOAT) * 2.5))
        # The water slows a plunge and eases into the swim.
        vy += (target - vy) * min(1.0, WATER_DRAG * dt)
    else:
        speed = float(a.get("speed", 4.0))
        if active and keys.pressed("shift"):
            speed *= RUN
        if ctx.physics.grounded(entity):
            vy = float(a.get("jump", 6.5)) if active and keys.just_pressed("space") else -2.0
        else:
            vy -= GRAVITY * dt
    s["vy"] = vy
    s["swimming"] = swimming
    s["depth"] = depth
    entity.character_velocity = (mx * speed, vy, mz * speed)
