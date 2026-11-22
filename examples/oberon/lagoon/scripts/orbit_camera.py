"""orbit_camera: circles the island and lets you look around.

Drag with the left or right mouse button (or use A / D, Q / E or the
arrow keys) to orbit, and W / S to zoom. Leave it alone for a few seconds
and it drifts round on its own again.

With a `follow` entity whose script is in control (`state["active"]`, the
swimmer after Tab), it orbits that entity instead, close behind it: W / A
/ S / D are the swimmer's then, so only dragging, Q / E and the arrow
keys turn the camera. It can dip below the swimmer's level, under water.

Attributes (Inspector → Attributes):
    target           the point to orbit, (x, y, z)
    distance         metres from the target
    yaw              starting angle around the target, radians (0 = from +Z)
    pitch            starting height angle, radians (0 = level)
    spin             idle drift speed, radians per second
    follow           name of an entity to follow while it's active ("" = none)
    follow_distance  metres behind it when following
"""

import math

DRAG = 0.005  # radians per pixel dragged
TURN = 1.2  # radians per second with the keys
ZOOM = 1.2  # zoom speed with W / S
MIN_PITCH, MAX_PITCH = 0.06, 1.35
MIN_DISTANCE, MAX_DISTANCE = 6.0, 90.0
IDLE = 3.0  # seconds without input before the drift resumes
MIN_FOLLOW_PITCH = -0.3  # following, the camera can look up from below
FOLLOW_HEIGHT = 0.6  # metres above the followed entity's centre
FOLLOW_EASE = 8.0  # how quickly the camera catches up (per second)
DIVING = 1.0  # the followed entity this far under: the camera dives too
UNDER = 0.4  # ... and stays this far under the surface


def start(entity, ctx):
    a = entity.attributes
    s = entity.state
    s["target"] = tuple(a.get("target", (0.0, 0.5, 0.0)))
    s["distance"] = float(a.get("distance", 30.0))
    s["yaw"] = float(a.get("yaw", 0.4))
    s["pitch"] = float(a.get("pitch", 0.35))
    s["idle"] = IDLE
    s["follow_yaw"] = s["yaw"]
    s["follow_pitch"] = 0.25
    s["follow_at"] = None
    place(entity)


def eye(target, distance, yaw, pitch):
    """The camera position orbiting `target` (yaw 0 looks from +Z)."""
    flat = distance * math.cos(pitch)
    return (
        target[0] + flat * math.sin(yaw),
        target[1] + distance * math.sin(pitch),
        target[2] + flat * math.cos(yaw),
    )


def place(entity, target=None, distance=None, yaw=None, pitch=None):
    s = entity.state
    target = s["target"] if target is None else target
    cam = entity.camera
    cam.position = eye(
        target,
        s["distance"] if distance is None else distance,
        s["yaw"] if yaw is None else yaw,
        s["pitch"] if pitch is None else pitch,
    )
    cam.look_at(target)


def followed(entity, ctx):
    """The entity to follow, while its script has it in control."""
    name = entity.attributes.get("follow", "")
    other = ctx.scene.find(name) if name else None
    if other is not None and other.state.get("active"):
        return other
    return None


def follow(entity, ctx, other):
    """Orbits close behind `other`; the keys it uses are left to it."""
    s = entity.state
    dt = ctx.time.delta
    inp = ctx.input
    if s["follow_at"] is None:
        s["follow_yaw"] = s["yaw"]  # carry on from the island view's angle
        s["view_pitch"] = s["follow_pitch"]
    turn = inp.axis("e", "q") + inp.axis("right", "left")
    tilt = inp.axis("down", "up")
    dragging = inp.mouse_pressed("left") or inp.mouse_pressed("right")
    dx, dy = inp.mouse_delta if dragging else (0.0, 0.0)
    s["follow_yaw"] += turn * TURN * dt - dx * DRAG
    s["follow_pitch"] += tilt * TURN * 0.5 * dt + dy * DRAG
    s["follow_pitch"] = min(MAX_PITCH, max(MIN_FOLLOW_PITCH, s["follow_pitch"]))
    x, y, z = other.transform.position
    goal = (x, y + FOLLOW_HEIGHT, z)
    at = s["follow_at"] or goal
    k = min(1.0, FOLLOW_EASE * dt)
    at = tuple(a + (g - a) * k for a, g in zip(at, goal))
    s["follow_at"] = at
    distance = float(entity.attributes.get("follow_distance", 6.0))
    pitch = s["follow_pitch"]
    # Diving: stay under the surface with the swimmer, looking along.
    depth = other.state.get("depth")
    surface = ctx.water.height(at[0], at[2])
    if depth is not None and depth > DIVING and surface is not None:
        under = (surface - UNDER - at[1]) / distance
        pitch = min(pitch, math.asin(max(-1.0, min(1.0, under))))
        pitch = max(MIN_FOLLOW_PITCH, pitch)
    s["view_pitch"] = s.get("view_pitch", pitch) + (pitch - s.get("view_pitch", pitch)) * k
    place(entity, at, distance, s["follow_yaw"], s["view_pitch"])


def update(entity, ctx):
    s = entity.state
    dt = ctx.time.delta
    inp = ctx.input

    other = followed(entity, ctx)
    if other is not None:
        follow(entity, ctx, other)
        return
    s["follow_at"] = None

    turn = inp.axis("d", "a") + inp.axis("e", "q") + inp.axis("right", "left")
    tilt = inp.axis("down", "up")
    zoom = inp.axis("w", "s")
    dragging = inp.mouse_pressed("left") or inp.mouse_pressed("right")
    dx, dy = inp.mouse_delta if dragging else (0.0, 0.0)

    if turn or tilt or zoom or dx or dy:
        s["idle"] = 0.0
    else:
        s["idle"] += dt

    s["yaw"] += turn * TURN * dt - dx * DRAG
    s["pitch"] += tilt * TURN * 0.5 * dt + dy * DRAG
    s["pitch"] = min(MAX_PITCH, max(MIN_PITCH, s["pitch"]))
    s["distance"] *= math.exp(zoom * ZOOM * dt)
    s["distance"] = min(MAX_DISTANCE, max(MIN_DISTANCE, s["distance"]))
    if s["idle"] >= IDLE:
        s["yaw"] += float(entity.attributes.get("spin", 0.05)) * dt
    place(entity)
