#!/usr/bin/env python3
"""OOP AUTOMATA: a platformer as classes — no decorators, no globals.

- `Game` subclass: engine options are class attributes; `on_start`,
  `on_update`, and `on_<event>` methods are the lifecycle hooks.
- `Entity`/`Scene` subclasses bundle reusable archetypes and levels.

    python examples/python/oop_platformer.py              # opens a window
    python examples/python/oop_platformer.py --headless   # no window (CI)
"""

import sys

import automata as at

WALK_SPEED = 3.0
GRAVITY = 12.0
JUMP_SPEED = 6.0

ORANGE = (0.9, 0.6, 0.2, 1.0)
STEEL = (0.35, 0.37, 0.42, 1.0)
MOSS = (0.4, 0.7, 0.35, 1.0)


class Camera(at.Entity):
    def __init__(self):
        super().__init__("camera")
        self.add(at.Camera3D(position=(0.0, 3.5, 9.0), target=(0.0, 1.0, 0.0)))


class Floor(at.Entity):
    def __init__(self):
        super().__init__("floor")
        # Transform scale stretches the unit cube to match the collider.
        self.add(at.Transform(position=(0.0, 0.0, 0.0), scale=(40.0, 1.0, 40.0)))
        self.add(at.MeshRenderer.cube(size=1.0, color=STEEL))
        self.add(at.RigidBody.fixed())
        self.add(at.Collider3D.cuboid(40.0, 1.0, 40.0))


class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(-3.0, 1.4, 0.0)))
        self.add(at.MeshRenderer.cube(size=0.9, color=ORANGE))
        self.add(at.RigidBody.kinematic_velocity())
        self.add(at.Collider3D.capsule(0.5, 0.4))
        self.add(at.CharacterController(snap_to_ground=0.5))
        self.add(at.CharacterVelocity())


class Crate(at.Entity):
    """A dynamic prop the player can bump into."""

    def __init__(self, position):
        super().__init__()  # unnamed; scene access by name is optional
        self.add(at.Transform(position=position))
        self.add(at.MeshRenderer.cube(size=0.8, color=MOSS))
        self.add(at.RigidBody.dynamic())
        self.add(at.Collider3D.cuboid(0.8, 0.8, 0.8))


class Level(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(Camera())
        self.add(Floor())
        self.add(Player())
        self.add(Crate(position=(2.0, 1.0, 0.0)))


class Platformer(at.Game):
    title = "oop platformer"
    headless = "--headless" in sys.argv  # window by default; flag for CI
    target_fps = 60

    def on_start(self):
        self.load_scene(Level())
        self.vy = 0.0
        self.controls = at.ActionMap()
        self.controls.bind("jump", at.Binding.key("space"))
        self.controls.bind("jump", at.Binding.gamepad_button("south"))
        self.controls.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
        self.controls.bind_axis("move_x", at.AxisBinding.gamepad_axis("left_stick_x"))

    def on_update(self, ctx):
        player = ctx.scene["player"]
        dx = self.controls.axis("move_x", ctx.input)

        if ctx.physics.grounded(player):
            self.vy = -1.0  # hold contact so snap-to-ground keeps working
            if self.controls.just_pressed("jump", ctx.input):
                self.vy = JUMP_SPEED
        else:
            self.vy -= GRAVITY * ctx.time.delta

        player.character_velocity = (dx * WALK_SPEED, self.vy, 0.0)

        if ctx.input.just_pressed("escape"):
            ctx.quit()

    def on_crate_hit(self, payload):
        """Auto-registered handler for events named "crate_hit"."""
        print("crate hit!", payload)


if __name__ == "__main__":
    frames = Platformer().run(max_frames=300 if "--headless" in sys.argv else None)
    print(f"finished after {frames} frames")
