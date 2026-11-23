"""Physics joints from Python: the `Joint` component, `view.joint` in a
game's update, and `entity.joint` in behaviour scripts."""

import pytest

import automata as at
from automata import script_api as api

def door_scene(script=None):
    """A frame post and a 1 m door hinged on its left edge."""
    door = (
        at.Entity("Door")
        .add(at.Transform(position=(0.6, 1.0, 0.0)))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.cuboid(1.0, 2.0, 0.1))
        .add(
            at.Joint.hinge(
                axis=(0, 1, 0),
                connected="Frame",
                anchor=(-0.6, 0.0, 0.0),
                limits=(-1.6, 1.6),
            )
        )
    )
    door.script = script
    frame = (
        at.Entity("Frame")
        .add(at.Transform(position=(0.0, 1.0, 0.0)))
        .add(at.Collider3D.cuboid(0.2, 2.0, 0.2))
    )
    return at.Scene().add(frame).add(door)

def test_joint_builders_and_options():
    j = at.Joint.hinge(axis=(0, 0, 1), connected="Wheelbase", limits=(1.0, -1.0), motor=2.5)
    assert j.kind == "hinge" and j.axis == (0, 0, 1)
    assert j.connected == "Wheelbase"
    assert j.limits == (-1.0, 1.0)
    assert j.motor == at.JointMotor(2.5)
    assert at.Joint.rope(3.0).length == 3.0
    s = at.Joint.spring(1.0, 50.0, 4.0)
    assert (s.kind, s.length, s.stiffness, s.damping) == ("spring", 1.0, 50.0, 4.0)
    assert at.Joint("BALL").kind == "ball"
    off = j.replace(enabled=False, motor=None)
    assert not off.enabled and off.motor is None and j.enabled
    assert at.Joint.fixed() == at.Joint("fixed") != at.Joint.ball()
    with pytest.raises(ValueError, match="unknown joint kind"):
        at.Joint("wobbly")
    with pytest.raises(TypeError, match='unexpected keyword argument "conected"'):
        at.Joint.ball(conected="Hook")
    with pytest.raises(TypeError, match="motor must be"):
        at.Joint.hinge(motor="fast")
    assert repr(at.Joint.ball(connected="Hook")) == 'Joint.ball(connected="Hook")'

def test_joints_round_trip_through_ron():
    scene = door_scene()
    back = at.Scene.from_ron(scene.to_ron())
    door = back.find("Door")
    joint = next(c for c in door.components() if isinstance(c, at.Joint))
    assert joint.connected == "Frame" and joint.limits == pytest.approx((-1.6, 1.6))
    assert joint.anchor == pytest.approx((-0.6, 0.0, 0.0))

def test_a_motor_swings_the_door_and_scripts_see_the_angle():
    game = at.Game(headless=True)
    game.load_scene(door_scene())
    angles = []

    @game.update
    def drive(ctx):
        joint = ctx.scene["Door"].joint
        assert joint is not None and ctx.scene["Frame"].joint is None
        if ctx.time.frame == 1:
            assert joint.motor_target is None
            joint.motor_target = 1.2
            assert joint.motor_target == pytest.approx(1.2), "reads see this frame's write"
        angles.append(joint.angle)

    game.run(max_frames=150)
    assert angles[0] == pytest.approx(0.0, abs=1e-3)
    assert angles[-1] == pytest.approx(1.2, abs=0.05)

def test_letting_go_drops_the_load():
    game = at.Game(headless=True)
    game.load_scene(
        at.Scene()
        .add(at.Entity("Hook").add(at.Transform(position=(0.0, 5.0, 0.0))))
        .add(
            at.Entity("Load")
            .add(at.Transform(position=(0.0, 4.0, 0.0)))
            .add(at.RigidBody.dynamic())
            .add(at.Collider3D.ball(0.3))
            .add(at.Joint.rope(connected="Hook"))
        )
    )
    heights = []

    @game.update
    def watch(ctx):
        load = ctx.scene["Load"]
        heights.append(load.transform.position[1])
        if ctx.time.frame == 30:
            assert load.joint.attached
            load.joint.enabled = False

    game.run(max_frames=60)
    assert heights[29] == pytest.approx(4.0, abs=0.05)
    assert heights[-1] < 3.5

def test_scripts_drive_joints_by_key_in_run_scene(tmp_path):
    (tmp_path / "door.py").write_text(
        "def start(entity, ctx):\n"
        "    ctx.set('Door:joint:motor_target', 0.8)\n"
        "def update(entity, ctx):\n"
        "    entity.attributes['angle'] = entity.joint.angle\n"
        "    entity.attributes['kind'] = ctx.get('Door:joint:kind')\n"
    )
    scene = door_scene("door.py")
    (tmp_path / "level.ron").write_text(scene.to_ron())
    attrs = {}

    def after(ctx):
        attrs.update(ctx.scene["Door"].attributes)

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=120, update=after)
    assert attrs["kind"] == "hinge"
    assert attrs["angle"] == pytest.approx(0.8, abs=0.05)

class Backend(api.Backend):
    def error(self, report):
        raise AssertionError(report["traceback"])

def joint_record(**over):
    rec = {
        "kind": "hinge", "to": "Frame", "on": True, "value": 0.5, "attached": True,
        "broken": False, "force": 12.0, "motor": None, "limits": [-1.0, 1.0], "bf": 0.0,
    }
    rec.update(over)
    return rec

def test_the_script_host_writes_only_what_changed_and_announces_breaks(tmp_path):
    script = tmp_path / "plank.py"
    script.write_text(
        "def start(entity, ctx):\n"
        "    ctx.subscribe('*:joint:broken', lambda m: entity.state.setdefault('heard', []).append((m.channel, m.data)))\n"
        "def update(entity, ctx):\n"
        "    j = entity.joint\n"
        "    if ctx.time.frame == 1:\n"
        "        j.motor_speed = 2.0\n"
        "        j.limits = (1.0, -0.5)\n"
        "        assert j.motor_speed == 2.0 and j.motor_target is None\n"
        "        entity.state['angle'] = j.angle\n"
        "def on_joint_broken(entity, ctx):\n"
        "    entity.state['snapped'] = entity.joint.force\n"
    )
    host = api.ScriptHost(Backend())
    rec = {"id": 1, "name": "Plank", "script": str(script), "joint": joint_record()}
    out = host.frame([rec], delta=1 / 60, frame=1)
    assert out.writes == [{"id": 1, "joint": {"motor": [2.0, None], "limits": [-0.5, 1.0]}}]
    plank = host.entities[1]
    assert plank.state["angle"] == 0.5
    assert host.frame([rec], delta=1 / 60, frame=2).writes == []
    snapped = dict(rec, joint=joint_record(broken=True, attached=False, force=900.0))
    host.frame([snapped], delta=1 / 60, frame=3)
    assert plank.state["snapped"] == 900.0
    assert plank.state["heard"] == [("Plank:joint:broken", 900.0)]
    host.frame([snapped], delta=1 / 60, frame=4)
    assert len(plank.state["heard"]) == 1, "announced once"
    # No Joint: None, and keys say so.
    host.frame([{"id": 2, "name": "Rock"}], delta=1 / 60, frame=5)
    assert host.entities[2].joint is None
