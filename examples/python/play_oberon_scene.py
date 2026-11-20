#!/usr/bin/env python3
"""Play a scene made in Oberon from Python.

Oberon saves scenes as RON files (Ctrl+S → `scene.ron`). This script loads
one into the engine and plays it the same way Oberon's Play button does:

- every entity with a **Character Controller** (+ Create → Player) is the
  player: WASD / arrows move (relative to the camera), Space jumps, Shift
  sprints;
- Q / E turn the camera, which follows the player from behind;
- Rigidbody + Collider objects are simulated by the engine's physics;
- Escape quits.

    python examples/python/play_oberon_scene.py scene.ron
    python examples/python/play_oberon_scene.py scene.ron --headless   # CI

The core of it is three lines — load the file, hand it to a Game, run:

    scene = at.Scene.load("scene.ron")
    game.load_scene(scene)
    game.run(update)
"""

import math
import sys

import automata as at

WALK_SPEED = 5.0
JUMP_SPEED = 7.0
GRAVITY = 20.0
CAMERA_DISTANCE = 9.0
CAMERA_HEIGHT = 3.5
TURN_SPEED = 2.0  # radians per second (Q / E)

args = [a for a in sys.argv[1:] if not a.startswith("--")]
HEADLESS = "--headless" in sys.argv
if not args:
    sys.exit(__doc__)
SCENE_PATH = args[0]


def held(ctx, *keys):
    return any(ctx.input.pressed(k) for k in keys)


def axis(ctx, positive, negative):
    return float(held(ctx, *positive)) - float(held(ctx, *negative))


class OberonScene(at.Game):
    title = f"{SCENE_PATH} — AUTOMATA"
    headless = HEADLESS
    target_fps = 60
    clear_color = (0.05, 0.055, 0.07, 1.0)
    graphics = at.GraphicsQuality.HIGH

    def on_start(self):
        scene = at.Scene.load(SCENE_PATH)
        self.load_scene(scene)
        print(f"loaded {SCENE_PATH}: {scene.entity_count} entities")
        self.vy = {}  # player entity id -> vertical speed
        self.yaw = 0.6  # camera yaw, same convention as Oberon's view
        self.focus = None
        self.camera_spawned = False

    def on_update(self, ctx):
        if ctx.input.just_pressed(at.Key.ESCAPE):
            ctx.quit()
            return
        dt = ctx.time.delta
        self.yaw += axis(ctx, [at.Key.Q], [at.Key.E]) * TURN_SPEED * dt

        players = [e for e in ctx.scene if e.character_velocity is not None]
        cameras = [e for e in ctx.scene if e.camera_3d is not None]

        # Movement relative to the camera, on the ground plane.
        forward = axis(ctx, [at.Key.W, at.Key.ARROW_UP], [at.Key.S, at.Key.ARROW_DOWN])
        right = axis(ctx, [at.Key.D, at.Key.ARROW_RIGHT], [at.Key.A, at.Key.ARROW_LEFT])
        if HEADLESS and ctx.time.elapsed < 1.0:
            forward = 1.0  # attract mode: no keyboard without a window
        sy, cy = math.sin(self.yaw), math.cos(self.yaw)
        wx = -sy * forward + cy * right
        wz = -cy * forward - sy * right
        length = math.hypot(wx, wz)
        if length > 1.0:
            wx, wz = wx / length, wz / length
        speed = WALK_SPEED * (2.0 if held(ctx, at.Key.SHIFT_LEFT, at.Key.SHIFT_RIGHT) else 1.0)

        for player in players:
            vy = self.vy.get(player.id, 0.0)
            if ctx.physics.grounded(player):
                vy = JUMP_SPEED if ctx.input.pressed(at.Key.SPACE) else -2.0
            else:
                vy = max(vy - GRAVITY * dt, -50.0)
            self.vy[player.id] = vy
            player.character_velocity = (wx * speed, vy, wz * speed)

        # Third-person follow camera on the first player (or the origin).
        if players and players[0].transform is not None:
            px, py, pz = players[0].transform.position
            goal = (px, py + 0.6, pz)
        else:
            goal = (0.0, 0.5, 0.0)
        if self.focus is None:
            self.focus = goal
        blend = min(1.0, 10.0 * dt)
        self.focus = tuple(f + (g - f) * blend for f, g in zip(self.focus, goal))
        fx, fy, fz = self.focus
        eye = (fx + CAMERA_DISTANCE * sy, fy + CAMERA_HEIGHT, fz + CAMERA_DISTANCE * cy)
        view = at.Camera3D(position=eye, target=self.focus)
        if cameras:
            for camera in cameras:
                camera.camera_3d = view
        elif not self.camera_spawned:
            # Oberon scenes often have no camera: add one to look through.
            ctx.scene.spawn(at.Entity("play camera").add(view))
            self.camera_spawned = True


if __name__ == "__main__":
    frames = OberonScene().run(max_frames=120 if HEADLESS else None)
    print(f"finished after {frames} frames")
