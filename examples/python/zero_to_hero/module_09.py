import automata as at

SKY = (0.55, 0.78, 0.95, 1.0)
WALK, GRAVITY, JUMP = 5.0, 20.0, 9.5


class Camera(at.Entity):
    def __init__(self):
        super().__init__("camera")
        self.add(at.Camera3D(position=(0, 3.5, 10), target=(0, 1, 0)))


class Sun(at.Entity):
    def __init__(self):
        super().__init__("sun")
        self.add(at.DirectionalLight(direction=(-0.45, -1.0, -0.3), ambient=0.35, shadows=True))


class Ground(at.Entity):
    def __init__(self):
        super().__init__("ground")
        self.add(at.Transform(scale=(30, 1, 8)))
        self.add(at.MeshRenderer.cube(color=(0.35, 0.65, 0.30, 1.0)))
        self.add(at.RigidBody.fixed())
        self.add(at.Collider3D.cuboid(30, 1, 8))


class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(-4, 1.4, 0)))
        self.add(at.MeshRenderer.cube(size=0.9, color=(0.95, 0.45, 0.15, 1.0)))
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


class Platformer(at.Game):
    title = "platformer"
    target_fps = 60
    clear_color = SKY
    graphics = at.GraphicsQuality.HIGH

    def on_start(self):
        self.load_scene(Level())
        self.vy = 0.0
        self.controls = at.ActionMap()
        self.controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
        self.controls.bind("jump", at.Binding.key(at.Key.SPACE))

    def on_update(self, ctx):
        player = ctx.scene["player"]
        if ctx.physics.grounded(player):
            self.vy = -1.0
            if self.controls.just_pressed("jump", ctx.input):
                self.vy = JUMP
        else:
            self.vy -= GRAVITY * ctx.time.delta
        dx = self.controls.axis("move_x", ctx.input)
        player.character_velocity = (dx * WALK, self.vy, 0.0)


if __name__ == "__main__":
    Platformer().run()
