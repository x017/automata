"""enemy: a flying drone with a mind of its own.

It bobs in place and looks around (``idle``) until it sees the player
(a cone in front of it, blocked by walls), hears a blaster shot, or gets
hit. Then it chases (``chase``): its Nav Agent flies it round the walls,
holding back at firing range, and it fires whenever it has a clear shot.
If it loses sight of the player for a while, it flies to where it last
saw or heard them and looks around (``search``) before settling down
again. The three moods are a state machine (``ctx.ai.state_machine``).

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
SIGHT = 14.0  # how far a drone can see (m)
FOV = 110.0  # how wide it looks (degrees)
LOOK_AROUND = 0.7  # how far it turns each way while looking around (radians)
FORGET = 3.0  # seconds out of sight before it goes looking
SEARCH_FOR = 3.0  # seconds it looks around where the player was
GIVE_UP = 25.0  # how far away the player must get to be forgotten


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
    rx, ry, _ = entity.transform.rotation_euler
    s["yaw"] = math.atan2(math.sin(ry), math.cos(rx) * math.cos(ry))  # the way it faces
    s["facing"] = s["yaw"]
    s["sees"] = False
    s["unseen"] = 0.0
    s["clue"] = None  # where the player was last seen or heard (a point)
    if entity.nav is not None:
        s["hover"] = entity.nav.hover
        s["hold_back"] = entity.nav.stop_distance
    s["brain"] = ctx.ai.state_machine(
        {
            "idle": {"enter": settle, "update": idle},
            "chase": chase,
            "search": {"enter": start_search, "update": search},
        },
        initial="idle",
    )


def update(entity, ctx):
    s = entity.state
    dt = ctx.time.delta
    player = ctx.scene.find("Player")
    health = entity.attributes.get("health", 100)
    position = entity.transform.position
    if health < s["health"]:
        if health <= 0:
            ctx.audio.play("sounds/enemy_destroy.ogg", volume=VOLUME, position=position)
            entity.despawn()
            return
        ctx.audio.play("sounds/enemy_hurt.ogg", volume=VOLUME, position=position)
        if player is not None:
            s["clue"] = player.transform.position  # go and look for the shooter
    s["health"] = health

    # Bob up and down around the starting height.
    s["t"] += dt
    s["y"] += math.cos(s["t"] * 5.0) * BOB * dt

    if player is None:
        return
    # Senses: a cone of sight blocked by walls, and ears for blaster shots.
    s["sees"] = ctx.ai.can_see(entity, player, fov=FOV, distance=SIGHT, eye=CENTER, aim=0.5)
    if s["sees"]:
        s["unseen"] = 0.0
        s["clue"] = player.transform.position
    else:
        s["unseen"] += dt
    for noise in ctx.ai.hear(entity, kinds="shot"):
        s["clue"] = noise.position

    s["brain"].update(entity, ctx)
    fire(entity, ctx, player)


def bob(entity):
    """How far the bob has it above or below its resting height."""
    s = entity.state
    return s["y"] - s["home"][1]


def look(entity, yaw, pitch=0.0):
    entity.state["facing"] = yaw
    entity.transform.rotation_euler = yaw_pitch_euler(yaw, pitch)


def settle(entity, ctx):
    """Entering idle: stop, and rest wherever it is now."""
    s = entity.state
    position = entity.transform.position
    if entity.nav is not None and entity.nav.target is not None:
        entity.nav.stop()
        s["home"] = (position[0], position[1] - bob(entity), position[2])
        s["y"] = position[1]
        s["yaw"] = s["facing"]
    s["clue"] = None


def idle(entity, ctx):
    """Bob in place and look slowly from side to side."""
    s = entity.state
    hx, _, hz = s["home"]
    entity.transform.position = (hx, s["y"], hz)
    look(entity, s["yaw"] + math.sin(s["t"] * 0.8) * LOOK_AROUND)
    if s["sees"]:
        return "chase"
    if s["clue"] is not None:
        return "search"


def fly_to(entity, target):
    """The Nav Agent flies it there, bobbing by changing its hover height.
    Without one, it stays put."""
    s = entity.state
    if entity.nav is None:
        hx, _, hz = s["home"]
        entity.transform.position = (hx, s["y"], hz)
        return
    entity.nav.go(target)
    entity.nav.hover = s["hover"] + bob(entity)


def chase(entity, ctx):
    """After the player, the shortest way round the walls, stopping short
    at firing range; eyes on them."""
    s = entity.state
    player = ctx.scene.find("Player")
    if entity.nav is not None:
        # Hold back at firing range while it has a clear shot; otherwise
        # keep coming, round the wall in the way.
        entity.nav.stop_distance = s["hold_back"] if s["sees"] else 0.5
    fly_to(entity, player)
    position = entity.transform.position
    px, py, pz = player.transform.position
    d = (px - position[0], py + 0.5 - position[1] - CENTER, pz - position[2])
    look(entity, math.atan2(-d[0], -d[2]), math.atan2(d[1], math.hypot(d[0], d[2])))
    if math.dist(position, (px, py, pz)) > GIVE_UP:
        return "idle"
    if s["unseen"] > FORGET:
        return "search"  # to where it last saw (or heard) them


def start_search(entity, ctx):
    entity.state["looked"] = 0.0
    entity.state["yaw"] = entity.state["facing"]


def search(entity, ctx, brain):
    """Fly to the clue, then look around there for a while."""
    s = entity.state
    if s["sees"]:
        return "chase"
    if s["clue"] is not None:
        fly_to(entity, s["clue"])
    nav = entity.nav
    moving = nav is not None and not nav.arrived
    if moving:
        vx, _, vz = nav.velocity
        if math.hypot(vx, vz) > 0.3:
            s["yaw"] = math.atan2(-vx, -vz)  # face the way it's flying
        s["looked"] = 0.0
    else:
        s["looked"] += ctx.time.delta
    look(entity, s["yaw"] + math.sin(s["t"] * 2.0) * LOOK_AROUND * (0.0 if moving else 1.0))
    if s["looked"] > SEARCH_FOR or brain.time > 20.0:
        return "idle"


def fire(entity, ctx, player):
    """Fire on a timer when the player is close and in plain sight."""
    s = entity.state
    position = entity.transform.position
    center = (position[0], position[1] + CENTER, position[2])
    px, py, pz = player.transform.position
    d = [t - c for t, c in zip((px, py + 0.5, pz), center)]
    s["timer"] -= ctx.time.delta
    if s["timer"] > 0.0 or not s["sees"]:
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
