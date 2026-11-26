"""Tests for the FPS example.

    pip install pytest
    pytest examples/oberon/fps/tests

The first group plays the real scene headlessly (``target_fps=60`` so game
time passes like it would on screen). The second drives the scripts with a
fake context, which is how to test shooting without a mouse.
"""

import math
import os
import shutil

import pytest

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))
player_script = load_script(os.path.join(GAME, "scripts", "player.py"))
enemy_script = load_script(os.path.join(GAME, "scripts", "enemy.py"))


def project_copy(tmp_path, **changes):
    """The project in a temp folder, with some build_level lists replaced."""
    for folder in ("scripts", "models", "sounds", "sprites"):
        shutil.copytree(os.path.join(GAME, folder), tmp_path / folder)
    saved = {name: list(getattr(build_level, name)) for name in changes}
    try:
        for name, value in changes.items():
            getattr(build_level, name)[:] = value
        build_level.build().save(str(tmp_path / "level.ron"))
    finally:
        for name, value in saved.items():
            getattr(build_level, name)[:] = value
    return str(tmp_path / "level.ron")


# --------------------------------------------------------------------------
# The real scene, headless
# --------------------------------------------------------------------------


NUMBER = r"-?\d+\.\d+(?:[eE][-+]?\d+)?"


def same_scene_text(a, b):
    """Scene files equal up to float noise: rotations go through a
    quaternion when saved, and the last digit depends on the platform's
    maths library. Everything but the numbers must match exactly."""
    import re

    if re.split(NUMBER, a) != re.split(NUMBER, b):
        return False
    xs, ys = re.findall(NUMBER, a), re.findall(NUMBER, b)
    return all(abs(float(x) - float(y)) <= 1e-5 for x, y in zip(xs, ys))


def test_the_saved_level_matches_build_level(tmp_path):
    path = tmp_path / "fresh.ron"
    build_level.build().save(str(path))
    with open(LEVEL, encoding="utf-8") as saved:
        assert same_scene_text(path.read_text(encoding="utf-8"), saved.read()), (
            "level.ron is stale: run build_level.py"
        )


def test_level_loads_and_runs():
    assert at.run_scene(LEVEL, headless=True, max_frames=120) == 120


def test_the_player_stands_on_the_platform_and_holds_a_blaster():
    seen = {}

    def after(ctx):
        seen["player"] = ctx.scene["Player"].transform.position
        seen["blaster"] = ctx.scene["Blaster"].transform.position
        seen["repeater"] = ctx.scene["Blaster Repeater"].transform.position
        seen["camera"] = ctx.scene["Player"].camera_3d.position

    at.run_scene(LEVEL, headless=True, max_frames=90, target_fps=60, update=after)
    x, y, z = seen["player"]
    assert 0.6 < y < 1.2, f"the player fell or floats: {seen['player']}"
    eye = seen["camera"]
    assert eye[1] == pytest.approx(y + player_script.EYE, abs=0.05)
    assert math.dist(seen["blaster"], eye) < 1.0, "the blaster is held in front of the eye"
    assert seen["repeater"][1] < -50, "the other weapon is put away"


def test_drones_bob_and_face_the_player():
    heights, yaws = [], []

    def after(ctx):
        drone = ctx.scene["Enemy 2"]  # starts at (-9.5, 2.5, 1.5), west of the player
        heights.append(drone.transform.position[1])
        yaws.append(drone.transform.rotation_euler)

    at.run_scene(LEVEL, headless=True, max_frames=90, target_fps=60, update=after)
    assert max(heights) - min(heights) > 0.05, "drones bob up and down"
    # Facing east (+X) toward the player: turned -90 degrees about Y.
    rx, ry, rz = yaws[-1]
    facing = (-math.sin(ry), math.sin(rx) * math.cos(ry), -math.cos(rx) * math.cos(ry))
    assert facing[0] > 0.8, f"the drone should look toward +X, looks along {facing}"


def test_a_close_drone_shoots_the_player(tmp_path):
    level = project_copy(tmp_path, ENEMIES=[((0.0, 1.0, -3.0), 0)])
    health = []

    def after(ctx):
        health.append(ctx.scene["Player"].attributes.get("health"))

    at.run_scene(level, headless=True, max_frames=60, target_fps=60, update=after)
    assert health[0] == 100
    assert 70 <= health[-1] <= 90, health[-1]


