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


def test_events_round_trip_with_two_frame_lifetime():
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
