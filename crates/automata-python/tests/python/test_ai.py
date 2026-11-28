"""AI helpers: state machines, behaviour trees, sight and hearing (C5)."""

import math
import os

import pytest

import automata as at
from automata import ai
from automata.script_api import AI


class FakeTime:
    def __init__(self):
        self.delta = 0.1
        self.elapsed = 0.0
        self.frame = 1

    def step(self, n=1):
        for _ in range(n):
            self.elapsed += self.delta
            self.frame += 1


class FakeTransform:
    def __init__(self, position=(0.0, 0.0, 0.0), rotation=(0.0, 0.0, 0.0)):
        self.position = position
        self.rotation_euler = rotation


class FakeEntity:
    _next = 1

    def __init__(self, name, position=(0.0, 0.0, 0.0), rotation=(0.0, 0.0, 0.0)):
        self.name = name
        self.id = FakeEntity._next
        FakeEntity._next += 1
        self.transform = FakeTransform(position, rotation)
        self.state = {}


class FakeHit:
    def __init__(self, entity, distance, point=(0.0, 0.0, 0.0)):
        self.entity = entity
        self.distance = distance
        self.point = point


class FakePhysics:
    def __init__(self):
        self.hit = None
        self.rays = []

    def raycast(self, origin, direction, max_distance, ignore=None):
        self.rays.append((origin, direction, max_distance, ignore))
        return self.hit


class FakeScene:
    def __init__(self, *entities):
        self.entities = list(entities)

    def find(self, name):
        return next((e for e in self.entities if e.name == name), None)

    def __iter__(self):
        return iter(self.entities)


class FakeCtx:
    def __init__(self, *entities):
        self.time = FakeTime()
        self.physics = FakePhysics()
        self.scene = FakeScene(*entities)
        self.ai = AI(self)


# --------------------------------------------------------------------------
# State machines
# --------------------------------------------------------------------------


def test_a_state_machine_enters_updates_and_switches_by_returning_a_name():
    calls = []
    guard = FakeEntity("Guard")
    ctx = FakeCtx(guard)

    def patrol(entity, ctx):
        calls.append("patrol")
        if entity.state.get("alarm"):
            return "chase"

    brain = ctx.ai.state_machine(
        {
            "patrol": patrol,
            "chase": {
                "enter": lambda e, c: calls.append("enter chase"),
                "update": lambda e, c, m: calls.append(f"chase {m.time:.1f}"),
                "exit": lambda e, c: calls.append("exit chase"),
            },
        },
        initial="patrol",
    )
    assert brain.state == "patrol"
    brain.update(guard, ctx)
    guard.state["alarm"] = True
    brain.update(guard, ctx)
    assert brain.state == "chase" and brain.previous == "patrol"
    brain.update(guard, ctx)
    assert calls == ["patrol", "patrol", "enter chase", "chase 0.1"]
    brain.go("patrol")
    assert calls[-1] == "exit chase" and brain.state == "patrol"
    assert brain.history == ["patrol", "chase", "patrol"]


def test_transitions_are_checked_before_the_state_runs():
    guard = FakeEntity("Guard")
    ctx = FakeCtx(guard)
    ran = []
    brain = ai.StateMachine()
    brain.add("idle", lambda e, c: ran.append("idle"))
    brain.add("alert", update=lambda e, c: ran.append("alert"))
    brain.when("*", "alert", lambda e, c: e.state.get("noise", False))
    brain.update(guard, ctx)
    guard.state["noise"] = True
    brain.update(guard, ctx)
    assert ran == ["idle", "alert"]
    assert brain.is_in("alert")


def test_state_time_counts_and_resets_on_a_switch():
    guard = FakeEntity("Guard")
    ctx = FakeCtx(guard)
    changes = []
    brain = ctx.ai.state_machine(
        {"wait": lambda e, c, m: "go" if m.time >= 0.25 else None, "go": lambda e, c: None},
        on_change=lambda e, c, old, new: changes.append((old, new)),
    )
    for _ in range(5):
        brain.update(guard, ctx)
    assert brain.state == "go" and brain.time == 0.1
    assert changes == [("wait", "go")]


def test_state_machine_mistakes_are_explained():
    with pytest.raises(KeyError, match="patorl"):
        ai.StateMachine({"patrol": lambda e, c: None}, initial="patorl")
    brain = ai.StateMachine({"patrol": lambda e, c: "chsae", "chase": lambda e, c: None})
    with pytest.raises(KeyError, match="did you mean 'chase'"):
        brain.update(FakeEntity("G"), FakeCtx())
    with pytest.raises(KeyError):
        brain.when("patrol", "nowhere", lambda e, c: True)


# --------------------------------------------------------------------------
# Behaviour trees
# --------------------------------------------------------------------------


