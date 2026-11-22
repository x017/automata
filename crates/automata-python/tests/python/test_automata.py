"""End-to-end tests for the AUTOMATA Python bindings (headless mode).

Run with:  pytest crates/automata-python/tests/python
"""

import pytest

import automata as at


def make_game():
    return at.Game(title="tests", headless=True)


def test_version_exposed():
    assert isinstance(at.__version__, str) and at.__version__


def test_headless_run_counts_frames():
    game = make_game()
    calls = []

    def update(ctx):
        calls.append(ctx.time.frame)

    frames = game.run(update, max_frames=30)
    assert frames == 30
    assert calls == list(range(1, 31))
    assert game.state == "shutdown"


def test_scene_load_find_and_transform_readback():
    game = make_game()
    scene = at.Scene()
    cube = at.Entity("cube")
    cube.add(at.Transform(position=(1.0, 2.0, 3.0)))
    scene.add(cube)
    game.load_scene(scene)

    seen = {}

    def update(ctx):
        cube_view = ctx.scene.find("cube")
        assert cube_view is not None
        seen["pos"] = cube_view.transform.position
        seen["count"] = ctx.scene.entity_count
        ctx.quit()

    game.run(update)
    assert seen["pos"] == (1.0, 2.0, 3.0)
    assert seen["count"] == 1


def test_transform_writes_apply_after_the_callback():
    game = make_game()
    scene = at.Scene()
    cube = at.Entity("cube")
    cube.add(at.Transform())
    scene.add(cube)
    game.load_scene(scene)

    positions = []

    def update(ctx):
        cube_view = ctx.scene.find("cube")
        positions.append(cube_view.transform.position)
        if ctx.time.frame == 1:
            # Written now, visible next frame.
            cube_view.transform = at.Transform(position=(5.0, 0.0, 0.0))

    game.run(update, max_frames=3)
    assert positions[0] == (0.0, 0.0, 0.0)
    assert positions[1] == (5.0, 0.0, 0.0)
    assert positions[2] == (5.0, 0.0, 0.0)


def test_spawn_and_despawn_commands():
    game = make_game()
    counts = []

    def update(ctx):
        counts.append(ctx.scene.entity_count)
        if ctx.time.frame == 1:
            bullet = at.Entity("bullet")
            bullet.add(at.Transform(position=(9.0, 0.0, 0.0)))
            ctx.scene.spawn(bullet)
        elif ctx.time.frame == 3:
            ctx.scene.find("bullet").despawn()

    game.run(update, max_frames=5)
    assert counts == [0, 1, 1, 0, 0]


def test_quit_stops_the_loop_early():
    game = make_game()

    def update(ctx):
        if ctx.time.frame == 7:
            ctx.quit()

    assert game.run(update, max_frames=100) == 7


def test_python_exception_propagates_and_stops_the_loop():
    game = make_game()
    frames = []

    def update(ctx):
        frames.append(ctx.time.frame)
        if ctx.time.frame == 3:
            raise ValueError("boom")

    with pytest.raises(ValueError, match="boom"):
        game.run(update)
    assert frames == [1, 2, 3]


def test_run_twice_raises_engine_error():
    game = make_game()
    game.run(lambda ctx: ctx.quit())
    with pytest.raises(at.Error):
        game.run(lambda ctx: None)


def test_input_snapshot_defaults_and_validation():
    game = make_game()
    checked = []

    def update(ctx):
        checked.append(ctx.input.pressed("w"))
        checked.append(ctx.input.just_pressed("escape"))
        checked.append(ctx.input.mouse_pressed("left"))
        checked.append(ctx.input.cursor is None)
        ctx.quit()

    game.run(update)
    assert checked == [False, False, False, True]

    def bad_key(ctx):
        ctx.input.pressed("not_a_key")

    game2 = make_game()
    with pytest.raises(KeyError):
        game2.run(bad_key)


def test_events_round_trip_next_frame_delivery():
    game = make_game()
    seen = []

    def update(ctx):
        seen.append(ctx.events.read("hit"))
        if ctx.time.frame == 1:
            ctx.events.send("hit", {"damage": 42})

    game.run(update, max_frames=4)
    assert seen[0] == []
    assert seen[1] == [{"damage": 42}]  # readable the frame after sending
    assert seen[2] == []
    assert seen[3] == []


def test_resources_persist_across_frames():
    game = make_game()
    scores = []

    def update(ctx):
        if ctx.time.frame == 1:
            assert "score" not in ctx.resources
            ctx.resources.set("score", 10)
        else:
            scores.append(ctx.resources.get("score"))

    game.run(update, max_frames=3)
    assert scores == [10, 10]


def test_repr_and_staging_inspection():
    entity = at.Entity("player")
    entity.add(at.Transform(position=(1.0, 0.0, 0.0)))
    entity.add(at.Camera2D())
    scene = at.Scene()
    scene.add(entity)
    assert len(scene) == 1
    assert scene.entity_count == 1
    assert "player" in repr(entity)

    with pytest.raises(TypeError):
        entity.add(object())


def test_builder_chaining():
    scene = (
        at.Scene()
        .add(at.Entity("a").add(at.Transform(position=(1.0, 0.0, 0.0))))
        .add(at.Entity("b").add(at.Camera2D()))
    )
    assert len(scene) == 2


def test_transform_properties_are_mutable():
    t = at.Transform()
    t.position = (1.0, 2.0, 3.0)
    t.scale = (2.0, 2.0, 2.0)
    assert t.position == (1.0, 2.0, 3.0)
    assert t.scale == (2.0, 2.0, 2.0)
    assert t.copy().position == (1.0, 2.0, 3.0)


def test_scene_getitem_and_iteration():
    game = make_game()
    scene = (
        at.Scene()
        .add(at.Entity("cube").add(at.Transform(position=(7.0, 0.0, 0.0))))
        .add(at.Entity("floor").add(at.Transform()))
    )
    game.load_scene(scene)

    result = {}

    def update(ctx):
        cube = ctx.scene["cube"]
        result["name"] = cube.name
        result["pos"] = cube.transform.position
        result["iterated"] = sorted(e.name for e in ctx.scene if e.name)
        with pytest.raises(KeyError):
            ctx.scene["nope"]
        ctx.quit()

    game.run(update)
    assert result["name"] == "cube"
    assert result["pos"] == (7.0, 0.0, 0.0)
    assert result["iterated"] == ["cube", "floor"]


def test_transform_attribute_writes_compose():
    game = make_game()
    scene = at.Scene().add(at.Entity("cube").add(at.Transform()))
    game.load_scene(scene)

    positions = []

    def update(ctx):
        cube = ctx.scene["cube"]
        positions.append(cube.transform.position)
        if ctx.time.frame == 1:
            cube.transform.position = (1.0, 0.0, 0.0)
            cube.transform.scale = (3.0, 3.0, 3.0)  # must not clobber position

    game.run(update, max_frames=3)
    assert positions[0] == (0.0, 0.0, 0.0)
    assert positions[1] == (1.0, 0.0, 0.0)

    scales = []

    def update2(ctx):
        scales.append(ctx.scene["cube"].transform.scale)

    # New game to observe the world from scratch; the same buffered-write
    # semantics apply, so verify with a fresh run instead.
    game2 = make_game()
    scene2 = at.Scene().add(at.Entity("cube").add(at.Transform(scale=(3.0, 3.0, 3.0))))
    game2.load_scene(scene2)
    game2.run(update2, max_frames=1)
    assert scales == [(3.0, 3.0, 3.0)]


