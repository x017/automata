"""player: first-person controls, two blasters, health and the HUD.

The mouse looks around (the cursor is captured; Esc frees it, click to take
it back). WASD walks, Space jumps (once more in the air), the left mouse
button shoots and E switches weapon.

Attributes (Inspector): health (Int), 100 at the start.

Shots are ray casts from the eye. Anything they hit with a ``health``
attribute loses some; enemy.py notices and reacts. Enemies hurt the player
the same way, by lowering this entity's ``health``.
"""

import math
import random

SPEED = 5.0
ACCELERATION = 10.0  # how quickly walking speed is reached
JUMP = 8.0
JUMPS = 2  # one on the ground, one in the air
GRAVITY = 20.0
SENSITIVITY = 1.0 / 700.0  # radians per pixel of mouse movement
EYE = 0.6  # eye height above the capsule's centre
FALL_LIMIT = -10.0  # respawn below this
RANGE = 30.0  # how far shots reach
VOLUME = 0.45
HEARD_FROM = 10.0  # how far away drones hear a shot (m)

# The two blasters: entity name, sound, crosshair and how they shoot.
WEAPONS = [
    {
        "entity": "Blaster",
        "sound": "sounds/blaster.ogg",
        "crosshair": "sprites/crosshair.png",
        "cooldown": 0.25,
        "damage": 25,
        "spread": 0.06,
        "shots": 3,
        "kick": 0.035,
    },
    {
        "entity": "Blaster Repeater",
        "sound": "sounds/blaster_repeater.ogg",
        "crosshair": "sprites/crosshair-repeater.png",
        "cooldown": 0.1,
        "damage": 10,
        "spread": 0.03,
        "shots": 1,
        "kick": 0.012,
    },
]
HOLD = (0.26, -0.3, 0.5)  # where the weapon sits: right, up, forward
WEAPON_SCALE = 0.4
JUMP_SOUNDS = ["sounds/jump_a.ogg", "sounds/jump_b.ogg", "sounds/jump_c.ogg"]


# ----- view math -----------------------------------------------------------


def look_direction(yaw, pitch):
    """The unit vector the camera looks along (yaw 0 looks down -Z)."""
    cp = math.cos(pitch)
    return (-math.sin(yaw) * cp, math.sin(pitch), -math.cos(yaw) * cp)


def view_axes(yaw, pitch):
    """Right, up and forward of the view, in world space."""
    sy, cy = math.sin(yaw), math.cos(yaw)
    sp, cp = math.sin(pitch), math.cos(pitch)
    right = (cy, 0.0, -sy)
    up = (sy * sp, cp, cy * sp)
    return right, up, look_direction(yaw, pitch)


def yaw_pitch_euler(yaw, pitch):
    """Transform rotation (XYZ Euler radians) for "turn by yaw, then tilt
    by pitch": what a camera-held object needs."""
    sy, cy = math.sin(yaw), math.cos(yaw)
    sp, cp = math.sin(pitch), math.cos(pitch)
    return (
        math.atan2(sp, cy * cp),
        math.asin(max(-1.0, min(1.0, sy * cp))),
        math.atan2(-sy * sp, cy),
    )


def offset(origin, axes, amounts):
    """``origin`` moved along each axis by the matching amount."""
    return tuple(
        origin[i] + sum(axis[i] * amount for axis, amount in zip(axes, amounts))
        for i in range(3)
    )


def normalized(v):
    length = math.sqrt(sum(c * c for c in v)) or 1.0
    return tuple(c / length for c in v)


# ----- behaviour -----------------------------------------------------------


def start(entity, ctx):
    s = entity.state
    s["spawn"] = entity.transform.position
    s["yaw"] = entity.attributes.get("yaw", 0.0)
    s["pitch"] = 0.0
    s["vy"] = 0.0
    s["vel"] = (0.0, 0.0)
    s["jumps"] = JUMPS
    s["air"] = 0.0
    s["weapon"] = 0
    s["cooldown"] = 0.0
    s["kick"] = 0.0
    s["swap"] = 0.0
    s["marker"] = 0.0
    s["flash"] = 0.0
    s["walking"] = False
    s["won"] = False
    s["banner"] = 0.0
    if entity.attributes.get("health") is None:
        entity.attributes["health"] = 100
    s["health"] = entity.attributes.get("health")
    ctx.input.lock_mouse()