def test_a_selector_prefers_the_first_branch_that_does_not_fail():
    guard = FakeEntity("Guard")
    ctx = FakeCtx(guard)
    did = []
    tree = ctx.ai.selector(
        ctx.ai.sequence(
            ctx.ai.condition(lambda e, c: e.state.get("sees")),
            lambda e, c: did.append("shoot"),
        ),
        lambda e, c: (did.append("patrol"), ai.RUNNING)[1],
    )
    assert tree.tick(guard, ctx) == ai.RUNNING
    guard.state["sees"] = True
    assert tree.tick(guard, ctx) == ai.SUCCESS
    assert did == ["patrol", "shoot"]
    assert "Condition(<lambda>) -> success" in tree.describe()


def test_a_sequence_carries_on_from_a_running_child():
    guard = FakeEntity("Guard")
    ctx = FakeCtx(guard)
    log = []
    tree = ai.Sequence(
        lambda e, c: log.append("aim"),
        ai.Wait(0.3),
        lambda e, c: log.append("fire"),
    )
    statuses = []
    for _ in range(6):
        statuses.append(tree.tick(guard, ctx))
        ctx.time.step()
    assert statuses[:4] == [ai.RUNNING, ai.RUNNING, ai.RUNNING, ai.SUCCESS]
    assert log[:3] == ["aim", "fire", "aim"], "aim once, wait, fire, then start over"


def test_a_failing_step_stops_a_sequence():
    tree = ai.Sequence(lambda e, c: True, lambda e, c: False, lambda e, c: pytest.fail("ran"))
    assert tree.tick(FakeEntity("G"), FakeCtx()) == ai.FAILURE


def test_decorators_invert_repeat_and_cool_down():
    ctx = FakeCtx()
    e = FakeEntity("G")
    assert ai.Invert(lambda e, c: False).tick(e, ctx) == ai.SUCCESS
    assert ai.Succeed(lambda e, c: False).tick(e, ctx) == ai.SUCCESS
    count = []
    rep = ai.Repeat(lambda e, c: count.append(1), times=3)
    assert [rep.tick(e, ctx) for _ in range(3)] == [ai.RUNNING, ai.RUNNING, ai.SUCCESS]
    shots = []
    gun = ai.Cooldown(0.5, lambda e, c: shots.append(c.time.elapsed))
    for _ in range(12):
        gun.tick(e, ctx)
        ctx.time.step()
    assert len(shots) == 3, shots


def test_parallel_needs_all_or_one():
    ctx = FakeCtx()
    e = FakeEntity("G")
    both = ai.Parallel(ai.Wait(0.15), lambda e, c: True)
    assert both.tick(e, ctx) == ai.RUNNING
    ctx.time.step(2)
    assert both.tick(e, ctx) == ai.SUCCESS
    first = ai.Parallel(ai.Wait(5), lambda e, c: True, need="one")
    assert first.tick(e, ctx) == ai.SUCCESS
    with pytest.raises(ValueError, match="return SUCCESS"):
        ai.Action(lambda e, c: "yes").tick(e, ctx)


# --------------------------------------------------------------------------
# Sight
# --------------------------------------------------------------------------


def test_the_sight_cone_sees_ahead_not_behind_or_too_far():
    guard = FakeEntity("Guard", (0.0, 0.0, 0.0))  # faces -Z
    ahead = FakeEntity("Ahead", (1.0, 0.0, -8.0))
    behind = FakeEntity("Behind", (0.0, 0.0, 5.0))
    far = FakeEntity("Far", (0.0, 0.0, -30.0))
    ctx = FakeCtx(guard, ahead, behind, far)
    assert ctx.ai.can_see(guard, "Ahead")
    assert not ctx.ai.can_see(guard, behind)
    assert not ctx.ai.can_see(guard, far)
    assert ctx.ai.can_see(guard, far, distance=40)
    assert ctx.ai.can_see(guard, behind, fov=360)
    assert ctx.ai.can_see(guard, behind, forward=(0, 0, 1)), "a model facing +Z"
    assert ctx.ai.visible(guard) == [ahead]
    # Turned round (180 degrees about Y), it sees the other way.
    guard.transform.rotation_euler = (0.0, math.pi, 0.0)
    assert ctx.ai.visible(guard) == [behind]
    # A narrow cone misses what's off to the side.
    guard.transform.rotation_euler = (0.0, 0.0, 0.0)
    assert not ctx.ai.can_see(guard, (6.0, 1.6, -6.0), fov=60)
    assert ctx.ai.can_see(guard, (6.0, 1.6, -6.0), fov=100)


def test_walls_block_sight_and_the_target_itself_does_not():
    guard = FakeEntity("Guard")
    player = FakeEntity("Player", (0.0, 0.0, -10.0))
    wall = FakeEntity("Wall", (0.0, 0.0, -5.0))
    ctx = FakeCtx(guard, player, wall)
    ctx.physics.hit = FakeHit(wall, 5.0)
    assert not ctx.ai.can_see(guard, player)
    assert ctx.ai.can_see(guard, player, walls=False)
    ctx.physics.hit = FakeHit(player, 9.7)
    assert ctx.ai.can_see(guard, player)
    origin, direction, length, ignore = ctx.physics.rays[-1]
    assert origin == (0.0, 1.6, 0.0) and ignore is guard
    assert length == pytest.approx(math.dist(origin, (0.0, 1.0, -10.0)))
    # An unnamed collider right at the target is the target.
    ctx.physics.hit = FakeHit(None, 9.6, point=(0.0, 1.0, -9.6))
    assert ctx.ai.can_see(guard, player)
    ctx.physics.hit = FakeHit(None, 4.0, point=(0.0, 1.2, -4.0))
    assert not ctx.ai.can_see(guard, player)


