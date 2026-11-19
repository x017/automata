#!/usr/bin/env python3
"""AUTOMATA physics playground: a controllable character among physics props.

Shows the Phase 6 surface working together:

- an ActionMap binding "move_x"/"jump" to keyboard AND gamepad (the same
  actions work with either device, remappable at runtime),
- a kinematic character controller driven per frame through
  `view.character_velocity`, with slide/slope/snap-to-ground resolution,
- dynamic bodies interacting (a ball drops onto the floor and settles),
- a raycast query each frame (result read one frame later),
- gamepad connection events.

Run after building the bindings:

    python examples/python/physics_playground.py              # opens a window
    python examples/python/physics_playground.py --headless   # no window (CI)

Headless mode has no devices, so the demo adds an "attract mode" autopilot
when no control is active — with a window, WASD/arrows/gamepad drive the
same code path.
"""

import sys

import automata as at

WALK_SPEED = 3.0
GRAVITY = 12.0
JUMP_SPEED = 6.0

HEADLESS = "--headless" in sys.argv

game = at.Game(
    title="physics playground",
    headless=HEADLESS,
    target_fps=60,
    clear_color=(0.55, 0.78, 0.95, 1.0),  # sky blue
)
game.headless_flag = HEADLESS  # consulted by the attract-mode check below

game.load_scene(
    at.Scene()
    .add(
        at.Entity("camera")
        .add(at.Camera3D(position=(0.0, 4.0, 10.0), target=(0.0, 1.0, 0.0)))
    )
    .add(
        at.Entity("sun").add(
            at.DirectionalLight(direction=(-0.45, -1.0, -0.3), ambient=0.35, shadows=True)
        )
    )
    .add(
        at.Entity("floor")
        .add(at.Transform(position=(0.0, 0.0, 0.0), scale=(40.0, 1.0, 40.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.35, 0.37, 0.42, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(40.0, 1.0, 40.0))
    )
    .add(
        at.Entity("ball")
        .add(at.Transform(position=(2.5, 6.0, 0.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.85, 0.3, 0.3, 1.0)))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.ball(0.5).restitution(0.3))
    )
    .add(
        at.Entity("player")
        .add(at.Transform(position=(-3.0, 1.4, 0.0)))
        .add(at.MeshRenderer.cube(size=0.9, color=(0.9, 0.6, 0.2, 1.0)))
        .add(at.RigidBody.kinematic_velocity())
        .add(at.Collider3D.capsule(0.5, 0.4))
        .add(at.CharacterController(snap_to_ground=0.5))
        .add(at.CharacterVelocity())
    )
)

# Genre-independent controls: keyboard keys, gamepad buttons and sticks all
# feed the same named actions/axes.
controls = at.ActionMap()
controls.bind("jump", at.Binding.key("space"))
controls.bind("jump", at.Binding.gamepad_button("south"))
controls.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
controls.bind_axis("move_x", at.AxisBinding.gamepad_axis("left_stick_x"))

state = {"vy": 0.0, "ray_key": None, "autopilot": 1.0}


@game.update
def tick(ctx):
    player = ctx.scene["player"]

    for event in ctx.input.gamepads.events:
        if event.kind == "connected":
            print(f"gamepad {event.id} connected: {event.name}")

    dx = controls.axis("move_x", ctx.input)
    jumping = controls.just_pressed("jump", ctx.input)
    if game.headless_flag and dx == 0.0 and not jumping:
        dx = state_autopilot(ctx)  # attract mode for headless runs

    grounded = ctx.physics.grounded(player)
    vy = state["vy"]
    if grounded:
        vy = -1.0  # keep contact so snap-to-ground holds on slopes
        if jumping:
            vy = JUMP_SPEED
    else:
        vy -= GRAVITY * ctx.time.delta
    state["vy"] = vy

    player.character_velocity = (dx * WALK_SPEED, vy, 0.0)

    # Rays are answered a frame later: read last frame's, cast a new one.
    if state["ray_key"] is not None:
        hit = ctx.physics.ray_result(state["ray_key"])
        if hit and ctx.time.frame % 30 == 0:
            _, point, _, distance = hit
            print(
                f"frame {ctx.time.frame}: ray down from player hits "
                f"({point[0]:.2f}, {point[1]:.2f}, {point[2]:.2f}) "
                f"at distance {distance:.2f}"
            )
    pos = player.transform.position
    state["ray_key"] = ctx.physics.cast_ray(pos, (0.0, -1.0, 0.0), 10.0, player)

    if ctx.time.frame % 60 == 0:
        ball = ctx.scene["ball"].transform.position
        print(
            f"frame {ctx.time.frame}: player=({pos[0]:.2f}, {pos[1]:.2f}) "
            f"grounded={grounded} ball=({ball[0]:.2f}, {ball[1]:.2f}) "
            f"bodies={ctx.physics.body_count()}"
        )


def state_autopilot(ctx):
    """Gentle back-and-forth walk for headless/demo runs."""
    if ctx.time.frame % 240 == 0:
        state["autopilot"] = -state["autopilot"]
    return state["autopilot"]


frames = game.run(max_frames=360 if HEADLESS else None)
print(f"playground finished after {frames} frames; state={game.state}")
