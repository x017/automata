"""enemy: a flying drone that bobs in place, keeps its eyes on the player
and fires when the player comes close and in sight. Once it notices the
player (they come near, or shoot it) its Nav Agent chases them around the
walls, holding back at firing range, until they get far away.

Attributes (Inspector): health (Int), 100 at the start.

player.py lowers ``health`` when its shots land; this script notices the
drop, plays the hurt sound, and blows the drone up at zero.
"""

import math
import random

RANGE = 6.0  # how close the player must be to get shot at
FIRE_EVERY = 0.25  # seconds between shots
DAMAGE = 5
BOB = 1.0  # bob speed scale (Kenney's kit: y += cos(t * 5) * dt)
CENTER = 0.45  # the drone's middle, above its origin
VOLUME = 0.45
NOTICE = 6.5  # how close the player must come to be noticed
GIVE_UP = 20.0  # how far away they must get to be forgotten


def yaw_pitch_euler(yaw, pitch):
    """Transform rotation (XYZ Euler radians) for "turn by yaw, then tilt
    by pitch" (same as player.py)."""
    sy, cy = math.sin(yaw), math.cos(yaw)
    sp, cp = math.sin(pitch), math.cos(pitch)
    return (
        math.atan2(sp, cy * cp),
        math.asin(max(-1.0, min(1.0, sy * cp))),
        math.atan2(-sy * sp, cy),
    )


def start(entity, ctx):
    s = entity.state
    s["home"] = entity.transform.position
    s["y"] = s["home"][1]
    s["t"] = 0.0
    s["timer"] = random.uniform(0.0, FIRE_EVERY)
    if entity.attributes.get("health") is None:
        entity.attributes["health"] = 100
    s["health"] = entity.attributes.get("health")
    s["chasing"] = False
    if entity.nav is not None:
        s["hover"] = entity.nav.hover


def update(entity, ctx):
    s = entity.state
    dt = ctx.time.delta
    if entity.attributes.get("health", 100) < s["health"]:
        s["chasing"] = True  # shot: come after the shooter

    health = entity.attributes.get("health", 100)
    position = entity.transform.position
    if health < s["health"]:
        if health <= 0:
            ctx.audio.play("sounds/enemy_destroy.ogg", volume=VOLUME, position=position)
            entity.despawn()
            return
        ctx.audio.play("sounds/enemy_hurt.ogg", volume=VOLUME, position=position)
    s["health"] = health

    # Bob up and down around the starting height.
    s["t"] += dt
    s["y"] += math.cos(s["t"] * 5.0) * BOB * dt
    bob = s["y"] - s["home"][1]

    player = ctx.scene.find("Player")
    if player is None:
        return
    px, py, pz = player.transform.position
    away = math.dist(position, (px, py, pz))
    if away < NOTICE:
        s["chasing"] = True
    elif away > GIVE_UP:
        s["chasing"] = False

    if s["chasing"] and entity.nav is not None:
        # The Nav Agent flies it: the shortest way round the walls, stopping
        # short at firing range. Bobbing is its hover height going up and down.
        entity.nav.go(player)
        entity.nav.hover = s["hover"] + bob
        hx, y, hz = position
    else:
        if entity.nav is not None and entity.nav.target is not None:
            # Lost interest: stop, and bob where it is from now on.
            entity.nav.stop()
            s["home"] = (position[0], position[1] - bob, position[2])
            s["y"] = position[1]
        hx, _, hz = s["home"]
        y = s["y"]
        entity.transform.position = (hx, y, hz)
    center = (hx, y + CENTER, hz)
    target = (px, py + 0.5, pz)
    d = [t - c for t, c in zip(target, center)]
    flat = math.hypot(d[0], d[2])
    yaw = math.atan2(-d[0], -d[2])
    pitch = math.atan2(d[1], flat)
    entity.transform.rotation_euler = yaw_pitch_euler(yaw, pitch)

    # Fire on a timer when the player is close and nothing is in the way.
    s["timer"] -= dt
    if s["timer"] > 0.0:
        return
    s["timer"] = FIRE_EVERY
    distance = math.sqrt(sum(c * c for c in d))
    if distance > RANGE or distance < 1e-4:
        return
    direction = tuple(c / distance for c in d)
    hit = ctx.physics.raycast(center, direction, RANGE, ignore=entity)
    if hit is not None and hit.entity is not None and hit.entity.name == "Player":
        ctx.audio.play("sounds/enemy_attack.ogg", volume=VOLUME, position=center)
        player.attributes["health"] = player.attributes.get("health", 100) - DAMAGE