# --------------------------------------------------------------------------
# Hearing
# --------------------------------------------------------------------------


def test_noises_are_heard_once_within_their_radius():
    player = FakeEntity("Player", (0.0, 0.0, 0.0))
    near = FakeEntity("Near", (5.0, 0.0, 0.0))
    far = FakeEntity("Far", (30.0, 0.0, 0.0))
    ctx = FakeCtx(player, near, far)
    shot = ctx.ai.noise(player, radius=12, kind="shot")
    assert shot.source == "Player"
    heard = ctx.ai.hear(near)
    assert [(n.kind, n.source, n.distance) for n in heard] == [("shot", "Player", 5.0)]
    assert ctx.ai.hear(near) == [], "each noise is heard once"
    assert ctx.ai.hear(far) == []
    assert len(ctx.ai.hear(far, hearing=3.0)) == 1, "sharp ears"
    assert ctx.ai.hear(player) == [], "nobody hears themselves"


def test_noises_last_until_the_end_of_the_next_frame():
    a = FakeEntity("A")
    ctx = FakeCtx(a)
    ctx.ai.noise((1.0, 0.0, 0.0), radius=5, kind="step")
    ctx.time.step()
    assert len(ctx.ai.noises) == 1
    ctx.time.step()
    assert ctx.ai.noises == []
    assert ctx.ai.hear(a) == []
    ctx.ai.noise((1.0, 0.0, 0.0), radius=5, kind="step")
    ctx.ai.noise((2.0, 0.0, 0.0), radius=5, kind="shot")
    assert [n.kind for n in ctx.ai.hear(a, kinds="shot")] == ["shot"]
    with pytest.raises(ValueError):
        ctx.ai.noise((0, 0, 0), radius=-1)


# --------------------------------------------------------------------------
# In a real game
# --------------------------------------------------------------------------

GUARD = """
def watch(entity, ctx):
    if ctx.ai.can_see(entity, "Player", fov=90, distance=15, eye=0.5, aim=0.0):
        return "alert"

def alert(entity, ctx, brain):
    entity.attributes["alert_for"] = round(brain.time, 2)

def start(entity, ctx):
    entity.state["brain"] = ctx.ai.state_machine({"watch": watch, "alert": alert})

def update(entity, ctx):
    entity.attributes["state"] = entity.state["brain"].update(entity, ctx)
    for n in ctx.ai.hear(entity):
        entity.attributes["heard"] = n.kind
"""

SHOUTER = """
def update(entity, ctx):
    if ctx.time.frame == 3:
        ctx.ai.noise(entity, radius=20, kind="shout")
"""


def run_guard(tmp_path, wall):
    (tmp_path / "guard.py").write_text(GUARD)
    (tmp_path / "shouter.py").write_text(SHOUTER)
    scene = at.Scene()
    scene.add(at.Entity("Ground").add(at.Transform(position=(0, -0.5, 0)))
              .add(at.Collider3D.cuboid(40, 1, 40)))
    guard = at.Entity("Guard").add(at.Transform(position=(0, 0.5, 0)))
    guard.script = "guard.py"
    scene.add(guard)
    player = (at.Entity("Player").add(at.Transform(position=(0, 0.5, -8)))
              .add(at.Collider3D.ball(0.4)))
    player.script = "shouter.py"
    scene.add(player)
    if wall:
        scene.add(at.Entity("Wall").add(at.Transform(position=(0, 1, -4)))
                  .add(at.Collider3D.cuboid(4, 3, 0.4)))
    path = tmp_path / "level.ron"
    at.set_asset_root(str(tmp_path))
    scene.save(str(path))
    seen = {}

    def after(ctx):
        g = ctx.scene["Guard"]
        seen["state"] = g.attributes.get("state")
        seen["heard"] = g.attributes.get("heard")

    at.run_scene(str(path), headless=True, max_frames=10, update=after)
    return seen


def test_a_guard_sees_the_player_unless_a_wall_is_in_the_way(tmp_path):
    open_field = run_guard(tmp_path, wall=False)
    assert open_field["state"] == "alert"
    behind_wall = run_guard(tmp_path, wall=True)
    assert behind_wall["state"] == "watch"
    assert behind_wall["heard"] == "shout", "walls don't stop sound"


def test_the_ai_classes_are_importable_and_documented():
    for name in ai.__all__:
        assert getattr(ai, name).__doc__ or name in ("SUCCESS", "FAILURE", "RUNNING")
    assert os.path.basename(ai.__file__) == "ai.py"
