#!/usr/bin/env python3
"""NEON BREACH — a small first-person shooter built with AUTOMATA's OOP API.

A wave-based FPS prototype with a blockout arena, first-person movement,
hitscan rifle, pursuing robot targets, a blocky weapon model, geometric HUD,
and synthesized sound effects.

Controls
--------
WASD          Move
Mouse         Look around (when the window reports cursor motion)
Arrow keys    Look around (keyboard fallback)
Space         Jump
Left mouse/F  Fire
R             Reload
Escape        Quit

Run from the repository root after installing the Python bindings:

    python examples/python/oop_fps.py
    python examples/python/oop_fps.py --headless  # smoke-test the OOP game loop

For audible output, build/install the bindings with the optional audio backend:

    maturin develop --manifest-path crates/automata-python/Cargo.toml --features audio-cpal

The game still runs with the engine's silent audio backend when that feature
isn't enabled.
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

import automata as at

ROOT = Path(__file__).resolve().parent
SOUNDS = ROOT / "assets"

# A compact, deliberately low-poly palette for the whole arena.
FLOOR = (0.12, 0.19, 0.27, 1.0)
WALL = (0.16, 0.25, 0.34, 1.0)
PANEL = (0.22, 0.34, 0.43, 1.0)
CYAN = (0.12, 0.88, 1.0, 1.0)
AMBER = (1.0, 0.57, 0.12, 1.0)
BOT_RED = (0.72, 0.14, 0.19, 1.0)
BOT_DARK = (0.24, 0.30, 0.38, 1.0)
RIFLE = (0.075, 0.095, 0.13, 1.0)

PLAYER_SPEED = 5.4
JUMP_SPEED = 6.5
GRAVITY = 19.0
STARTING_BOTS = [(-5.0, 1.0, -6.0), (0.0, 1.0, -6.5), (5.0, 1.0, -6.0)]


def _cube(name, position, scale, color, *, body=None, collider=None):
    """Build one cube-shaped scene object; shared by map archetypes."""
    entity = at.Entity(name)
    entity.add(at.Transform(position=position, scale=scale))
    entity.add(at.MeshRenderer.cube(size=1.0, color=color))
    if body is not None:
        entity.add(body)
    if collider is not None:
        entity.add(collider)
    return entity


def _add_box(scene, name, position, scale, color, *, solid=True):
    body = at.RigidBody.fixed() if solid else None
    collider = at.Collider3D.cuboid(*scale) if solid else None
    scene.add(_cube(name, position, scale, color, body=body, collider=collider))


class ArenaCamera(at.Entity):
    """The movable first-person camera, aimed down the arena on spawn."""

    def __init__(self):
        super().__init__("camera")
        self.add(at.Camera3D(position=(0.0, 1.55, 7.2), target=(0.0, 1.45, 6.2), fov=76.0))


class Player(at.Entity):
    """A hidden capsule driven by the engine's character controller."""

    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(0.0, 0.98, 7.2)))
        self.add(at.RigidBody.kinematic_velocity())
        self.add(at.Collider3D.capsule(0.48, 0.34))
        self.add(at.CharacterController(snap_to_ground=0.35, autostep=0.35))
        self.add(at.CharacterVelocity())


class ArenaProp(at.Entity):
    """A reusable fixed cover block in the greybox arena."""

    def __init__(self, name, position, scale, color=PANEL):
        super().__init__(name)
        self.add(at.Transform(position=position, scale=scale))
        self.add(at.MeshRenderer.cube(size=1.0, color=color))
        self.add(at.RigidBody.fixed())
        self.add(at.Collider3D.cuboid(*scale))


class BotPart(at.Entity):
    """A decorative, non-physical piece of a blocky robot model."""

    def __init__(self, name, position, scale, color):
        super().__init__(name)
        self.add(at.Transform(position=position, scale=scale))
        self.add(at.MeshRenderer.cube(size=1.0, color=color))