def respawn(entity):
    s = entity.state
    entity.transform.position = s["spawn"]
    s["yaw"] = entity.attributes.get("yaw", 0.0)
    s["pitch"] = 0.0
    s["vy"] = 0.0
    s["vel"] = (0.0, 0.0)
    entity.attributes["health"] = 100
    s["health"] = 100


def update(entity, ctx):
    s = entity.state
    inp = ctx.input
    dt = ctx.time.delta

    # Mouse capture: Esc frees the cursor, a click takes it back.
    clicked_to_capture = False
    if inp.just_pressed("escape"):
        inp.lock_mouse(False)
    elif not inp.mouse_locked and inp.mouse_just_pressed("left"):
        inp.lock_mouse()
        clicked_to_capture = True

    # Mouse look.
    dx, dy = inp.mouse_delta
    s["yaw"] -= dx * SENSITIVITY
    s["pitch"] = max(-1.5, min(1.5, s["pitch"] - dy * SENSITIVITY))
    yaw, pitch = s["yaw"], s["pitch"]

    walk(entity, ctx, yaw)

    # Health: enemies lower the attribute; a drop flashes the screen.
    health = entity.attributes.get("health", 100)
    if health < s["health"]:
        s["flash"] = 1.0
    s["health"] = health
    x, y, z = entity.transform.position
    if health <= 0 or y < FALL_LIMIT:
        respawn(entity)
        x, y, z = s["spawn"]

    eye = (x, y + EYE, z)
    right, up, forward = view_axes(yaw, pitch)

    # Weapons.
    if inp.just_pressed("e"):
        s["weapon"] = (s["weapon"] + 1) % len(WEAPONS)
        s["swap"] = 0.2
        ctx.audio.play("sounds/weapon_change.ogg", volume=VOLUME)
    s["cooldown"] = max(0.0, s["cooldown"] - dt)
    if inp.mouse_pressed("left") and not clicked_to_capture and s["cooldown"] <= 0.0:
        shoot(entity, ctx, eye, (right, up, forward))

    # The camera rides at eye height.
    cam = entity.camera
    if cam is not None:
        cam.position = eye
        cam.target = tuple(e + f for e, f in zip(eye, forward))

    hold_weapons(entity, ctx, eye, yaw, pitch)
    draw_hud(entity, ctx)


def walk(entity, ctx, yaw):
    """WASD relative to where the player faces, jumping and gravity."""
    s = entity.state
    inp = ctx.input
    dt = ctx.time.delta
    ahead = inp.axis("s", "w") + inp.axis("down", "up")
    side = inp.axis("a", "d") + inp.axis("left", "right")
    fx, fz = -math.sin(yaw), -math.cos(yaw)
    rx, rz = math.cos(yaw), -math.sin(yaw)
    gx, gz = fx * ahead + rx * side, fz * ahead + rz * side
    length = math.hypot(gx, gz)
    if length > 1.0:
        gx, gz = gx / length, gz / length
    vx, vz = s["vel"]
    k = min(1.0, ACCELERATION * dt)
    vx += (gx * SPEED - vx) * k
    vz += (gz * SPEED - vz) * k
    s["vel"] = (vx, vz)

    grounded = ctx.physics.grounded(entity)
    vy = s["vy"]
    if grounded and vy <= 0.0:
        if s["air"] > 0.35:
            ctx.audio.play("sounds/land.ogg", volume=VOLUME)
        s["air"] = 0.0
        s["jumps"] = JUMPS
        vy = -2.0  # keeps the controller pressed onto slopes
    else:
        s["air"] += dt
        vy -= GRAVITY * dt
    if inp.just_pressed("space") and s["jumps"] > 0:
        vy = JUMP
        s["jumps"] -= 1
        ctx.audio.play(random.choice(JUMP_SOUNDS), volume=VOLUME)
    s["vy"] = vy
    entity.character_velocity = (vx, vy, vz)

    # Footsteps loop on the player's own Audio Source while walking.
    walking = grounded and math.hypot(vx, vz) > 1.0
    if walking != s["walking"]:
        s["walking"] = walking
        if walking:
            entity.audio.play()
        else:
            entity.audio.stop()


