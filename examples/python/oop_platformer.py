#!/usr/bin/env python3
"""OOP AUTOMATA: a platformer as classes — no decorators, no globals.

- `Game` subclass: engine options are class attributes; `on_start`,
  `on_update`, and `on_<event>` methods are the lifecycle hooks.
- `Entity`/`Scene` subclasses bundle reusable archetypes and levels.

Controls: A/D or Left/Right arrows to run, Space to jump
(gamepad: left stick to run, south button to jump). Collect all coins.

    python examples/python/oop_platformer.py              # opens a window
    python examples/python/oop_platformer.py --headless   # no window (CI)
"""

import sys

import automata as at

WALK_SPEED = 5.0
GRAVITY = 20.0
JUMP_SPEED = 9.5

SKY = (0.55, 0.78, 0.95, 1.0)
GRASS = (0.35, 0.65, 0.30, 1.0)
WOOD = (0.72, 0.52, 0.30, 1.0)
GOLD = (1.0, 0.85, 0.25, 1.0)
ORANGE = (0.95, 0.45, 0.15, 1.0)
STEEL = (0.45, 0.47, 0.52, 1.0)

# Floating platforms: (x, y) centers; size is PLATFORM_SIZE.
PLATFORMS = [(1.5, 1.6), (4.5, 3.0), (7.5, 4.4), (10.5, 3.0)]
PLATFORM_SIZE = (2.4, 0.35, 2.0)

# Coins hover above each platform and along the ground.
COINS = [(x, y + 0.9) for (x, y) in PLATFORMS] + [(-2.0, 1.1), (13.5, 1.1)]


class Sun(at.Entity):
    """Warm afternoon sunlight with shadows."""

    def __init__(self):
        super().__init__("sun")
        self.add(
            at.DirectionalLight(
                direction=(-0.45, -1.0, -0.3),
                color=(1.0, 0.96, 0.88),
                ambient=0.35,
                shadows=True,
            )
        )


class Camera(at.Entity):
    def __init__(self):
        super().__init__("camera")
        self.add(at.Camera3D(position=(-4.0, 3.0, 10.0), target=(-4.0, 1.5, 0.0)))


class Ground(at.Entity):
    def __init__(self):
        super().__init__("ground")
        # Transform scale stretches the unit cube to match the collider.
        self.add(at.Transform(position=(3.0, 0.0, 0.0), scale=(26.0, 1.0, 6.0)))
        self.add(at.MeshRenderer.cube(size=1.0, color=GRASS))
        self.add(at.RigidBody.fixed())
        self.add(at.Collider3D.cuboid(26.0, 1.0, 6.0))


class Platform(at.Entity):
    """A floating wooden platform the player can jump onto."""

    def __init__(self, x, y):
        super().__init__()
        self.add(at.Transform(position=(x, y, 0.0), scale=PLATFORM_SIZE))
        self.add(at.MeshRenderer.cube(size=1.0, color=WOOD))
        self.add(at.RigidBody.fixed())
        self.add(at.Collider3D.cuboid(*PLATFORM_SIZE))


class Coin(at.Entity):
    """A spinning collectible; visual only, picked up by proximity."""

    def __init__(self, name, x, y):
        super().__init__(name)
        self.add(at.Transform(position=(x, y, 0.0), scale=(0.4, 0.4, 0.4)))
        self.add(at.MeshRenderer.cube(size=1.0, color=GOLD))


class Crate(at.Entity):
    """A dynamic prop the player can bump into."""

    def __init__(self, position):
        super().__init__()  # unnamed; scene access by name is optional
        self.add(at.Transform(position=position))
        self.add(at.MeshRenderer.cube(size=0.8, color=STEEL))
        self.add(at.RigidBody.dynamic())
        self.add(at.Collider3D.cuboid(0.8, 0.8, 0.8))


class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(-4.0, 1.4, 0.0)))
        self.add(at.MeshRenderer.cube(size=0.9, color=ORANGE))
        self.add(at.RigidBody.kinematic_velocity())
        self.add(at.Collider3D.capsule(0.5, 0.4))
        self.add(at.CharacterController(snap_to_ground=0.5))
        self.add(at.CharacterVelocity())


class Level(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(Camera())
        self.add(Sun())
        self.add(Ground())
        self.add(Player())
        self.add(Crate(position=(6.0, 1.0, 0.0)))
        for x, y in PLATFORMS:
            self.add(Platform(x=x, y=y))
        for i, (x, y) in enumerate(COINS):
            self.add(Coin(f"coin_{i}", x=x, y=y))


class Platformer(at.Game):
    title = "oop platformer"
    headless = "--headless" in sys.argv  # window by default; flag for CI
    target_fps = 60
    clear_color = SKY

    def on_start(self):
        self.load_scene(Level())
        self.vy = 0.0
        self.score = 0
        self.coins = {f"coin_{i}": (x, y) for i, (x, y) in enumerate(COINS)}
        self.cam_x, self.cam_y = -4.0, 3.0
        self.controls = at.ActionMap()
        self.controls.bind("jump", at.Binding.key("space"))
        self.controls.bind("jump", at.Binding.gamepad_button("south"))
        # Multiple bindings stack: WASD, arrows, and gamepad all drive move_x.
        self.controls.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
        self.controls.bind_axis("move_x", at.AxisBinding.key_pair("arrow_left", "arrow_right"))
        self.controls.bind_axis("move_x", at.AxisBinding.gamepad_axis("left_stick_x"))

    def on_update(self, ctx):
        player = ctx.scene["player"]
        px, py, _ = player.transform.position
        dx = self.controls.axis("move_x", ctx.input)

        if ctx.physics.grounded(player):
            self.vy = -1.0  # hold contact so snap-to-ground keeps working
            if self.controls.just_pressed("jump", ctx.input):
                self.vy = JUMP_SPEED
        else:
            self.vy -= GRAVITY * ctx.time.delta

        player.character_velocity = (dx * WALK_SPEED, self.vy, 0.0)

        # Smooth follow camera: ease toward the player, look slightly ahead.
        blend = min(1.0, 8.0 * ctx.time.delta)
        self.cam_x += (px - self.cam_x) * blend
        self.cam_y += (max(py + 1.6, 2.6) - self.cam_y) * blend
        ctx.scene["camera"].camera_3d = at.Camera3D(
            position=(self.cam_x, self.cam_y, 10.0),
            target=(self.cam_x, self.cam_y - 1.4, 0.0),
        )

        # Spin coins and pick them up on contact.
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

        if ctx.input.just_pressed("escape"):
            ctx.quit()

    def on_coin_pickup(self, payload):
        """Auto-registered handler for events named "coin_pickup"."""
        print(f"coin collected: {payload['score']}/{payload['total']}")
        if payload["score"] == payload["total"]:
            print("all coins collected — you win!")


if __name__ == "__main__":
    frames = Platformer().run(max_frames=300 if "--headless" in sys.argv else None)
    print(f"finished after {frames} frames")
