#!/usr/bin/env python3
"""The Cookbook recipes, pasted into the capstone platformer.

Every recipe from docs/wiki/cookbook.md working together in one game, built
on `oop_platformer.py`:

- movement: double jump, coyote time, jump buffering
- cameras: screen shake on a hard landing (a "hit" event)
- spawning: a wave of crates rains down every few seconds
- flow: Esc pauses, collecting every coin loads the next, busier level
- input: F1 rebinds jump to the next key you press

Controls: A/D or arrows to run, Space to jump (twice for a double jump),
Esc to pause, F1 to rebind jump, Q to quit.

    python examples/python/cookbook_platformer.py
    python examples/python/cookbook_platformer.py --headless   # CI
"""

import random
import sys

import automata as at
from oop_platformer import COINS, GRAVITY, JUMP_SPEED, WALK_SPEED, Crate, Level, Platformer

MAX_JUMPS = 2
COYOTE = 0.12  # seconds after leaving a ledge that a jump still works
BUFFER = 0.12  # seconds before landing that a jump press is remembered
HARD_LANDING = -12.0  # falling faster than this shakes the camera
SHAKE_TIME = 0.25
WAVE_EVERY = 5.0
MAX_WAVE = 4


def make_level(number):
    """Level 1 is the capstone level; each later level starts with more crates."""
    level = Level()
    for i in range(number - 1):
        level.add(Crate(position=(-1.0 + 2.5 * i, 1.0, 0.0)))
    return level


class CookbookPlatformer(Platformer):
    title = "cookbook platformer"

    def on_start(self):
        super().on_start()
        self.level = 1
        self.reset_level_state()
        self.paused = False
        self.listening_for = None
        self.controls.bind("quit", at.Binding.key(at.Key.Q))

    def reset_level_state(self):
        self.vy = 0.0
        self.score = 0
        self.coins = {f"coin_{i}": (x, y) for i, (x, y) in enumerate(COINS)}
        self.jumps_left = MAX_JUMPS
        self.coyote = 0.0
        self.jump_buffer = 0.0
        self.shake = 0.0
        self.next_wave = WAVE_EVERY
        self.wave = 0

    # -- Input: rebindable controls ----------------------------------------

    def listen_for_rebind(self, ctx):
        """F1 starts listening; the next key pressed becomes the jump key."""
        if self.listening_for is None:
            if ctx.input.just_pressed(at.Key.F1):
                self.listening_for = "jump"
                print("press a key for jump…")
            return False
        for key in at.Key:
            if key != at.Key.F1 and ctx.input.just_pressed(key):
                self.controls.rebind(self.listening_for, [at.Binding.key(key)])
                print(f"jump is now {key.value}")
                self.listening_for = None
                break
        return True  # swallow input while listening

    # -- Movement: double jump, coyote time, jump buffering ----------------

    def move(self, ctx, player):
        dt = ctx.time.delta
        grounded = ctx.physics.grounded(player) and self.vy <= 0.0

        if grounded:
            if self.vy < HARD_LANDING:
                ctx.events.send("hit", {"speed": -self.vy})
            self.vy = -1.0
            self.coyote = COYOTE
            self.jumps_left = MAX_JUMPS
        else:
            self.vy -= GRAVITY * dt
            self.coyote = max(0.0, self.coyote - dt)
            if self.coyote == 0.0:
                # Walked off a ledge without jumping: only the air jump is left.
                self.jumps_left = min(self.jumps_left, MAX_JUMPS - 1)

        pressed = self.controls.just_pressed("jump", ctx.input)
        self.jump_buffer = BUFFER if pressed else max(0.0, self.jump_buffer - dt)

        if self.jump_buffer > 0.0 and self.coyote > 0.0:
            self.jump(MAX_JUMPS - 1)  # from the ground (or just after leaving it)
        elif pressed and self.jumps_left > 0:
            self.jump(self.jumps_left - 1)  # in mid-air

        dx = self.controls.axis("move_x", ctx.input)
        player.character_velocity = (dx * WALK_SPEED, self.vy, 0.0)

    def jump(self, jumps_left):
        self.vy = JUMP_SPEED
        self.jumps_left = jumps_left
        self.jump_buffer = 0.0
        self.coyote = 0.0

    # -- Camera: smooth follow plus screen shake ----------------------------

    def follow_camera(self, ctx, px, py):
        dt = ctx.time.delta
        blend = min(1.0, 8.0 * dt)
        self.cam_x += (px - self.cam_x) * blend
        self.cam_y += (max(py + 1.6, 2.6) - self.cam_y) * blend

        self.shake = max(0.0, self.shake - dt)
        mag = 0.3 * (self.shake / SHAKE_TIME)
        ox, oy = random.uniform(-mag, mag), random.uniform(-mag, mag)
        ctx.scene["camera"].camera_3d = at.Camera3D(
            position=(self.cam_x + ox, self.cam_y + oy, 10.0),
            target=(self.cam_x, self.cam_y - 1.4, 0.0),
        )

    # -- Spawning: crate waves on a timer ---------------------------------

    def spawn_waves(self, ctx):
        self.next_wave -= ctx.time.delta
        if self.next_wave <= 0.0 and self.wave < MAX_WAVE:
            self.next_wave = WAVE_EVERY
            self.wave += 1
            for i in range(self.wave):
                ctx.scene.spawn(Crate(position=(2.0 + i * 1.5, 8.0, 0.0)))  # appears next frame

    # -- Pickup by proximity -----------------------------------------------

    def collect_coins(self, ctx, px, py):
        for name, (cx, cy) in list(self.coins.items()):
            coin = ctx.scene.find(name)
            if coin is None:
                continue
            coin.transform.rotation_euler = (0.0, ctx.time.elapsed * 2.5, 0.0)
            if abs(px - cx) < 0.8 and abs(py - cy) < 1.0:
                coin.despawn()
                del self.coins[name]
                self.score += 1
                ctx.events.send("coin_pickup", {"score": self.score, "total": len(COINS)})
                if not self.coins:
                    ctx.events.send("level_complete", {"level": self.level})

    # -- The frame ------------------------------------------------------------

    def on_update(self, ctx):
        if self.controls.just_pressed("quit", ctx.input):
            ctx.quit()
        if self.listen_for_rebind(ctx):
            return

        player = ctx.scene["player"]

        # Pause: Esc toggles; while paused nothing moves.
        if ctx.input.just_pressed(at.Key.ESCAPE):
            self.paused = not self.paused
            print("paused" if self.paused else "unpaused")
        if self.paused:
            player.character_velocity = (0.0, 0.0, 0.0)
            return

        px, py, _ = player.transform.position
        self.move(ctx, player)
        self.follow_camera(ctx, px, py)
        self.spawn_waves(ctx)
        self.collect_coins(ctx, px, py)

    # -- Events -------------------------------------------------------------

    def on_hit(self, payload):
        self.shake = SHAKE_TIME  # seconds of shake

    def on_coin_pickup(self, payload):
        print(f"coin collected: {payload['score']}/{payload['total']}")

    def on_level_complete(self, payload):
        self.level += 1
        print(f"level {self.level}!")
        self.load_scene(make_level(self.level))  # applies immediately
        self.reset_level_state()


if __name__ == "__main__":
    frames = CookbookPlatformer().run(max_frames=300 if "--headless" in sys.argv else None)
    print(f"finished after {frames} frames")