def test_decorator_style_and_event_handlers():
    game = make_game()

    scene = at.Scene().add(at.Entity("ball").add(at.Transform()))
    game.load_scene(scene)

    hits = []

    @game.update
    def tick(ctx):
        if ctx.time.frame == 1:
            ctx.events.send("hit", 5)

    @game.on_event("hit")
    def on_hit(payload):
        hits.append(payload)

    frames = game.run(max_frames=3)  # no positional callback
    assert frames == 3
    assert hits == [5]


def test_run_without_any_callback_raises():
    game = make_game()
    with pytest.raises(TypeError, match="no update callback"):
        game.run()


# -- Phase 6: physics -------------------------------------------------------


def test_physics_components_exist():
    body = at.RigidBody.dynamic()
    assert body.body_type == "dynamic"
    collider = at.Collider3D.ball(0.5).density(2.0).sensor(True)
    assert "Ball" in repr(collider)
    c2d = at.Collider2D.cuboid(1.0, 2.0)
    assert "Cuboid" in repr(c2d)
    controller = at.CharacterController(snap_to_ground=0.4)
    assert "Normal" in repr(controller)
    vel = at.CharacterVelocity((1.0, 2.0, 3.0))
    assert vel.velocity == (1.0, 2.0, 3.0)


def test_scene_spawns_physics_components():
    scene = at.Scene().add(
        at.Entity("ball")
        .add(at.Transform(position=(0.0, 5.0, 0.0)))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.ball(0.5))
    )
    assert len(scene) == 1