def test_far_drones_hold_fire():
    health = []

    def after(ctx):
        health.append(ctx.scene["Player"].attributes.get("health"))

    at.run_scene(LEVEL, headless=True, max_frames=60, target_fps=60, update=after)
    assert health[-1] == 100


def test_falling_off_the_world_respawns():
    seen = []

    def after(ctx):
        player = ctx.scene["Player"]
        if ctx.time.frame == 10:
            # Off the edge, and hurt: a respawn restores both.
            player.transform.position = (40.0, -20.0, 40.0)
            player.attributes["health"] = 50
        seen.append((player.transform.position, player.attributes.get("health")))

    at.run_scene(LEVEL, headless=True, max_frames=20, update=after)
    (x, y, z), health = seen[-1]
    assert math.dist((x, z), (0.0, 0.0)) < 0.5 and y > 0.5, f"back at the spawn point: {seen[-1]}"
    assert health == 100


def test_a_shot_drone_chases_the_player_around_the_walls():
    """Enemy 4 starts 17 m away on the high platform, behind a wall. Hurt,
    it flies the navigation mesh round the wall and down the platforms
    (jump links) until it's in range, then fires."""
    seen = []

    def after(ctx):
        drone = ctx.scene.find("Enemy 4")
        player = ctx.scene["Player"]
        if ctx.time.frame == 5:
            drone.attributes["health"] = 90
        nav = drone.nav_state
        seen.append(
            (
                drone.transform.position,
                nav["jmp"],
                math.dist(drone.transform.position, player.transform.position),
                player.attributes.get("health"),
            )
        )

    at.run_scene(LEVEL, headless=True, max_frames=480, target_fps=60, update=after)
    assert seen[0][2] > 15
    assert any(jumping for _, jumping, _, _ in seen), "it took a jump link down"
    position, _, distance, health = seen[-1]
    assert distance < enemy_script.RANGE, f"it caught up: {position}"
    assert health < 100, "and opened fire"
    # The straight line from its start to the player crosses the tall wall
    # at (11.5, 3, -5.5); it went round it (to the north-east side).
    assert all(math.dist(p[::2], (11.5, -5.5)) > 0.6 for p, _, _, _ in seen)


def test_the_navigation_mesh_links_every_platform():
    reachable = []

    def after(ctx):
        if ctx.time.frame == 1:
            for position, _ in build_level.ENEMIES:
                start = ctx.physics.nav_nearest(position, 5.0)
                reachable.append(ctx.physics.nav_path(start, build_level.SPAWN, False) is not None)

    at.run_scene(LEVEL, headless=True, max_frames=2, update=after)
    assert reachable == [True] * len(build_level.ENEMIES)


def test_a_drone_at_zero_health_is_destroyed(tmp_path):
    level = project_copy(tmp_path, ENEMIES=[((0.0, 3.0, -8.0), 0)])
    alive = []

    def after(ctx):
        drone = ctx.scene.find("Enemy 1")
        if drone is not None and ctx.time.frame == 5:
            drone.attributes["health"] = 0
        alive.append(drone is not None)

    at.run_scene(level, headless=True, max_frames=12, update=after)
    assert alive[0] and not alive[-1]


# --------------------------------------------------------------------------
# Script logic with a fake context
# --------------------------------------------------------------------------


class FakeTransform:
    def __init__(self, position=(0.0, 0.0, 0.0)):
        self.position = position
        self.rotation_euler = (0.0, 0.0, 0.0)


class FakeAudio:
    def __init__(self):
        self.played = []
        self.source = []

    def play(self, path=None, volume=1.0, pitch=1.0, position=None):
        if path is None:
            self.source.append("play")
        else:
            self.played.append(path)

    def stop(self):
        self.source.append("stop")