def shoot(entity, ctx, eye, axes):
    s = entity.state
    weapon = WEAPONS[s["weapon"]]
    right, up, forward = axes
    s["cooldown"] = weapon["cooldown"]
    s["kick"] = 1.0
    s["pitch"] = min(1.5, s["pitch"] + weapon["kick"])
    ctx.audio.play(weapon["sound"], volume=VOLUME, pitch=random.uniform(0.92, 1.08))
    ctx.ai.noise(entity, radius=HEARD_FROM, kind="shot")  # drones hear it
    spread = weapon["spread"]
    for _ in range(weapon["shots"]):
        jitter = (random.uniform(-spread, spread), random.uniform(-spread, spread))
        direction = normalized(offset(forward, (right, up), jitter))
        hit = ctx.physics.raycast(eye, direction, RANGE, ignore=entity)
        if hit is None or hit.entity is None:
            continue
        # Sparks fly off whatever it hit, back along the surface's normal.
        count = 24 // weapon["shots"]
        health = hit.entity.attributes.get("health")
        if health is not None and hit.entity.name != entity.name:
            hit.entity.attributes["health"] = health - weapon["damage"]
            s["marker"] = 0.15
            # Drones spit hot red-orange sparks.
            ctx.particles.burst("sparks", hit.point, hit.normal, count * 2,
                                color=(12.0, 2.8, 0.6, 1.0))
        else:
            ctx.particles.burst("sparks", hit.point, hit.normal, count)


def hold_weapons(entity, ctx, eye, yaw, pitch):
    """Places the held blaster in front of the camera; hides the other."""
    s = entity.state
    dt = ctx.time.delta
    s["kick"] = max(0.0, s["kick"] - dt * 8.0)
    s["swap"] = max(0.0, s["swap"] - dt)
    vx, vz = s["vel"]
    bob = math.sin(ctx.time.elapsed * 10.0) * 0.012 * min(1.0, math.hypot(vx, vz) / SPEED)
    right, up, forward = view_axes(yaw, pitch)
    hold = (
        HOLD[0],
        HOLD[1] + bob - s["swap"] * 2.0,  # dips out of view while switching
        HOLD[2] - s["kick"] * 0.08,
    )
    rotation = yaw_pitch_euler(yaw, pitch)
    for i, weapon in enumerate(WEAPONS):
        model = ctx.scene.find(weapon["entity"])
        if model is None:
            continue
        if i == s["weapon"]:
            model.transform.position = offset(eye, (right, up, forward), hold)
            model.transform.rotation_euler = rotation
        else:
            model.transform.position = (eye[0], -100.0, eye[2])  # put away


def draw_hud(entity, ctx):
    s = entity.state
    hud = ctx.hud
    dt = ctx.time.delta
    w, h = hud.size
    cx, cy = hud.center
    weapon = WEAPONS[s["weapon"]]

    if s["flash"] > 0.0:
        hud.rect(cx, cy, w, h, (0.9, 0.1, 0.05, 0.3 * s["flash"]))
        s["flash"] = max(0.0, s["flash"] - dt * 3.0)

    hud.image(weapon["crosshair"], cx, cy, size=(44, 44))
    if s["marker"] > 0.0:
        hud.image("sprites/hit.png", cx, cy, size=(64, 64), color=(1.0, 1.0, 1.0, 0.9))
        s["marker"] -= dt

    # Health bar, bottom left.
    health = max(0, min(100, s["health"]))
    left, bottom, width = 48.0, h - 48.0, 220.0
    hud.rect(left + width / 2, bottom, width + 8, 26, (0.0, 0.0, 0.0, 0.55))
    fill = width * health / 100.0
    if fill > 0:
        color = (0.95, 0.95, 0.92, 1.0) if health > 30 else (1.0, 0.3, 0.2, 1.0)
        hud.rect(left + fill / 2, bottom, fill, 18, color)
    hud.text(f"{health:.0f}%", left + width + 20, bottom, size=26, shadow=True)

    # One pip per drone still flying, top right.
    drones = sum(1 for e in ctx.scene if (e.name or "").startswith("Enemy"))
    for i in range(drones):
        hud.rect(w - 48 - i * 22, 48, 14, 14, (0.95, 0.95, 0.92, 0.9))
    if drones == 0 and not s["won"]:
        s["won"] = True
        s["banner"] = 4.0
        ctx.log("All drones down. Nice shooting!")
    if s["banner"] > 0.0:
        fade = min(1.0, s["banner"])
        hud.text("ARENA CLEAR", cx, cy - 120, size=56, color=(0.95, 0.95, 0.92, fade),
                 align="center", outline=(0.0, 0.0, 0.0, 0.8 * fade))
        s["banner"] -= dt
