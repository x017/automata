"""motor_boat: cruises a loop round the island, riding the swell.

The boat is a kinematic body: it goes where this script puts it, shoving
floating crates and buoys aside, and the water it pushes through leaves a
V-shaped wake (the ripple simulation does that for anything moving through
the water). Each frame it samples the surface under its bow, stern and
sides to sit on the waves and pitch and roll with them.

Attributes (Inspector → Attributes):
    center   the loop's centre, (x, y, z) (y is ignored)
    radius_x the loop's half width along X, metres
    radius_z the loop's half width along Z, metres
    speed    metres per second
    draft    how deep the hull sits, metres below the waterline
"""

import math

LENGTH, BEAM, HEIGHT = 3.8, 1.6, 0.6  # the hull's size (its Transform scale)
SETTLE = 4.0  # how quickly it follows the waves (per second)


def start(entity, ctx):
    a = entity.attributes
    s = entity.state
    center = tuple(a.get("center", (0.0, 0.0, 2.0)))
    s["center"] = (center[0], center[2])
    s["radius"] = (float(a.get("radius_x", 24.0)), float(a.get("radius_z", 25.0)))
    s["speed"] = float(a.get("speed", 4.5))
    s["draft"] = float(a.get("draft", 0.18))
    # Join the loop where the boat starts.
    x, _, z = entity.transform.position
    cx, cz = s["center"]
    rx, rz = s["radius"]
    s["angle"] = math.atan2((x - cx) / rx, (z - cz) / rz)
    s["pose"] = None


def update(entity, ctx):
    s = entity.state
    dt = ctx.time.delta
    cx, cz = s["center"]
    rx, rz = s["radius"]
    # Along the ellipse at a steady speed: the angle advances by the speed
    # over the local radius of the loop.
    a = s["angle"]
    dx, dz = rx * math.cos(a), -rz * math.sin(a)
    s["angle"] = a + s["speed"] * dt / max(math.hypot(dx, dz), 1e-3)
    a = s["angle"]
    x, z = cx + rx * math.sin(a), cz + rz * math.cos(a)
    dx, dz = rx * math.cos(a), -rz * math.sin(a)
    yaw = math.atan2(dx, dz)

    # The waves under it: bow and stern give the pitch, the sides the roll.
    fx, fz = math.sin(yaw), math.cos(yaw)
    rgt_x, rgt_z = fz, -fx

    def height(ox, oz):
        h = ctx.water.height(x + ox, z + oz)
        return 0.0 if h is None else h

    half_l, half_b = LENGTH * 0.4, BEAM * 0.4
    bow, stern = height(fx * half_l, fz * half_l), height(-fx * half_l, -fz * half_l)
    right, left = height(rgt_x * half_b, rgt_z * half_b), height(-rgt_x * half_b, -rgt_z * half_b)
    level = (bow + stern + right + left) * 0.25
    pose = (
        level + HEIGHT * 0.5 - s["draft"],
        -math.atan2(bow - stern, 2.0 * half_l),
        math.atan2(right - left, 2.0 * half_b),
    )
    # Ease toward it, so the hull rides the swell rather than every ripple.
    if s["pose"] is not None:
        k = 1.0 - math.exp(-SETTLE * dt)
        pose = tuple(old + (new - old) * k for old, new in zip(s["pose"], pose))
    s["pose"] = pose
    y, pitch, roll = pose
    entity.transform.position = (x, y, z)
    entity.transform.rotation_euler = euler_xyz(yaw, pitch, roll)


def euler_xyz(yaw, pitch, roll):
    """The Transform's XYZ Euler angles for a heading ``yaw`` (about Y),
    then ``pitch`` (about the boat's own X) and ``roll`` (its own Z)."""
    cy, sy = math.cos(yaw), math.sin(yaw)
    cp, sp = math.cos(pitch), math.sin(pitch)
    cr, sr = math.cos(roll), math.sin(roll)
    # Ry(yaw) * Rx(pitch) * Rz(roll), the entries the XYZ angles need.
    m00 = cy * cr + sy * sp * sr
    m01 = -cy * sr + sy * sp * cr
    m02 = sy * cp
    m12 = -sp
    m22 = cy * cp
    return (
        math.atan2(-m12, m22),
        math.asin(max(-1.0, min(1.0, m02))),
        math.atan2(-m01, m00),
    )