class FakeEntity:
    def __init__(self, name, position=(0.0, 0.0, 0.0), attributes=None):
        self.name = name
        self.transform = FakeTransform(position)
        self.attributes = dict(attributes or {})
        self.state = {}
        self.camera = None
        self.character_velocity = (0.0, 0.0, 0.0)
        self.audio = FakeAudio()
        self.despawned = False
        self.nav = None

    def despawn(self):
        self.despawned = True


class FakeHit:
    def __init__(self, entity, distance=5.0):
        self.entity = entity
        self.point = (0.0, 0.0, -distance)
        self.normal = (0.0, 0.0, 1.0)
        self.distance = distance


class FakePhysics:
    def __init__(self, hit=None, grounded=True):
        self.hit = hit
        self.on_ground = grounded
        self.rays = []

    def raycast(self, origin, direction, max_distance=100.0, ignore=None):
        self.rays.append((tuple(origin), tuple(direction), ignore))
        return self.hit

    def grounded(self, entity):
        return self.on_ground


class FakeInput:
    def __init__(self, held=(), just=(), buttons=(), delta=(0.0, 0.0)):
        self.held, self.just, self.buttons = set(held), set(just), set(buttons)
        self.mouse_delta = delta
        self.mouse_locked = True

    def pressed(self, key):
        return key in self.held

    def just_pressed(self, key):
        return key in self.just

    def axis(self, negative, positive):
        return float(self.pressed(positive)) - float(self.pressed(negative))

    def mouse_pressed(self, button="left"):
        return button in self.buttons

    def mouse_just_pressed(self, button="left"):
        return button in self.buttons

    def lock_mouse(self, locked=True):
        self.mouse_locked = locked


class FakeHud:
    size = (1280, 720)
    center = (640.0, 360.0)

    def __init__(self):
        self.images, self.rects, self.texts = [], [], []

    def image(self, path, x, y, size=None, color=(1, 1, 1, 1), rotation=0.0):
        self.images.append((path, x, y))

    def rect(self, x, y, w, h, color=(1, 1, 1, 1)):
        self.rects.append((x, y, w, h, color))

    def text(self, text, x, y, size=24.0, color=(1, 1, 1, 1), align="left", shadow=None,
             outline=None):
        self.texts.append((text, x, y, align))


class FakeTime:
    delta = 1.0 / 60.0
    elapsed = 1.0


class FakeScene:
    def __init__(self, *entities):
        self.entities = {e.name: e for e in entities}

    def find(self, name):
        return self.entities.get(name)

    def __iter__(self):
        return iter(list(self.entities.values()))


class FakeParticles:
    def __init__(self):
        self.bursts = []

    def burst(self, effect, position, direction=None, count=None, **fields):
        self.bursts.append((effect, tuple(position), direction, count, fields))


class FakeCtx:
    def __init__(self, scene, inp=None, physics=None):
        self.scene = scene
        self.input = inp or FakeInput()
        self.physics = physics or FakePhysics()
        self.particles = FakeParticles()
        self.audio = FakeAudio()
        self.hud = FakeHud()
        self.time = FakeTime()
        self.logs = []

    def log(self, *parts):
        self.logs.append(" ".join(str(p) for p in parts))


def fps_world(drone_health=100):
    player = FakeEntity("Player", (0.0, 1.0, 0.0), {"health": 100})
    drone = FakeEntity("Enemy 1", (0.0, 2.0, -6.0), {"health": drone_health})
    weapons = [FakeEntity("Blaster"), FakeEntity("Blaster Repeater")]
    return player, drone, FakeScene(player, drone, *weapons)


def test_view_math_agrees_with_itself():
    for yaw, pitch in [(0.0, 0.0), (0.7, 0.3), (-2.0, -1.1), (math.pi / 2, 0.0)]:
        right, up, forward = player_script.view_axes(yaw, pitch)
        # An orthonormal, right-handed basis whose forward is the look direction.
        assert sum(r * f for r, f in zip(right, forward)) == pytest.approx(0.0, abs=1e-9)
        assert sum(u * f for u, f in zip(up, forward)) == pytest.approx(0.0, abs=1e-9)
        assert forward == pytest.approx(player_script.look_direction(yaw, pitch))
        # The Euler angles rotate -Z onto the look direction (XYZ order).
        rx, ry, rz = player_script.yaw_pitch_euler(yaw, pitch)
        fwd = (-math.sin(ry), math.sin(rx) * math.cos(ry), -math.cos(rx) * math.cos(ry))
        assert fwd == pytest.approx(forward, abs=1e-9)