def test_physics_gravity_moves_body():
    game = at.Game(headless=True)
    game.load_scene(
        at.Scene().add(
            at.Entity("ball")
            .add(at.Transform(position=(0.0, 5.0, 0.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.ball(0.5))
        )
    )
    positions = []

    @game.update
    def track(ctx):
        ball = ctx.scene["ball"]
        positions.append(ball.transform.position[1])

    game.run(max_frames=30)
    # Ball falls under gravity (y decreases over 30 frames).
    assert positions[-1] < positions[0]
    assert positions[-1] < 5.0


def test_physics_body_count():
    game = at.Game(headless=True)
    game.load_scene(
        at.Scene().add(
            at.Entity("ball")
            .add(at.Transform(position=(0.0, 5.0, 0.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.ball(0.5))
        )
    )
    counts = []

    @game.update
    def track(ctx):
        counts.append(ctx.physics.body_count())

    game.run(max_frames=3)
    assert counts[-1] == 1


def test_physics_collision_events():
    game = at.Game(headless=True)
    game.load_scene(
        at.Scene()
        .add(
            at.Entity("floor")
            .add(at.Transform(position=(0.0, 0.0, 0.0)))
            .add(at.RigidBody.fixed())
            .add(at.Collider3D.cuboid(20.0, 1.0, 20.0))
        )
        .add(
            at.Entity("ball")
            .add(at.Transform(position=(0.0, 1.05, 0.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.ball(0.5))
        )
    )
    hit = []

    @game.update
    def track(ctx):
        y = ctx.scene["ball"].transform.position[1]
        hit.append(y)

    game.run(max_frames=60)
    # Ball rests on the floor (y ≈ 1.0: floor top 0.5 + ball radius 0.5).
    assert 0.8 < hit[-1] < 1.3


def test_physics_impulse():
    game = at.Game(headless=True)
    game.load_scene(
        at.Scene().add(
            at.Entity("ball")
            .add(at.Transform(position=(0.0, 50.0, 0.0)))
            .add(at.RigidBody("dynamic", gravity_scale=0.0))
            .add(at.Collider3D.ball(0.5))
        )
    )
    xs = []

    @game.update
    def push(ctx):
        ball = ctx.scene["ball"]
        if ctx.time.frame == 1:
            ctx.physics.apply_impulse(ball.id, (5.0, 0.0, 0.0))
        xs.append(ball.transform.position[0])

    game.run(max_frames=20)
    assert xs[-1] > 0.5, f"impulse should move the ball +x, got {xs[-1]}"


def test_physics_views_present_on_context():
    game = at.Game(headless=True)
    seen = {}

    @game.update
    def check(ctx):
        seen["has_physics"] = hasattr(ctx, "physics")
        seen["has_physics2d"] = hasattr(ctx, "physics2d")
        seen["gravity"] = ctx.physics.gravity()
        ctx.quit()

    game.run()
    assert seen["has_physics"]
    assert seen["has_physics2d"]
    assert abs(seen["gravity"][1] - (-9.81)) < 1e-4


# -- Phase 6: gamepads and action mapping -----------------------------------


def test_gamepads_view_present_and_empty_headless():
    game = make_game()
    seen = {}

    def update(ctx):
        pads = ctx.input.gamepads
        seen["len"] = len(pads)
        seen["ids"] = pads.ids()
        seen["iter"] = list(pads)
        seen["any"] = pads.any_connected()
        seen["primary"] = pads.primary()
        seen["events"] = pads.events
        seen["pressed_any"] = pads.pressed_any("south")
        seen["axis_any"] = pads.axis_any("left_stick_x")
        seen["axis_missing_pad"] = pads.axis(99, "left_stick_x")
        seen["get_missing"] = pads.get(99)
        ctx.quit()

    game.run(update)
    # CI sandboxes have no gamepad hardware.
    assert seen["len"] == 0
    assert seen["ids"] == []
    assert seen["iter"] == []
    assert seen["any"] is False
    assert seen["primary"] is None
    assert seen["events"] == []
    assert seen["pressed_any"] is False
    assert seen["axis_any"] == 0.0
    assert seen["axis_missing_pad"] == 0.0
    assert seen["get_missing"] is None


def test_gamepad_unknown_control_names_raise():
    game = make_game()
    errors = {}

    def update(ctx):
        pads = ctx.input.gamepads
        for label, call in {
            "button": lambda: pads.pressed_any("not_a_button"),
            "axis": lambda: pads.axis_any("not_an_axis"),
            "pad_button": lambda: pads.pressed(0, "not_a_button"),
        }.items():
            try:
                call()
            except KeyError:
                errors[label] = True
        ctx.quit()

    game.run(update)
    assert errors == {"button": True, "axis": True, "pad_button": True}


def test_binding_constructors_validate_names():
    assert at.Binding.key("space").kind == "key"
    assert at.Binding.mouse("left").kind == "mouse"
    assert at.Binding.gamepad_button("south").kind == "gamepad_button"
    assert at.Binding.gamepad_axis_positive("left_z").kind == "gamepad_axis_positive"
    assert at.Binding.gamepad_axis_negative("left_stick_y").kind == (
        "gamepad_axis_negative"
    )
    assert "space" in repr(at.Binding.key("space"))

    with pytest.raises(KeyError):
        at.Binding.key("not_a_key")
    with pytest.raises(KeyError):
        at.Binding.mouse("not_a_button")
    with pytest.raises(KeyError):
        at.Binding.gamepad_button("not_a_button")
    with pytest.raises(KeyError):
        at.Binding.gamepad_axis_positive("not_an_axis")


def test_axis_binding_constructors():
    axis = at.AxisBinding.gamepad_axis("left_stick_x", 2.0)
    assert axis.kind == "gamepad_axis"
    assert axis.name == "left_stick_x"
    assert axis.scale == 2.0
    assert axis.negative is None

    default_scale = at.AxisBinding.gamepad_axis("right_stick_y")
    assert default_scale.scale == 1.0

    pair = at.AxisBinding.key_pair("a", "d")
    assert pair.kind == "button_pair"
    assert pair.negative.kind == "key" and pair.negative.name == "a"
    assert pair.positive.name == "d"

    mixed = at.AxisBinding.button_pair(
        at.Binding.key("s"), at.Binding.gamepad_button("north")
    )
    assert mixed.positive.kind == "gamepad_button"

    with pytest.raises(KeyError):
        at.AxisBinding.gamepad_axis("not_an_axis")
    with pytest.raises(KeyError):
        at.AxisBinding.key_pair("a", "not_a_key")


def test_action_map_bookkeeping():
    actions = at.ActionMap()
    assert len(actions) == 0
    assert "jump" not in actions

    actions.bind("jump", at.Binding.key("space"))
    actions.bind("jump", at.Binding.gamepad_button("south"))
    assert len(actions) == 1
    assert "jump" in actions
    assert actions.actions() == ["jump"]
    bound = actions.bindings("jump")
    assert [b.kind for b in bound] == ["key", "gamepad_button"]

    actions.rebind("jump", [at.Binding.key("enter")])
    assert [b.name for b in actions.bindings("jump")] == ["enter"]

    actions.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
    assert actions.axis_names() == ["move_x"]

    actions.unbind("jump")
    assert len(actions) == 0
    assert actions.bindings("jump") == []


def test_action_map_evaluates_against_frame_input():
    game = make_game()
    actions = at.ActionMap()
    actions.bind("jump", at.Binding.key("space"))
    actions.bind("jump", at.Binding.gamepad_button("south"))
    actions.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
    seen = {}

    def update(ctx):
        # Headless mode has no devices, so nothing is active; this still
        # exercises the full evaluation path against the frame snapshot.
        seen["pressed"] = actions.pressed("jump", ctx.input)
        seen["just"] = actions.just_pressed("jump", ctx.input)
        seen["unbound"] = actions.pressed("fire", ctx.input)
        seen["axis"] = actions.axis("move_x", ctx.input)
        seen["axis_unbound"] = actions.axis("move_y", ctx.input)
        seen["pair"] = actions.axis_pair("move_x", "move_y", ctx.input)
        ctx.quit()

    game.run(update)
    assert seen["pressed"] is False
    assert seen["just"] is False
    assert seen["unbound"] is False
    assert seen["axis"] == 0.0
    assert seen["axis_unbound"] == 0.0
    assert seen["pair"] == (0.0, 0.0)


def test_action_map_survives_across_frames():
    game = make_game()
    actions = at.ActionMap()
    actions.bind("quit", at.Binding.key("escape"))
    frames = []

    def update(ctx):
        frames.append(ctx.time.frame)
        # The map is plain Python-visible state, usable every frame.
        assert "quit" in actions

    game.run(update, max_frames=5)
    assert frames == [1, 2, 3, 4, 5]


def test_character_controller_driven_per_frame():
    game = make_game()
    scene = at.Scene()
    scene.add(
        at.Entity("floor")
        .add(at.Transform(position=(0.0, 0.0, 0.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(10.0, 0.5, 10.0))
    )
    scene.add(
        at.Entity("player")
        .add(at.Transform(position=(0.0, 2.0, 0.0)))
        .add(at.RigidBody.kinematic_velocity())
        .add(at.Collider3D.capsule(0.5, 0.4))
        .add(at.CharacterController())
        .add(at.CharacterVelocity())
    )
    game.load_scene(scene)

    actions = at.ActionMap()
    actions.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
    seen = {}

    def update(ctx):
        player = ctx.scene["player"]
        dx = actions.axis("move_x", ctx.input)
        # Headless has no devices (dx == 0); add a constant walk speed plus
        # a gravity-ish fall so the controller exercises both axes.
        player.character_velocity = (dx + 1.0, -3.0, 0.0)
        seen.setdefault("start", player.transform.position)
        seen["end"] = player.transform.position
        seen["grounded"] = ctx.physics.grounded(player)
        seen["readback"] = player.character_velocity

    game.run(update, max_frames=90)
    sx, sy, _ = seen["start"]
    ex, ey, _ = seen["end"]
    assert ex > sx + 0.5, f"expected the character to walk +x, got {sx} -> {ex}"
    assert ey < 1.5, f"expected the character to land, got y={ey}"
    assert seen["grounded"] is True
    assert seen["readback"] == (1.0, -3.0, 0.0)


def test_character_velocity_setter_validates():
    game = make_game()
    scene = at.Scene().add(
        at.Entity("player")
        .add(at.RigidBody.kinematic_velocity())
        .add(at.Collider3D.ball(0.5))
        .add(at.CharacterController())
        .add(at.CharacterVelocity())
    )
    game.load_scene(scene)
    seen = {}

    def update(ctx):
        player = ctx.scene["player"]
        player.character_velocity = at.CharacterVelocity((0.0, 1.0, 0.0))
        seen["via_component"] = True
        try:
            player.character_velocity = "not a velocity"
        except TypeError:
            seen["bad_type"] = True
        ctx.quit()

    game.run(update)
    assert seen == {"via_component": True, "bad_type": True}


# -- OOP game style -----------------------------------------------------------


def test_oop_game_subclass_full_lifecycle():
    log = []

    class Platformer(at.Game):
        title = "oop test"
        headless = True
        target_fps = 60

        def on_start(self):
            self.load_scene(at.Scene().add(at.Entity("marker")))
            log.append("start")

        def on_update(self, ctx):
            log.append(ctx.time.frame)
            if ctx.time.frame == 1:
                ctx.events.send("hit", {"dmg": 5})

        def on_hit(self, payload):
            log.append(("hit", payload["dmg"]))

    game = Platformer()
    frames = game.run(max_frames=3)
    assert frames == 3
    # on_start once, three update ticks, the event lands the next frame.
    assert log == ["start", 1, 2, ("hit", 5), 3]
    assert game.state == "shutdown"


def test_oop_constructor_kwargs_beat_class_attributes():
    class Headed(at.Game):
        headless = True

    # Constructing must not error either way; there is no window in CI, so
    # only assert the headless paths actually run.
    game = Headed(title="kwarg title")
    frames = game.run(lambda ctx: ctx.quit())
    assert frames == 1

    # Direct construction is strict about option names; subclasses may add
    # their own constructor parameters.
    with pytest.raises(TypeError, match="unknown Game options"):
        at.Game(no_such_option=True)


def test_oop_on_update_not_overridden_still_raises():
    game = at.Game(headless=True)
    with pytest.raises(TypeError, match="no update callback"):
        game.run()


def test_entity_and_scene_subclasses():
    class Player(at.Entity):
        def __init__(self, name="player"):
            super().__init__(name)
            self.add(at.Transform(position=(1.0, 2.0, 3.0)))

    class Level(at.Scene):
        def __init__(self):
            super().__init__()
            self.add(Player())

    level = Level()
    assert len(level) == 1

    game = make_game()
    game.load_scene(level)
    seen = {}

    def update(ctx):
        player = ctx.scene["player"]
        seen["pos"] = player.transform.position
        ctx.quit()

    game.run(update)
    assert seen["pos"] == (1.0, 2.0, 3.0)


def test_entity_rename_and_plain_constructor():
    entity = at.Entity("cube")
    assert entity.name == "cube"
    entity.name = "renamed"
    assert entity.name == "renamed"
    entity.name = None
    assert entity.name is None


def test_decorator_style_still_wins_over_on_update():
    order = []

    class Both(at.Game):
        headless = True

        def on_update(self, ctx):
            order.append("method")

    game = Both()

    @game.update
    def tick(ctx):
        order.append("decorator")

    game.run(max_frames=2)
    assert order == ["decorator", "decorator"]


def test_entity_subclass_with_own_constructor_params():
    class Crate(at.Entity):
        def __init__(self, position=(0.0, 0.0, 0.0)):
            super().__init__()
            self.add(at.Transform(position=position))

    crate = Crate(position=(2.0, 1.0, 0.0))
    assert crate.name is None

    # Direct construction stays strict: typos must not pass silently.
    at.Scene().add(crate)  # subclass instances stage like plain entities
    with pytest.raises(TypeError, match="unexpected Entity arguments"):
        at.Entity("cube", bogus=1)
    with pytest.raises(TypeError, match="Scene takes no arguments"):
        at.Scene(nope=1)


def test_game_subclass_with_own_constructor_params():
    class Difficulty(at.Game):
        headless = True

        def __init__(self, difficulty="normal"):
            self.difficulty = difficulty
            super().__init__()

    game = Difficulty(difficulty="hard")
    assert game.difficulty == "hard"
    frames = game.run(lambda ctx: ctx.quit())
    assert frames == 1


# -- Rendering components -----------------------------------------------------


def test_mesh_renderer_component_builders():
    cube = at.MeshRenderer.cube(size=2.0, color=(0.9, 0.6, 0.2, 1.0))
    assert cube.kind == "cube"
    # Colors are stored as f32 engine-side.
    assert all(abs(a - b) < 1e-6 for a, b in zip(cube.color, (0.9, 0.6, 0.2, 1.0)))
    assert "cube" in repr(cube)

    quad = at.MeshRenderer.quad()
    assert quad.kind == "quad"
    assert quad.color == (1.0, 1.0, 1.0, 1.0)  # white default


def test_mesh_renderer_surfaces():
    plain = at.MeshRenderer.cube()
    assert plain.roughness is None and plain.metallic is None
    assert plain.emissive == (0.0, 0.0, 0.0) and plain.emissive_strength == 1.0
    gold = at.MeshRenderer.cube(color=(1.0, 0.77, 0.34, 1.0), roughness=0.3, metallic=1.0)
    assert gold.roughness == pytest.approx(0.3) and gold.metallic == pytest.approx(1.0)
    ball = at.MeshRenderer.sphere(0.5, metallic=1.0)
    assert ball.kind == "sphere"
    assert "Sphere(" in at.Scene().add(at.Entity("b").add(ball)).to_ron()
    lamp = at.MeshRenderer.quad(emissive=(1.0, 0.8, 0.5), emissive_strength=4.0)
    assert lamp.emissive == pytest.approx((1.0, 0.8, 0.5)) and lamp.emissive_strength == 4.0
    assert lamp.glow_light == 1.0
    sign = at.MeshRenderer.quad(emissive=(0.2, 1.0, 0.4), glow_light=0.0)
    assert sign.glow_light == 0.0
    assert "glow_light: 0.0" in at.Scene().add(at.Entity("s").add(sign)).to_ron()
    assert lamp.glow_shadows is True
    torch = at.MeshRenderer.sphere(emissive=(1.0, 0.5, 0.1), glow_shadows=False)
    assert torch.glow_shadows is False
    assert "glow_shadows: false" in at.Scene().add(at.Entity("t").add(torch)).to_ron()
    assert "glow_shadows" not in at.Scene().add(at.Entity("l").add(lamp)).to_ron()
    with pytest.raises(ValueError):
        at.MeshRenderer.cube(glow_light=-1.0)
    with pytest.raises(ValueError):
        at.MeshRenderer.cube(roughness=1.5)
    with pytest.raises(ValueError):
        at.MeshRenderer.cube(emissive_strength=-1.0)
    # Saved with the scene (and only when set).
    text = at.Scene().add(at.Entity("gold").add(at.Transform()).add(gold)).to_ron()
    assert "metallic: Some(1.0)" in text and "emissive" not in text
    assert "surface" not in at.Scene().add(at.Entity("p").add(plain)).to_ron()
    back = at.Scene.from_ron(text)
    assert back.to_ron() == text


def test_mesh_renderer_stages_and_runs_headless():
    # Headless mode has no GPU: visuals are inert but must not break the
    # loop, the scene, or physics.
    game = make_game()
    scene = (
        at.Scene()
        .add(at.Entity("camera").add(at.Camera3D(position=(0, 2, 8), target=(0, 0, 0))))
        .add(
            at.Entity("cube")
            .add(at.Transform(position=(0.0, 1.0, 0.0)))
            .add(at.MeshRenderer.cube(color=(0.9, 0.6, 0.2, 1.0)))
        )
    )
    game.load_scene(scene)
    seen = {}

    def update(ctx):
        seen["names"] = ctx.scene.names()
        ctx.quit()

    game.run(update)
    assert sorted(seen["names"]) == ["camera", "cube"]


def test_game_clear_color_option():
    game = at.Game(headless=True, clear_color=(0.5, 0.6, 0.7, 1.0))
    assert game.state == "created"
    with pytest.raises(ValueError):
        at.Game(headless=True, clear_color=(2.0, 0.0, 0.0, 1.0))


def test_camera_3d_read_write_via_entity_view():
    game = make_game()
    game.load_scene(
        at.Scene().add(
            at.Entity("cam").add(at.Camera3D(position=(0, 0, 8), target=(0, 0, 0)))
        )
    )
    observed = {}

    def update(ctx):
        cam = ctx.scene["cam"]
        if ctx.time.frame == 1:
            assert cam.camera_3d is not None
            assert cam.camera_3d.position == (0.0, 0.0, 8.0)
            cam.camera_3d = at.Camera3D(position=(1, 2, 3), target=(0, 1, 0), fov=45.0)
        elif ctx.time.frame == 3:
            observed["position"] = cam.camera_3d.position
            observed["target"] = cam.camera_3d.target
            observed["fov"] = cam.camera_3d.fov
            ctx.quit()

    game.run(update)
    assert observed["position"] == (1.0, 2.0, 3.0)
    assert observed["target"] == (0.0, 1.0, 0.0)
    assert observed["fov"] == pytest.approx(45.0)


def test_action_map_axis_bindings_stack():
    controls = at.ActionMap()
    controls.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
    controls.bind_axis("move_x", at.AxisBinding.key_pair("arrow_left", "arrow_right"))
    assert controls.axis_names() == ["move_x"]
    controls.unbind_axis("move_x")
    assert controls.axis_names() == []


def test_point_and_spot_light_components():
    torch = at.PointLight(color=(1.0, 0.6, 0.3), intensity=4.0, range=8.0)
    assert torch.color[1] == pytest.approx(0.6)
    assert torch.intensity == 4.0 and torch.range == 8.0
    assert torch.shadows is True and torch.radius == pytest.approx(0.05)
    head = at.SpotLight(intensity=20.0, outer_angle=24.0, shadows=False)
    assert head.inner_angle == pytest.approx(18.0), "three quarters of the edge"
    assert head.shadows is False
    assert at.SpotLight(outer_angle=40.0, inner_angle=10.0).inner_angle == 10.0
    for bad in (
        lambda: at.PointLight(intensity=-1.0),
        lambda: at.PointLight(radius=0.0),
        lambda: at.SpotLight(outer_angle=90.0),
        lambda: at.SpotLight(outer_angle=30.0, inner_angle=40.0),
    ):
        with pytest.raises(ValueError):
            bad()
    assert "PointLight" in repr(torch) and "SpotLight" in repr(head)

    scene = at.Scene()
    scene.add(at.Entity("torch").add(at.Transform(position=(0.0, 2.0, 0.0))).add(torch))
    scene.add(at.Entity("headlight").add(at.Transform()).add(head))
    text = scene.to_ron()
    assert "point_light" in text and "spot_light" in text
    assert "radius" not in text, "defaults stay out of the file"
    assert at.Scene.from_ron(text).to_ron() == text
    kinds = [type(c).__name__ for e in scene.entities() for c in e.components()]
    assert "PointLight" in kinds and "SpotLight" in kinds

    # Stages and runs headless.
    game = make_game()
    game.load_scene(scene)
    assert game.run(lambda ctx: ctx.quit()) >= 1


def test_directional_light_component():
    light = at.DirectionalLight(
        direction=(0.0, -1.0, 0.0), color=(1.0, 0.9, 0.8), ambient=0.4, shadows=True
    )
    assert light.direction == (0.0, -1.0, 0.0)
    assert light.color[0] == pytest.approx(1.0)
    assert light.color[1] == pytest.approx(0.9)
    assert light.ambient == pytest.approx(0.4)
    assert light.shadows is True

    # Defaults: shadows on, white light.
    default = at.DirectionalLight()
    assert default.shadows is True
    assert default.shadow_distance == 120.0 and default.shadow_softness == 1.0
    assert "shadow_distance" not in at.Scene().add(at.Entity("d").add(default)).to_ron()
    crisp = at.DirectionalLight(shadow_distance=40.0, shadow_softness=0.0)
    assert crisp.shadow_distance == 40.0 and crisp.shadow_softness == 0.0
    text = at.Scene().add(at.Entity("c").add(crisp)).to_ron()
    assert "shadow_distance: 40.0" in text and "shadow_softness: 0.0" in text
    with pytest.raises(ValueError):
        at.DirectionalLight(shadow_distance=0.0)
    with pytest.raises(ValueError):
        at.DirectionalLight(shadow_softness=9.0)

    # Stages and runs headless (light is inert without a GPU).
    game = make_game()
    game.load_scene(at.Scene().add(at.Entity("sun").add(light)))
    seen = {}

    def update(ctx):
        seen["names"] = ctx.scene.names()
        ctx.quit()

    assert game.run(update) >= 1
    assert seen["names"] == ["sun"]


def test_input_enums_are_str_and_autocomplete_friendly():
    # Members ARE strings: plain-string API keeps working, enums add
    # autocomplete and typo safety.
    assert isinstance(at.Key.SPACE, str)
    assert at.Key.SPACE == "space"
    assert at.GamepadButton.SOUTH == "south"
    assert at.GamepadAxis.LEFT_STICK_X == "left_stick_x"
    assert at.MouseButton.LEFT == "left"


def test_every_key_enum_member_is_known_to_the_engine():
    # Drift guard: enums.py must mirror the native key table exactly.
    for key in at.Key:
        binding = at.Binding.key(key)  # raises KeyError if unknown
        assert binding.name == key.value


def test_every_gamepad_enum_member_is_known_to_the_engine():
    for button in at.GamepadButton:
        assert at.Binding.gamepad_button(button).name == button.value
    for axis in at.GamepadAxis:
        assert at.AxisBinding.gamepad_axis(axis).name == axis.value
    for button in at.MouseButton:
        assert at.Binding.mouse(button).name == button.value


def test_enums_work_in_action_maps():
    controls = at.ActionMap()
    controls.bind("jump", at.Binding.key(at.Key.SPACE))
    controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
    assert "jump" in controls
    assert controls.axis_names() == ["move_x"]


def test_graphics_quality_option():
    for quality in at.GraphicsQuality:
        game = at.Game(headless=True, graphics=quality)
        assert game.state == "created"
    # Plain strings work too.
    at.Game(headless=True, graphics="low")
    with pytest.raises(ValueError):
        at.Game(headless=True, graphics="extreme")


def test_graphics_quality_as_oop_class_attribute():
    class Chill(at.Game):
        headless = True
        graphics = at.GraphicsQuality.LOW

    assert Chill().state == "created"


# ---------------------------------------------------------------------------
# Scene files (RON serialization)
# ---------------------------------------------------------------------------


def _kitchen_sink_scene():
    return (
        at.Scene()
        .add(at.Entity("cam").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0))))
        .add(
            at.Entity("crate")
            .add(at.Transform(position=(0, 5, 0)))
            .add(at.MeshRenderer.cube(size=0.8, color=(0.72, 0.52, 0.30, 1.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.cuboid(0.8, 0.8, 0.8).restitution(0.3))
        )
        .add(
            at.Entity("player")
            .add(at.Transform(position=(-4, 1.4, 0)))
            .add(at.RigidBody.kinematic_velocity())
            .add(at.Collider3D.capsule(0.5, 0.4))
            .add(at.CharacterController(snap_to_ground=0.5))
            .add(at.CharacterVelocity())
        )
        .add(
            at.Entity("sun").add(
                at.DirectionalLight(direction=(-0.45, -1.0, -0.3), ambient=0.35, shadows=True)
            )
        )
        .add(at.Entity("flat").add(at.Camera2D(center=(0, 0), half_height=5.0)))
        .add(
            at.Entity("paper")
            .add(at.Collider2D.ball(0.5))
            .add(at.MeshRenderer.quad(size=2.0))
        )
    )


def test_environment_component_and_presets():
    env = at.Environment(tonemapping="aces", exposure=1.5, fog=True, fog_density=0.03)
    assert env.tonemapping == "aces"
    assert env.exposure == pytest.approx(1.5)
    assert env.fog is True
    assert at.Environment().tonemapping == "none"
    dusk = at.Environment.preset("dusk")
    assert dusk.tonemapping == "agx" and dusk.fog
    with pytest.raises(ValueError):
        at.Environment(tonemapping="filmic")
    with pytest.raises(ValueError):
        at.Environment.preset("noon")


    scene = at.Scene().add(at.Entity("look").add(dusk))
    text = scene.to_ron()
    assert "environment" in text and "Agx" in text
    back = at.Scene.from_ron(text)
    assert back.to_ron() == text


def test_environment_sky_and_sun():
    assert at.Environment().sky is False
    day = at.Environment.preset("daylight")
    assert day.sky and day.clouds > 0.0 and day.sun_size > 0.0
    env = at.Environment(sky=True, zenith_color=(0.1, 0.2, 0.6), sun_size=3.0, clouds=0.5, lens_flare=0.0)
    assert env.zenith_color == pytest.approx((0.1, 0.2, 0.6))
    assert (env.sun_size, env.clouds, env.lens_flare) == pytest.approx((3.0, 0.5, 0.0))
    assert env.ambient_occlusion == pytest.approx(1.0)
    assert at.Environment(ambient_occlusion=0.4).ambient_occlusion == pytest.approx(0.4)
    assert env.sun_glow == pytest.approx(1.0)
    text = at.Scene().add(at.Entity("look").add(env)).to_ron()
    assert "sky: true" in text and "clouds: 0.5" in text

def test_water_component_and_presets():
    water = at.Water(size=(40.0, 30.0), wave_height=0.3)
    assert water.size == pytest.approx((40.0, 30.0))
    assert water.wave_height == pytest.approx(0.3)
    assert water.foam == pytest.approx(at.Water().foam)
    ocean = at.Water.preset("ocean")
    assert ocean.wave_height > at.Water.preset("pond").wave_height
    assert ocean.endless and not at.Water.preset("pond").endless
    assert at.Water(endless=True).endless
    with pytest.raises(ValueError):
        at.Water.preset("puddle")

    scene = at.Scene().add(at.Entity("sea").add(at.Transform()).add(ocean))
    text = scene.to_ron()
    assert "water" in text
    assert at.Scene.from_ron(text).to_ron() == text

    # A scene with water runs on every preset, Beautiful included.
    for quality in ("low", "beautiful"):
        game = at.Game(headless=True, graphics=quality, fixed_delta=1 / 60)
        game.load_scene(at.Scene().add(at.Entity("sea").add(at.Transform()).add(ocean)))
        assert game.run(lambda ctx: None, max_frames=2) == 2


def test_water_caustics_and_wind():
    water = at.Water()
    assert water.caustics == pytest.approx(1.0)
    assert water.wind == pytest.approx(0.0)
    stormy = at.Water(caustics=0.5, wind=0.8)
    assert stormy.caustics == pytest.approx(0.5)
    assert stormy.wind == pytest.approx(0.8)
    text = at.Scene().add(at.Entity("sea").add(at.Transform()).add(stormy)).to_ron()
    assert "caustics: 0.5" in text and "wind: 0.8" in text
    assert at.Scene.from_ron(text).to_ron() == text


def test_audio_low_pass_muffles_and_lifts():
    game = make_game()
    seen = {}

    def update(ctx):
        seen["start"] = (ctx.audio.low_pass, ctx.audio.underwater)
        ctx.audio.set_low_pass(800.0, fade=0.1)
        seen["muffled"] = ctx.audio.low_pass
        ctx.audio.set_low_pass(None)
        seen["lifted"] = ctx.audio.low_pass
        ctx.quit()

    game.run(update, max_frames=2)
    assert seen["start"] == (None, False)
    assert seen["muffled"] == pytest.approx(800.0)
    assert seen["lifted"] is None


def test_water_floats_bodies_and_reports_its_height():
    water = at.Water(size=(20.0, 20.0), wave_height=0.0, density=2.0, drag=1.5)
    assert water.density == pytest.approx(2.0)
    assert water.drag == pytest.approx(1.5)
    assert at.Water().density == pytest.approx(2.0)
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(
        at.Scene()
        .add(at.Entity("pool").add(at.Transform(position=(0.0, 1.0, 0.0))).add(water))
        .add(
            at.Entity("crate")
            .add(at.Transform(position=(0.0, 4.0, 0.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.cuboid(1.0, 1.0, 1.0))
        )
        .add(
            at.Entity("rock")
            .add(at.Transform(position=(3.0, 1.0, 0.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.ball(0.3).density(8.0))
        )
    )
    heights = []
    ys = []

    def update(ctx):
        heights.append((ctx.physics.water_height(2.0, 3.0), ctx.physics.water_height(30.0, 0.0)))
        ys.append((ctx.scene["crate"].transform.position[1], ctx.scene["rock"].transform.position[1]))

    game.run(update, max_frames=600)
    # Collider density 1 in water of density 2: half under, centre at y = 1.
    crate_y, rock_y = ys[-1]
    assert crate_y == pytest.approx(1.0, abs=0.08)
    assert rock_y < -5.0
    inside, outside = heights[-1]
    assert inside == pytest.approx(1.0, abs=1e-4)
    assert outside is None


def test_graphics_quality_switches_at_runtime():
    game = at.Game(headless=True, graphics="high", fixed_delta=1 / 60)
    game.load_scene(at.Scene())
    seen = []

    def update(ctx):
        seen.append(ctx.screen.graphics)
        if len(seen) == 1:
            ctx.screen.set_graphics(at.GraphicsQuality.BEAUTIFUL)
        if len(seen) == 3:
            with pytest.raises(ValueError):
                ctx.screen.set_graphics("shiny")

    game.run(update, max_frames=3)
    assert seen == ["high", "beautiful", "beautiful"]


def test_scene_scripts_read_the_water_height(tmp_path):
    (tmp_path / "probe.py").write_text(
        "def update(entity, ctx):\n"
        "    entity.attributes['h'] = ctx.physics.water_height(1, 1)\n"
        "    entity.attributes['dry'] = ctx.physics.water_height(99, 0) is None\n"
    )
    scene = at.Scene()
    scene.add(at.Entity("sea").add(at.Transform(position=(0, -0.5, 0))).add(at.Water(wave_height=0)))
    probe = at.Entity("probe").add(at.Transform())
    probe.script = "probe.py"
    scene.add(probe)
    (tmp_path / "level.ron").write_text(scene.to_ron())
    seen = {}

    def after(ctx):
        seen.update(ctx.scene["probe"].attributes)

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=3, update=after)
    assert seen["h"] == pytest.approx(-0.5, abs=1e-4)
    assert seen["dry"] is True


def test_scene_scripts_splash_the_water(tmp_path):
    (tmp_path / "splasher.py").write_text(
        "def update(entity, ctx):\n"
        "    entity.attributes['h'] = ctx.water.height(1, 1)\n"
        "    ctx.water.splash(entity, 2.0)\n"
        "    ctx.water.splash((3.0, 0.0, 3.0))\n"
        "    ctx.water.splash((0.0, 0.0, 0.0), 0)\n"
        "    try:\n"
        "        ctx.water.splash((1.0, 2.0))\n"
        "    except ValueError:\n"
        "        entity.attributes['checked'] = True\n"
    )
    scene = at.Scene()
    scene.add(at.Entity("sea").add(at.Transform(position=(0, -0.5, 0))).add(at.Water(wave_height=0)))
    splasher = at.Entity("splasher").add(at.Transform(position=(1.0, 0.0, 1.0)))
    splasher.script = "splasher.py"
    scene.add(splasher)
    (tmp_path / "level.ron").write_text(scene.to_ron())
    seen = {}

    def after(ctx):
        seen.update(ctx.scene["splasher"].attributes)

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=3, update=after)
    assert seen["h"] == pytest.approx(-0.5, abs=1e-4)
    assert seen["checked"] is True


def test_engine_scripts_splash_the_water():
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(
        at.Scene().add(at.Entity("pool").add(at.Transform()).add(at.Water(wave_height=0.0)))
    )
    frames = []

    def update(ctx):
        ctx.physics.splash((1.0, 0.0, 2.0), 1.5)
        ctx.physics.splash((1.0, 0.0, 2.0))
        frames.append(ctx.physics.water_height(1.0, 2.0))

    game.run(update, max_frames=3)
    # Ripples ride on top: the waves' height (what floats) is unchanged.
    assert frames[-1] == pytest.approx(0.0, abs=1e-4)


def test_scene_ron_round_trip_staged():
    scene = _kitchen_sink_scene()
    text = scene.to_ron()
    assert "version: 1" in text
    assert 'name: Some("crate")' in text

    loaded = at.Scene.from_ron(text)
    assert len(loaded) == len(scene) == 6

    # Behavior, not just data: the loaded scene runs and answers lookups.
    seen = {}

    def probe(ctx):
        seen["names"] = ctx.scene.names()
        ctx.quit()

    game = at.Game(headless=True)
    game.load_scene(loaded)
    game.run(probe, max_frames=4)
    assert {"cam", "crate", "player", "sun", "flat", "paper"} <= set(seen["names"])


def test_scene_save_and_load_files(tmp_path):
    path = tmp_path / "level.ron"
    _kitchen_sink_scene().save(str(path))
    text = path.read_text()
    assert "primitive: Cube" in text

    loaded = at.Scene.load(str(path))
    assert len(loaded) == 6


# A scene exactly as the Oberon editor saves it (Ctrl+S): a static ground and
# a "+ Create -> Player" entity (kinematic body + Character Controller).
OBERON_SCENE = """(
    version: 1,
    entities: [
        (
            name: Some("Ground"),
            transform: Some((
                position: (0.0, -0.1, 0.0),
                rotation_euler: (0.0, 0.0, 0.0),
                scale: (20.0, 0.2, 20.0),
            )),
            mesh_visual: Some((primitive: Cube(size: 1.0), color: (0.16, 0.17, 0.2, 1.0))),
            rigid_body: Some((
                body_type: Static,
                gravity_scale: 1.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                enabled: true,
            )),
            collider_3d: Some((
                shape: Cuboid(extents: (20.0, 0.2, 20.0)),
                density: 1.0,
                friction: 0.5,
                restitution: 0.0,
                sensor: false,
                enabled: true,
            )),
        ),
        (
            name: Some("Player"),
            transform: Some((
                position: (-3.0, 0.9, 2.0),
                rotation_euler: (0.0, 0.0, 0.0),
                scale: (0.8, 1.6, 0.8),
            )),
            mesh_visual: Some((primitive: Cube(size: 1.0), color: (1.0, 0.56, 0.24, 1.0))),
            rigid_body: Some((
                body_type: KinematicPosition,
                gravity_scale: 1.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                enabled: true,
            )),
            collider_3d: Some((
                shape: Cuboid(extents: (0.8, 1.6, 0.8)),
                density: 1.0,
                friction: 0.5,
                restitution: 0.0,
                sensor: false,
                enabled: true,
            )),
            character_controller: Some((
                mode: Normal,
                max_slope_climb_angle: 0.7853982,
                min_slope_slide_angle: 0.5235988,
                autostep_height: None,
                snap_to_ground: Some(0.3),
                offset: 0.05,
                slide: true,
            )),
            character_velocity: Some((velocity: (0.0, 0.0, 0.0))),
        ),
    ],
)
"""


def test_oberon_scene_loads_and_its_player_walks(tmp_path):
    path = tmp_path / "scene.ron"
    path.write_text(OBERON_SCENE)
    scene = at.Scene.load(str(path))
    assert len(scene) == 2

    game = at.Game(headless=True)
    game.load_scene(scene)
    seen = {}

    def walk(ctx):
        player = ctx.scene["Player"]
        player.character_velocity = (3.0, -2.0, 0.0)
        seen["x"], seen["y"], _ = player.transform.position
        seen["grounded"] = ctx.physics.grounded(player)

    game.run(walk, max_frames=60)
    assert seen["x"] > -1.0  # walked ~3 m/s for ~1 s from x = -3
    assert seen["y"] == pytest.approx(0.8, abs=0.15)  # standing on the ground
    assert seen["grounded"]


def test_scene_from_ron_rejects_garbage_and_newer_versions():
    with pytest.raises(ValueError):
        at.Scene.from_ron("definitely not ron")
    with pytest.raises(ValueError):
        at.Scene.from_ron("(version: 99, entities: [])")


def test_scene_load_missing_file_raises(tmp_path):
    with pytest.raises(IOError):
        at.Scene.load(str(tmp_path / "nope.ron"))


def test_game_save_scene_captures_the_live_world(tmp_path):
    # A dynamic crate falls; the saved file must hold its *moved* transform.
    game = at.Game(headless=True)
    game.load_scene(
        at.Scene()
        .add(
            at.Entity("floor")
            .add(at.Transform(scale=(20, 1, 20)))
            .add(at.RigidBody.fixed())
            .add(at.Collider3D.cuboid(20, 1, 20))
        )
        .add(
            at.Entity("crate")
            .add(at.Transform(position=(0, 5, 0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.cuboid(0.8, 0.8, 0.8).restitution(0.3))
        )
    )
    game.run(lambda ctx: None, max_frames=240)

    path = tmp_path / "live.ron"
    game.save_scene(str(path))
    text = path.read_text()
    assert 'name: Some("crate")' in text

    # The crate rests at y ~ 0.9 — the file records that, not the staged y=5.
    assert "position: (0.0, 5.0, 0.0)" not in text

    # Behavioral proof: the saved file loads and the crate starts at rest.
    game2 = at.Game(headless=True)
    game2.load_scene(at.Scene.load(str(path)))
    seen = {}

    def probe(ctx):
        seen["y"] = ctx.scene["crate"].transform.position[1]
        ctx.quit()

    frames = game2.run(probe, max_frames=4)
    assert frames >= 1
    assert seen["y"] == pytest.approx(0.9, abs=0.15)


def test_game_scene_to_ron_matches_save_scene(tmp_path):
    game = at.Game(headless=True)
    game.load_scene(at.Scene().add(at.Entity("marker").add(at.Transform(position=(1, 2, 3)))))
    game.run(lambda ctx: None, max_frames=1)
    assert 'name: Some("marker")' in game.scene_to_ron()


# --- audio -----------------------------------------------------------------


def _write_wav(path, seconds=0.5, freq=440.0):
    """Write a minimal 44.1 kHz mono PCM WAV of a sine tone."""
    import math
    import struct

    sample_rate = 44100
    n = int(seconds * sample_rate)
    frames = bytearray()
    for i in range(n):
        s = int(math.sin(2.0 * math.pi * freq * i / sample_rate) * 32767)
        frames += struct.pack("<h", s)
    data_len = len(frames)
    with open(path, "wb") as f:
        f.write(b"RIFF")
        f.write(struct.pack("<I", 36 + data_len))
        f.write(b"WAVEfmt ")
        f.write(struct.pack("<IHHIIHH", 16, 1, 1, sample_rate, sample_rate * 2, 2, 16))
        f.write(b"data")
        f.write(struct.pack("<I", data_len))
        f.write(frames)


def test_audio_play_and_query(tmp_path):
    wav = tmp_path / "tone.wav"
    _write_wav(wav)
    game = make_game()
    seen = {}

    def update(ctx):
        sid = ctx.audio.play(str(wav))
        seen["id"] = sid
        seen["is_playing"] = ctx.audio.is_playing(sid)
        seen["count"] = ctx.audio.sound_count()
        ctx.quit()

    game.run(update, max_frames=2)
    assert seen["id"] >= 1
    assert seen["is_playing"] is True
    assert seen["count"] == 1


def test_audio_missing_file_raises(tmp_path):
    game = make_game()
    errors = []

    def update(ctx):
        try:
            ctx.audio.play(str(tmp_path / "nope.wav"))
        except ValueError:
            errors.append("value")
        ctx.quit()

    game.run(update, max_frames=1)
    assert errors == ["value"]


def test_audio_position_advances_headless(tmp_path):
    wav = tmp_path / "tone.wav"
    _write_wav(wav, seconds=2.0)
    game = make_game()
    positions = {}
    sid = {}

    def update(ctx):
        if ctx.time.frame == 1:
            sid["id"] = ctx.audio.play(str(wav))
            positions["start"] = ctx.audio.position(sid["id"])
        if ctx.time.frame == 30:
            positions["later"] = ctx.audio.position(sid["id"])
            ctx.quit()

    game.run(update, max_frames=40)
    assert positions["start"] == 0.0
    # The silent backend advances deterministically with the frame loop.
    assert positions["later"] > positions["start"]


def test_audio_pause_resume_stop(tmp_path):
    wav = tmp_path / "tone.wav"
    _write_wav(wav, seconds=2.0)
    game = make_game()
    states = {}
    sid = {}

    def update(ctx):
        if ctx.time.frame == 1:
            sid["id"] = ctx.audio.play(str(wav))
        elif ctx.time.frame == 2:
            ctx.audio.pause(sid["id"])
            states["paused"] = ctx.audio.is_playing(sid["id"])
        elif ctx.time.frame == 3:
            states["paused_now"] = ctx.audio.is_playing(sid["id"])
            ctx.audio.resume(sid["id"])
        elif ctx.time.frame == 4:
            states["resumed"] = ctx.audio.is_playing(sid["id"])
            ctx.audio.stop(sid["id"])
        elif ctx.time.frame == 5:
            states["after_stop_count"] = ctx.audio.sound_count()
            states["after_stop_pos"] = ctx.audio.position(sid["id"])
            ctx.quit()

    game.run(update, max_frames=6)
    # pause issues a fade; on the same frame it may still report playing...
    # by the next frame it is paused.
    assert states["paused_now"] is False
    assert states["resumed"] is True
    assert states["after_stop_count"] == 0
    assert states["after_stop_pos"] is None


def test_audio_unknown_id_raises(tmp_path):
    game = make_game()
    errors = []

    def update(ctx):
        for op in ("pause", "resume", "seek_to"):
            try:
                getattr(ctx.audio, op)(999, 0.5) if op == "seek_to" else getattr(ctx.audio, op)(999)
            except KeyError:
                errors.append(op)
        ctx.quit()

    game.run(update, max_frames=1)
    assert errors == ["pause", "resume", "seek_to"]


def test_audio_master_volume_and_loop(tmp_path):
    wav = tmp_path / "tone.wav"
    _write_wav(wav, seconds=1.0)
    game = make_game()
    seen = {}

    def update(ctx):
        if ctx.time.frame == 1:
            ctx.audio.set_master_volume(0.5)
            sid = ctx.audio.play(str(wav), loop_=True, volume=0.8, panning=-0.5)
            seen["id"] = sid
        if ctx.time.frame == 3:
            # looping sound is still playing past the (short) duration window
            seen["playing"] = ctx.audio.is_playing(seen["id"])
            ctx.audio.stop_all()
        if ctx.time.frame == 4:
            seen["count"] = ctx.audio.sound_count()
            ctx.quit()

    game.run(update, max_frames=5)
    assert seen["playing"] is True
    assert seen["count"] == 0


def test_particle_emitter_presets_fields_and_errors():
    fire = at.ParticleEmitter.preset("fire")
    assert fire.preset_name == "fire"
    assert fire == at.ParticleEmitter.preset("FIRE")
    assert repr(fire) == 'ParticleEmitter.preset("fire")'
    assert fire.blend == "additive" and fire.sprite == "flame" and fire.shape == "cone"
    assert fire.color[0] > 1.0, "fire glows"
    assert at.ParticleEmitter.presets() == ["fire", "smoke", "sparks", "dust", "splash", "magic"]
    calm = fire.replace(rate=10, color=(1, 0.5, 0.2))
    assert calm.preset_name is None and calm.rate == 10.0
    assert calm.color == pytest.approx((1.0, 0.5, 0.2, 1.0))
    assert fire.rate == 45.0, "replace copies"
    custom = at.ParticleEmitter(rate=4, sprite="puff", lit=True, size=1.5, texture=None)
    assert custom.size == (1.5, 1.5) and custom.lit and custom.texture == ""
    assert at.ParticleEmitter(rate=-3).rate == 0.0
    assert "cone_angle" in dir(fire)
    with pytest.raises(ValueError):
        at.ParticleEmitter.preset("lava")
    with pytest.raises(ValueError):
        at.ParticleEmitter(shape="donut")
    with pytest.raises(ValueError):
        at.ParticleEmitter(rate="fast")
    with pytest.raises(KeyError):
        at.ParticleEmitter(colour=(1, 0, 0))
    with pytest.raises(AttributeError):
        fire.colour  # noqa: B018


def test_particle_emitters_round_trip_through_scene_files():
    scene = at.Scene()
    scene.add(at.Entity("fire").add(at.Transform()).add(at.ParticleEmitter.preset("fire")))
    scene.add(
        at.Entity("mist").add(at.Transform()).add(at.ParticleEmitter(rate=3, shape="box"))
    )
    text = scene.to_ron()
    assert "particle_emitter: Some(" in text
    loaded = at.Scene.from_ron(text)
    emitters = {
        e.name: c
        for e in loaded.entities()
        for c in e.components()
        if isinstance(c, at.ParticleEmitter)
    }
    assert emitters["fire"] == at.ParticleEmitter.preset("fire")
    assert emitters["mist"].shape == "box" and emitters["mist"].rate == 3.0
    assert loaded.to_ron() == text


def test_engine_particles_burst_and_clean_up():
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(
        at.Scene().add(
            at.Entity("torch").add(at.Transform()).add(at.ParticleEmitter.preset("fire", rate=0))
        )
    )
    counts = []

    def update(ctx):
        if ctx.time.frame == 1:
            ctx.scene.burst("sparks", (0.0, 1.0, 0.0), direction=(0.0, 0.0, 1.0), count=10)
            ctx.scene.burst(at.ParticleEmitter.preset("splash"), (2.0, 0.0, 0.0))
            ctx.scene.burst_emitter(ctx.scene["torch"].id, 5)
            with pytest.raises(ValueError):
                ctx.scene.burst("lava", (0.0, 0.0, 0.0))
        counts.append(ctx.scene.entity_count)

    game.run(update, max_frames=150)
    # Two burst entities appear, then despawn once their particles are gone.
    assert max(counts) == 3
    assert counts[-1] == 1


def test_scene_scripts_burst_particles(tmp_path):
    (tmp_path / "gun.py").write_text(
        "def update(entity, ctx):\n"
        "    if ctx.time.frame == 1:\n"
        "        ctx.particles.burst('sparks', entity, direction=(0, 0, 1), count=8)\n"
        "        ctx.particles.burst('dust', (1, 0, 1), color=(1, 0, 0, 1))\n"
        "        ctx.particles.emit(entity, 4)\n"
        "        for bad in ('lava', 'shape'):\n"
        "            try:\n"
        "                if bad == 'lava':\n"
        "                    ctx.particles.burst('lava', (0, 0, 0))\n"
        "                else:\n"
        "                    ctx.particles.burst('fire', (0, 0, 0), shap='box')\n"
        "            except (ValueError, KeyError):\n"
        "                entity.attributes['bad_' + bad] = True\n"
        "    entity.attributes['n'] = len(list(ctx.scene))\n"
    )
    scene = at.Scene()
    gun = at.Entity("gun").add(at.Transform()).add(at.ParticleEmitter.preset("fire", rate=0))
    gun.script = "gun.py"
    scene.add(gun)
    (tmp_path / "level.ron").write_text(scene.to_ron())
    seen = []
    attrs = {}

    def after(ctx):
        seen.append(ctx.scene.entity_count)
        attrs.update(ctx.scene["gun"].attributes)

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=6, update=after)
    assert max(seen) == 3, seen
    assert attrs.get("bad_lava") and attrs.get("bad_shape"), attrs