class Robot(at.Entity):
    """One enemy robot: a kinematic target plus a few cube-model details."""

    def __init__(self, name, position):
        super().__init__(name)
        self.position = position
        self.parts = []
        self.add(at.Transform(position=position, scale=(0.72, 1.05, 0.48)))
        self.add(at.MeshRenderer.cube(size=1.0, color=BOT_DARK))
        self.add(at.RigidBody.kinematic_position())
        # Slightly taller than the visible torso so a level head-height shot
        # still intersects the target's collider reliably.
        self.add(at.Collider3D.cuboid(0.72, 1.35, 0.48))

        x, y, z = position
        self.parts.extend(
            [
                BotPart(f"{name}_head", (x, y + 0.75, z), (0.52, 0.42, 0.48), BOT_RED),
                BotPart(f"{name}_visor", (x, y + 0.77, z - 0.25), (0.34, 0.10, 0.035), CYAN),
                BotPart(f"{name}_shoulders", (x, y + 0.24, z), (1.02, 0.25, 0.55), BOT_RED),
            ]
        )

    def stage_into(self, scene):
        """Add the complete robot archetype to a staged or live scene."""
        scene.add(self)
        for part in self.parts:
            scene.add(part)

    def spawn_into(self, scene_view):
        """Queue the complete archetype for spawning during an update."""
        scene_view.spawn(self)
        for part in self.parts:
            scene_view.spawn(part)


class RiflePiece(at.Entity):
    """One primitive in the camera-relative low-poly rifle model."""

    def __init__(self, name, scale, color):
        super().__init__(name)
        self.local_scale = scale
        self.add(at.Transform(scale=scale))
        self.add(at.MeshRenderer.cube(size=1.0, color=color))


class Arena(at.Scene):
    """A small enclosed training yard, assembled from reusable entities."""

    def __init__(self):
        super().__init__()
        self.add(ArenaCamera())
        self.add(Player())
        self.add(
            at.Entity("sun").add(
                at.DirectionalLight(
                    direction=(-0.45, -1.0, -0.25),
                    color=(0.63, 0.82, 1.0),
                    ambient=0.40,
                    shadows=True,
                )
            )
        )

        # Floor and perimeter walls make a simple, fully collidable arena.
        _add_box(self, "floor", (0.0, -0.30, 0.0), (24.0, 0.60, 20.0), FLOOR)
        _add_box(self, "wall_back", (0.0, 2.5, -10.0), (24.0, 5.0, 0.5), WALL)
        _add_box(self, "wall_front", (0.0, 2.5, 10.0), (24.0, 5.0, 0.5), WALL)
        _add_box(self, "wall_left", (-12.0, 2.5, 0.0), (0.5, 5.0, 20.0), WALL)
        _add_box(self, "wall_right", (12.0, 2.5, 0.0), (0.5, 5.0, 20.0), WALL)

        # Cyan wall lights, ceiling beams, and waist-high cover give the room
        # readable scale without needing external model or texture files.
        for index, x in enumerate((-8.0, -4.0, 0.0, 4.0, 8.0)):
            _add_box(self, f"back_panel_{index}", (x, 3.1, -9.68), (2.8, 0.10, 0.035), CYAN, solid=False)
            _add_box(self, f"ceiling_beam_{index}", (x, 4.9, 0.0), (0.10, 0.12, 18.0), PANEL, solid=False)

        props = [
            ("cover_a", (-3.4, 0.62, -2.0), (2.2, 1.25, 1.0), PANEL),
            ("cover_b", (3.5, 0.75, -3.0), (1.5, 1.5, 1.5), (0.30, 0.37, 0.42, 1.0)),
            ("crate_a", (-7.5, 0.70, 2.0), (1.4, 1.4, 1.4), AMBER),
            ("crate_b", (7.5, 0.70, 1.0), (1.4, 1.4, 1.4), BOT_RED),
            ("low_barrier", (0.0, 0.45, 0.5), (4.0, 0.9, 0.55), PANEL),
        ]
        for name, position, scale, color in props:
            self.add(ArenaProp(name, position, scale, color))

        for index, position in enumerate(STARTING_BOTS):
            Robot(f"enemy_{index}", position).stage_into(self)