def test_shooting_a_drone_lowers_its_health_and_plays_the_blaster():
    player, drone, scene = fps_world()
    physics = FakePhysics(hit=FakeHit(drone))
    ctx = FakeCtx(scene, FakeInput(buttons={"left"}), physics)
    player_script.start(player, ctx)
    player_script.update(player, ctx)
    blaster = player_script.WEAPONS[0]
    assert len(physics.rays) == blaster["shots"]
    assert all(ignore is player for _, _, ignore in physics.rays)
    assert drone.attributes["health"] == 100 - blaster["shots"] * blaster["damage"]
    assert blaster["sound"] in ctx.audio.played
    assert ("sprites/hit.png", 640.0, 360.0) in ctx.hud.images
    # Every pellet throws sparks off the drone, back along the hit normal.
    assert len(ctx.particles.bursts) == blaster["shots"]
    for effect, position, normal, count, fields in ctx.particles.bursts:
        assert effect == "sparks" and position == (0.0, 0.0, -5.0)
        assert normal == (0.0, 0.0, 1.0) and count > 0 and "color" in fields
    # The cooldown stops the next frame from firing again.
    player_script.update(player, ctx)
    assert len(physics.rays) == blaster["shots"]


def test_e_switches_to_the_repeater():
    player, _, scene = fps_world()
    ctx = FakeCtx(scene, FakeInput(just={"e"}))
    player_script.start(player, ctx)
    player_script.update(player, ctx)
    assert player.state["weapon"] == 1
    assert "sounds/weapon_change.ogg" in ctx.audio.played
    crosshairs = [p for p, _, _ in ctx.hud.images if "crosshair" in p]
    assert crosshairs == ["sprites/crosshair-repeater.png"]
    assert scene.find("Blaster").transform.position[1] < -50


def test_mouse_looks_and_wasd_walks_where_you_look():
    player, _, scene = fps_world()
    ctx = FakeCtx(scene, FakeInput(held={"w"}, delta=(-700 * math.pi / 2, 0.0)))
    player_script.start(player, ctx)
    for _ in range(30):
        player_script.update(player, ctx)
        ctx.input.mouse_delta = (0.0, 0.0)
    # A quarter turn left: forward is now -X.
    vx, vy, vz = player.character_velocity
    assert vx < -4.0 and abs(vz) < 0.5, player.character_velocity
    assert player.audio.source == ["play"], "footsteps start once"


def test_space_jumps_twice_then_not_again():
    player, _, scene = fps_world()
    physics = FakePhysics(grounded=True)
    ctx = FakeCtx(scene, FakeInput(just={"space"}), physics)
    player_script.start(player, ctx)
    player_script.update(player, ctx)
    assert player.character_velocity[1] == pytest.approx(player_script.JUMP)
    physics.on_ground = False
    player_script.update(player, ctx)  # the air jump
    assert player.character_velocity[1] == pytest.approx(player_script.JUMP)
    player_script.update(player, ctx)  # out of jumps: gravity wins
    assert player.character_velocity[1] < player_script.JUMP
    jumps = [p for p in ctx.audio.played if "jump" in p]
    assert len(jumps) == 2


def test_the_hud_shows_health_and_drones_left():
    player, _, scene = fps_world()
    ctx = FakeCtx(scene)
    player_script.start(player, ctx)
    player.attributes["health"] = 40
    player_script.update(player, ctx)
    fills = [r for r in ctx.hud.rects if r[3] == 18]
    assert len(fills) == 1 and fills[0][2] == pytest.approx(220 * 0.4)
    pips = [r for r in ctx.hud.rects if r[2] == 14]
    assert len(pips) == 1, "one drone left"
    assert any(r[2] == 1280 for r in ctx.hud.rects), "a damage flash covers the screen"
    assert [t[0] for t in ctx.hud.texts] == ["40%"], "the health label"


