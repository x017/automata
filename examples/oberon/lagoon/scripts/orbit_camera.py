"""orbit_camera: circles the island and lets you look around.

Drag with the left or right mouse button (or use A / D, Q / E or the
arrow keys) to orbit, and W / S to zoom. Leave it alone for a few seconds
and it drifts round on its own again.

Attributes (Inspector → Attributes):
    target    the point to orbit, (x, y, z)
    distance  metres from the target
    yaw       starting angle around the target, radians (0 = from +Z)
    pitch     starting height angle, radians (0 = level)
    spin      idle drift speed, radians per second
"""

import math

DRAG = 0.005  # radians per pixel dragged
TURN = 1.2  # radians per second with the keys
ZOOM = 1.2  # zoom speed with W / S
MIN_PITCH, MAX_PITCH = 0.06, 1.35
MIN_DISTANCE, MAX_DISTANCE = 6.0, 90.0
IDLE = 3.0  # seconds without input before the drift resumes


def start(entity, ctx):
    a = entity.attributes
    s = entity.state
    s["target"] = tuple(a.get("target", (0.0, 0.5, 0.0)))
    s["distance"] = float(a.get("distance", 30.0))
    s["yaw"] = float(a.get("yaw", 0.4))
    s["pitch"] = float(a.get("pitch", 0.35))
    s["idle"] = IDLE
    place(entity)


def eye(target, distance, yaw, pitch):
    """The camera position orbiting `target` (yaw 0 looks from +Z)."""
    flat = distance * math.cos(pitch)
    return (
        target[0] + flat * math.sin(yaw),
        target[1] + distance * math.sin(pitch),
        target[2] + flat * math.cos(yaw),
    )


def place(entity):
    s = entity.state
    cam = entity.camera
    cam.position = eye(s["target"], s["distance"], s["yaw"], s["pitch"])
    cam.look_at(s["target"])


def update(entity, ctx):
    s = entity.state
    dt = ctx.time.delta
    inp = ctx.input

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