class NeonBreach(at.Game):
    """Playable first-person game, with lifecycle and gameplay in OOP hooks."""

    title = "NEON BREACH — AUTOMATA FPS"
    resolution = (1280, 720)
    headless = "--headless" in sys.argv
    target_fps = 60
    clear_color = (0.018, 0.035, 0.075, 1.0)
    graphics = at.GraphicsQuality.HIGH

    def on_start(self):
        level = Arena()
        self.yaw = 0.0
        self.pitch = 0.0
        self.vertical_speed = 0.0
        self.health = 5
        self.max_health = 5
        self.ammo = 12
        self.magazine_size = 12
        self.reload_timer = 0.0
        self.fire_timer = 0.0
        self.recoil = 0.0
        self.muzzle_timer = 0.0
        self.hit_flash = 0.0
        self.pending_shots = []
        self.enemy_records = {
            f"enemy_{index}": {"health": 2, "attack": 0.9 + index * 0.2}
            for index in range(len(STARTING_BOTS))
        }
        self.wave = 1
        self.wave_pause = 0.0
        self.score = 0
        self.last_cursor = None
        self.ambient_sound = None
        self._audio_started = False

        # Build a first-person rifle from simple 3D blocks.
        self.rifle_parts = [
            RiflePiece("rifle_receiver", (0.24, 0.16, 0.46), RIFLE),
            RiflePiece("rifle_barrel", (0.085, 0.075, 0.55), (0.16, 0.19, 0.23, 1.0)),
            RiflePiece("rifle_rail", (0.10, 0.035, 0.42), AMBER),
            RiflePiece("rifle_grip", (0.14, 0.25, 0.18), (0.12, 0.15, 0.19, 1.0)),
            RiflePiece("rifle_sight", (0.065, 0.13, 0.07), CYAN),
            RiflePiece("muzzle_flash", (0.001, 0.001, 0.001), AMBER),
        ]

        # A simple world-space HUD: crosshair and little health/ammo lamps.
        self.hud_parts = [
            RiflePiece("crosshair_vertical", (0.018, 0.12, 0.012), CYAN),
            RiflePiece("crosshair_horizontal", (0.12, 0.018, 0.012), CYAN),
        ]
        self.health_lamps = [RiflePiece(f"health_lamp_{i}", (0.09, 0.045, 0.018), CYAN) for i in range(self.max_health)]
        self.ammo_lamps = [RiflePiece(f"ammo_lamp_{i}", (0.045, 0.10, 0.018), AMBER) for i in range(self.magazine_size)]
        self._was_reloading = False

        for piece in self.rifle_parts + self.hud_parts + self.health_lamps + self.ammo_lamps:
            level.add(piece)
        self.load_scene(level)

        print("NEON BREACH | WASD move · mouse/arrows look · click/F fire · R reload · Space jump")
        print("Clear the robot wave. Use cover, aim for the red bots, and watch the ammo lamps.")

    def _start_audio(self, ctx):
        if self._audio_started:
            return
        self._audio_started = True
        try:
            self.ambient_sound = ctx.audio.play(str(SOUNDS / "arena_hum.wav"), volume=0.16, loop_=True)
        except Exception as exc:  # a missing asset must not make the game unplayable
            print(f"audio disabled: {exc}")

    def _camera_basis(self):
        cp = math.cos(self.pitch)
        forward = (math.sin(self.yaw) * cp, math.sin(self.pitch), -math.cos(self.yaw) * cp)
        right = (math.cos(self.yaw), 0.0, math.sin(self.yaw))
        up = (
            -math.sin(self.yaw) * math.sin(self.pitch),
            math.cos(self.pitch),
            math.cos(self.yaw) * math.sin(self.pitch),
        )
        return forward, right, up

    @staticmethod
    def _offset(origin, forward, right, up, f=0.0, r=0.0, u=0.0):
        return (
            origin[0] + forward[0] * f + right[0] * r + up[0] * u,
            origin[1] + forward[1] * f + right[1] * r + up[1] * u,
            origin[2] + forward[2] * f + right[2] * r + up[2] * u,
        )

    @staticmethod
    def _set_pose(view, position, rotation, scale=None):
        if view is None or view.transform is None:
            return
        view.transform.position = position
        view.transform.rotation_euler = rotation
        if scale is not None:
            view.transform.scale = scale

    def _move_camera_and_weapon(self, ctx, camera_position, forward, right, up):
        rotation = (self.pitch, self.yaw, 0.0)
        camera = ctx.scene.find("camera")
        if camera is not None:
            camera.camera_3d = at.Camera3D(
                position=camera_position,
                target=(camera_position[0] + forward[0], camera_position[1] + forward[1], camera_position[2] + forward[2]),
                fov=76.0,
            )

        self.recoil = max(0.0, self.recoil - ctx.time.delta * 1.8)
        self.muzzle_timer = max(0.0, self.muzzle_timer - ctx.time.delta)
        gun_back = self.recoil
        local_poses = {
            "rifle_receiver": (0.88 - gun_back, 0.34, -0.30),
            "rifle_barrel": (1.28 - gun_back, 0.34, -0.245),
            "rifle_rail": (0.93 - gun_back, 0.34, -0.20),
            "rifle_grip": (0.70 - gun_back, 0.34, -0.49),
            "rifle_sight": (0.98 - gun_back, 0.34, -0.10),
            "muzzle_flash": (1.62 - gun_back, 0.34, -0.245),
        }
        for piece in self.rifle_parts:
            f, r, u = local_poses[piece.name]
            scale = piece.local_scale
            if piece.name == "muzzle_flash":
                scale = (0.21, 0.21, 0.28) if self.muzzle_timer > 0.0 else (0.001, 0.001, 0.001)
            self._set_pose(ctx.scene.find(piece.name), self._offset(camera_position, forward, right, up, f, r, u), rotation, scale)

        self.hit_flash = max(0.0, self.hit_flash - ctx.time.delta)
        for piece in self.hud_parts:
            view = ctx.scene.find(piece.name)
            self._set_pose(view, self._offset(camera_position, forward, right, up, 2.0), rotation)
            # A short bright flash expands the reticle when a shot connects.
            if view is not None and view.transform is not None and piece.name == "crosshair_vertical":
                stretch = 1.35 if self.hit_flash > 0 else 1.0
                view.transform.scale = (0.018, 0.12 * stretch, 0.012)
        for index, piece in enumerate(self.health_lamps):
            pos = self._offset(camera_position, forward, right, up, 1.9, -0.62 + index * 0.12, -0.48)
            size = (0.09, 0.045, 0.018) if index < self.health else (0.001, 0.001, 0.001)
            self._set_pose(ctx.scene.find(piece.name), pos, rotation, size)
        for index, piece in enumerate(self.ammo_lamps):
            pos = self._offset(camera_position, forward, right, up, 1.9, 0.50 + index * 0.075, -0.48)
            size = (0.045, 0.10, 0.018) if index < self.ammo else (0.001, 0.001, 0.001)
            self._set_pose(ctx.scene.find(piece.name), pos, rotation, size)

    def _update_enemy_models(self, ctx, player_position, dt):
        px, py, pz = player_position
        id_to_enemy = {}
        for name, enemy in list(self.enemy_records.items()):
            view = ctx.scene.find(name)
            if view is None or view.transform is None:
                continue
            id_to_enemy[view.id] = name
            ex, ey, ez = view.transform.position
            dx, dz = px - ex, pz - ez
            distance = math.hypot(dx, dz)
            yaw = math.atan2(dx, -dz) if distance > 0.001 else 0.0

            # Move the bots toward the player, but give the player room to aim.
            if distance > 1.65:
                step = min(distance - 1.5, 1.45 * dt)
                ex += dx / max(distance, 0.001) * step
                ez += dz / max(distance, 0.001) * step
                view.transform.position = (ex, ey, ez)
                view.transform.rotation_euler = (0.0, yaw, 0.0)

            enemy["attack"] = max(0.0, enemy["attack"] - dt)
            if distance < 1.75 and enemy["attack"] <= 0.0:
                enemy["attack"] = 1.1
                self.health = max(0, self.health - 1)
                self.hit_flash = 0.25
                ctx.events.send("player_hit", {"health": self.health})
                try:
                    ctx.audio.play(str(SOUNDS / "player_hit.wav"), volume=0.45)
                except Exception:
                    pass
                if self.health <= 0:
                    self.health = self.max_health
                    self.vertical_speed = 0.0
                    view_player = ctx.scene.find("player")
                    if view_player is not None and view_player.transform is not None:
                        view_player.transform.position = (0.0, 0.98, 7.2)
                    print("REBOOT: suit restored; keep moving!")

            # Follow the torso with the robot's head, visor and shoulder plate.
            for suffix, offset in (("head", 0.75), ("visor", 0.77), ("shoulders", 0.24)):
                part = ctx.scene.find(f"{name}_{suffix}")
                if part is None or part.transform is None:
                    continue
                if suffix == "visor":
                    front = (math.sin(yaw) * 0.255, 0.0, -math.cos(yaw) * 0.255)
                else:
                    front = (0.0, 0.0, 0.0)
                part.transform.position = (ex + front[0], ey + offset, ez + front[2])
                part.transform.rotation_euler = (0.0, yaw, 0.0)
        return id_to_enemy

    def _resolve_shots(self, ctx, id_to_enemy):
        for key in self.pending_shots:
            result = ctx.physics.ray_result(key)
            if result is None:
                continue
            entity_id = result[0]
            name = id_to_enemy.get(entity_id)
            if name is None or name not in self.enemy_records:
                continue
            self.hit_flash = 0.18
            enemy = self.enemy_records[name]
            enemy["health"] -= 1
            try:
                ctx.audio.play(str(SOUNDS / "bot_hit.wav"), volume=0.5)
            except Exception:
                pass
            if enemy["health"] <= 0:
                target = ctx.scene.find(name)
                if target is not None:
                    target.despawn()
                for suffix in ("head", "visor", "shoulders"):
                    part = ctx.scene.find(f"{name}_{suffix}")
                    if part is not None:
                        part.despawn()
                del self.enemy_records[name]
                self.score += 100
                print(f"BOT DOWN  +100  | score {self.score}")
                ctx.events.send("enemy_down", {"score": self.score, "wave": self.wave})
                try:
                    ctx.audio.play(str(SOUNDS / "bot_down.wav"), volume=0.75)
                except Exception:
                    pass
        self.pending_shots.clear()

    def _start_wave(self, ctx):
        self.wave += 1
        count = min(3 + self.wave - 1, 6)
        positions = [(-7.0, 1.0, -6.0), (0.0, 1.0, -7.0), (7.0, 1.0, -6.0), (-7.0, 1.0, -1.0), (7.0, 1.0, -1.0), (0.0, 1.0, -3.0)]
        for index in range(count):
            name = f"enemy_{self.wave}_{index}"
            robot = Robot(name, positions[index])
            robot.spawn_into(ctx.scene)
            self.enemy_records[name] = {"health": 2, "attack": 0.9 + index * 0.18}
        print(f"WAVE {self.wave}  | {count} bots incoming")

    def on_update(self, ctx):
        self._start_audio(ctx)
        dt = min(ctx.time.delta, 0.05)
        player = ctx.scene.find("player")
        if player is None or player.transform is None:
            return
        px, py, pz = player.transform.position

        # Resolve the previous frame's hitscan queries before this frame's fire.
        id_to_enemy = self._update_enemy_models(ctx, (px, py, pz), dt)
        self._resolve_shots(ctx, id_to_enemy)

        # Mouse motion plus arrow-key look keeps the prototype playable even
        # on window systems that don't provide relative mouse input.
        cursor = ctx.input.cursor
        if cursor is not None:
            if self.last_cursor is not None:
                self.yaw -= (cursor[0] - self.last_cursor[0]) * 0.0035
                self.pitch -= (cursor[1] - self.last_cursor[1]) * 0.0035
            self.last_cursor = cursor
        else:
            self.last_cursor = None
        look_x = float(ctx.input.pressed("arrow_right")) - float(ctx.input.pressed("arrow_left"))
        look_y = float(ctx.input.pressed("arrow_down")) - float(ctx.input.pressed("arrow_up"))
        self.yaw += look_x * 1.8 * dt
        self.pitch += look_y * 1.4 * dt
        self.pitch = max(-1.15, min(1.15, self.pitch))

        # Character controller movement in camera-relative ground plane.
        forward_input = float(ctx.input.pressed("w")) - float(ctx.input.pressed("s"))
        strafe_input = float(ctx.input.pressed("d")) - float(ctx.input.pressed("a"))
        magnitude = math.hypot(forward_input, strafe_input)
        if magnitude > 1.0:
            forward_input /= magnitude
            strafe_input /= magnitude
        forward = (math.sin(self.yaw), 0.0, -math.cos(self.yaw))
        right = (math.cos(self.yaw), 0.0, math.sin(self.yaw))
        vx = (forward[0] * forward_input + right[0] * strafe_input) * PLAYER_SPEED
        vz = (forward[2] * forward_input + right[2] * strafe_input) * PLAYER_SPEED
        if ctx.physics.grounded(player):
            self.vertical_speed = -0.8
            if ctx.input.just_pressed("space"):
                self.vertical_speed = JUMP_SPEED
        else:
            self.vertical_speed -= GRAVITY * dt
        player.character_velocity = (vx, self.vertical_speed, vz)

        # Camera sits at eye-height; the visual capsule stays out of frame.
        camera_position = (px, py + 0.54, pz)
        forward, right, up = self._camera_basis()

        # Process reload and fire timers. Holding the trigger gives a simple
        # semi-automatic rifle with a short per-shot cadence.
        self.fire_timer = max(0.0, self.fire_timer - dt)
        had_reload_timer = self.reload_timer > 0.0
        self.reload_timer = max(0.0, self.reload_timer - dt)
        if had_reload_timer and self.reload_timer == 0.0:
            self.ammo = self.magazine_size
            self._was_reloading = False
        if ctx.input.just_pressed("r") and self.ammo < self.magazine_size and self.reload_timer <= 0.0:
            self.reload_timer = 1.05
            self._was_reloading = True
            try:
                ctx.audio.play(str(SOUNDS / "reload.wav"), volume=0.5)
            except Exception:
                pass

        trigger = ctx.input.mouse_pressed("left") or ctx.input.pressed("f")
        if trigger and self.fire_timer <= 0.0 and self.reload_timer <= 0.0:
            if self.ammo <= 0:
                self.reload_timer = 1.05
                self._was_reloading = True
                try:
                    ctx.audio.play(str(SOUNDS / "reload.wav"), volume=0.5)
                except Exception:
                    pass
            else:
                direction = self._camera_basis()[0]
                key = ctx.physics.cast_ray(camera_position, direction, max_distance=38.0, exclude=player)
                self.pending_shots.append(key)
                self.ammo -= 1
                self.fire_timer = 0.18
                self.recoil = 0.13
                self.muzzle_timer = 0.07
                try:
                    ctx.audio.play(str(SOUNDS / "rifle_shot.wav"), volume=0.62, playback_rate=0.94 + (self.ammo % 4) * 0.035)
                except Exception as exc:
                    if not getattr(self, "_audio_error_shown", False):
                        print(f"audio disabled: {exc}")
                        self._audio_error_shown = True

        # Update the rifle and HUD after recoil / ammo have changed.
        forward, right, up = self._camera_basis()
        self._move_camera_and_weapon(ctx, camera_position, forward, right, up)

        if not self.enemy_records:
            self.wave_pause += dt
            if self.wave_pause >= 2.0:
                self.wave_pause = 0.0
                self._start_wave(ctx)
        else:
            self.wave_pause = 0.0

        if ctx.input.just_pressed("escape"):
            ctx.quit()

    def on_player_hit(self, payload):
        print(f"SUIT HIT  | integrity {payload['health']}/{self.max_health}")

    def on_enemy_down(self, payload):
        if not self.enemy_records:
            print(f"WAVE {payload['wave']} CLEARED")


if __name__ == "__main__":
    game = NeonBreach()
    frames = game.run(max_frames=180 if "--headless" in sys.argv else None)
    print(f"NEON BREACH ended after {frames} frames | score {game.score}")