def test_clearing_every_drone_is_announced_once():
    player = FakeEntity("Player", (0.0, 1.0, 0.0), {"health": 100})
    ctx = FakeCtx(FakeScene(player))
    player_script.start(player, ctx)
    player_script.update(player, ctx)
    player_script.update(player, ctx)
    assert len(ctx.logs) == 1 and "drones down" in ctx.logs[0]
    banners = [t for t in ctx.hud.texts if t[0] == "ARENA CLEAR"]
    assert banners and banners[0][3] == "center", "a banner announces it too"


def test_a_hurt_drone_cries_out_and_a_dead_one_explodes():
    player, drone, scene = fps_world()
    ctx = FakeCtx(scene)
    enemy_script.start(drone, ctx)
    drone.attributes["health"] = 75
    enemy_script.update(drone, ctx)
    assert ctx.audio.played == ["sounds/enemy_hurt.ogg"]
    assert not drone.despawned
    drone.attributes["health"] = 0
    enemy_script.update(drone, ctx)
    assert ctx.audio.played[-1] == "sounds/enemy_destroy.ogg"
    assert drone.despawned


class FakeNav:
    def __init__(self):
        self.target = None
        self.hover = 1.5

    def go(self, target):
        self.target = target

    def stop(self):
        self.target = None


def test_a_drone_notices_a_close_player_and_gives_up_on_a_far_one():
    player, drone, scene = fps_world()
    drone.nav = FakeNav()
    drone.transform.position = (0.0, 2.0, -8.0)
    ctx = FakeCtx(scene)
    enemy_script.start(drone, ctx)
    enemy_script.update(drone, ctx)
    assert drone.nav.target is None, "8 m away: not noticed yet"
    drone.transform.position = (0.0, 2.0, -4.0)
    enemy_script.update(drone, ctx)
    assert drone.nav.target is player, "close: it gives chase"
    drone.transform.position = (0.0, 2.0, -30.0)
    enemy_script.update(drone, ctx)
    assert drone.nav.target is None, "far away: it gives up"
    assert drone.state["home"][2] == -30.0, "and bobs where it stopped"


def test_a_shot_drone_gives_chase():
    player, drone, scene = fps_world()
    drone.nav = FakeNav()
    ctx = FakeCtx(scene)
    enemy_script.start(drone, ctx)
    drone.transform.position = (0.0, 2.0, -12.0)
    drone.attributes["health"] = 75
    enemy_script.update(drone, ctx)
    assert drone.nav.target is player
    assert drone.nav.hover > 1.5, "it bobs by changing its hover height"


def test_drones_only_fire_with_a_clear_line_of_sight():
    player, drone, scene = fps_world()
    drone.transform.position = (0.0, 1.0, -3.0)
    wall = FakeEntity("Wall Low 1")
    ctx = FakeCtx(scene, physics=FakePhysics(hit=FakeHit(wall, 1.5)))
    enemy_script.start(drone, ctx)
    drone.state["timer"] = 0.0
    enemy_script.update(drone, ctx)
    assert player.attributes["health"] == 100, "the wall blocks the shot"
    ctx.physics.hit = FakeHit(player, 3.0)
    drone.state["timer"] = 0.0
    enemy_script.update(drone, ctx)
    assert player.attributes["health"] == 100 - enemy_script.DAMAGE
    assert ctx.audio.played == ["sounds/enemy_attack.ogg"]


# --------------------------------------------------------------------------
# The tutorial quotes these scripts
# --------------------------------------------------------------------------

TUTORIAL = os.path.join(GAME, "..", "..", "..", "docs", "wiki", "tutorials", "fps.md")


def test_the_tutorial_quotes_the_scripts_as_they_are():
    """Every ``# scripts/x.py`` block in fps.md appears verbatim in that file."""
    import re

    with open(TUTORIAL, encoding="utf-8") as f:
        blocks = re.findall(r"```python\n# (scripts/\w+\.py)\n(.*?)```", f.read(), re.S)
    assert len(blocks) >= 8
    for script, excerpt in blocks:
        with open(os.path.join(GAME, script), encoding="utf-8") as f:
            source = f.read()
        assert excerpt in source, f"{script} no longer contains:\n{excerpt}"
