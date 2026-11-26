"""Animation graphs from Python: the component's ``graph``, ``view.animation``
in a Game, and ``entity.animation`` in scene scripts."""

from pathlib import Path

import pytest

import automata as at

ROBOT = Path(__file__).resolve().parents[4] / "examples/oberon/robot_plaza/models/robot.glb"

GRAPH = """(
    parameters: [
        (name: "speed", kind: Float(0.0)),
        (name: "grounded", kind: Bool(true)),
        (name: "slot", kind: Int(1)),
        (name: "jump", kind: Trigger),
        (name: "wave", kind: Trigger),
    ],
    layers: [
        (
            name: "Base",
            states: [
                (name: "Move", motion: Blend1D(param: "speed", points: [
                    (clip: "Idle", at: 0.0),
                    (clip: "Walking", at: 2.0),
                    (clip: "Running", at: 5.0),
                ])),
                (name: "Jump", motion: Clip("Jump"), looping: false),
                (name: "Dance", motion: Clip("Dance")),
            ],
            transitions: [
                (from: "*", to: "Jump", when: [Triggered("jump")], fade: 0.1),
                (from: "Jump", to: "Move", when: [IsTrue("grounded")], fade: 0.2, exit_time: 0.9),
            ],
        ),
        (
            name: "Arms",
            mask: ["Torso"],
            states: [
                (name: "Off", motion: Empty),
                (name: "Wave", motion: Clip("Wave"), looping: false),
            ],
            transitions: [
                (from: "Off", to: "Wave", when: [Triggered("wave")], fade: 0.2),
                (from: "Wave", to: "Off", fade: 0.2),
            ],
        ),
    ],
)
"""


@pytest.fixture
def graph(tmp_path):
    path = tmp_path / "robot.anim.ron"
    path.write_text(GRAPH)
    return path


def robot(graph_path):
    return (
        at.Entity("bot")
        .add(at.Transform())
        .add(at.MeshRenderer.model(str(ROBOT)))
        .add(at.Animation("Idle", graph=str(graph_path)))
    )


def test_the_component_names_a_graph(graph):
    anim = at.Animation(graph="anims/robot.anim.ron")
    assert anim.graph == "anims/robot.anim.ron"
    assert "graph='anims/robot.anim.ron'" in repr(anim) or 'graph="anims' in repr(anim)
    assert at.Animation().graph == ""
    text = at.Scene().add(robot(graph)).to_ron()
    assert "graph:" in text
    loaded = at.Scene.from_ron(text)
    (got,) = [c for e in loaded.entities() for c in e.components() if isinstance(c, at.Animation)]
    assert got.graph == str(graph)


def test_a_game_steers_a_graph_with_parameters(graph):
    game = at.Game(headless=True, fixed_delta=1 / 60)
    game.load_scene(at.Scene().add(robot(graph)))
    seen = {}

    def update(ctx):
        anim = ctx.scene["bot"].animation
        frame = ctx.time.frame
        if frame == 1:
            seen["start"] = (anim.graph, anim.state, anim.clip, anim.layers)
            seen["params"] = anim.parameters
            with pytest.raises(KeyError):
                anim.set("sped", 1.0)
            with pytest.raises(TypeError):
                anim.set("jump", True)
            with pytest.raises(TypeError):
                anim.trigger("speed")
            with pytest.raises(KeyError):
                anim.play_state("Moonwalk")
            with pytest.raises(KeyError):
                anim.layer_state("Legs")
            anim.set("speed", 5.0)
            anim.set("slot", 2.4)
        if frame == 3:
            seen["running"] = (anim.state, anim.clip, anim.get("speed"), anim.get("slot"))
            anim.trigger("jump")
            anim.trigger("wave")
        if frame == 5:
            seen["jumping"] = (anim.state, anim.in_transition, anim.get("jump"))
            seen["arms"] = anim.layer_state("Arms")
            anim.set_layer_weight("Arms", 0.25)
        if frame == 7:
            seen["weight"] = anim.layer_weight("Arms")
            anim.play("Dance", fade=0.0)
        if frame == 9:
            seen["dance"] = (anim.state, anim.clip)
            anim.graph = None
        if frame == 11:
            seen["no_graph"] = (anim.graph, anim.state, anim.parameters)
            with pytest.raises(RuntimeError):
                anim.set("speed", 1.0)

    game.run(update, max_frames=12)
    assert seen["start"] == (str(graph), "Move", "Idle", ["Base", "Arms"])
    assert seen["params"] == {
        "speed": 0.0,
        "grounded": True,
        "slot": 1,
        "jump": False,
        "wave": False,
    }
    assert seen["running"] == ("Move", "Running", 5.0, 2)
    assert seen["jumping"] == ("Jump", True, False), "the trigger was used up"
    assert seen["arms"] == "Wave"
    assert seen["weight"] == pytest.approx(0.25)
    assert seen["dance"] == ("Dance", "Dance")
    assert seen["no_graph"] == (None, None, {})


def test_scene_scripts_set_parameters_and_read_states(tmp_path, graph):
    (tmp_path / "bot.py").write_text(
        "def update(entity, ctx):\n"
        "    anim = entity.animation\n"
        "    a = entity.attributes\n"
        "    f = ctx.time.frame\n"
        "    if f == 2:\n"
        "        a['state'] = anim.state\n"
        "        a['layers'] = ','.join(anim.layers)\n"
        "        anim.set('speed', 2.0)\n"
        "        a['speed_now'] = anim.get('speed')\n"
        "        anim.set('grounded', False)\n"
        "        a['grounded_now'] = anim.get('grounded')\n"
        "        try:\n"
        "            anim.set('nope', 1)\n"
        "        except KeyError:\n"
        "            a['bad_name'] = True\n"
        "        try:\n"
        "            anim.set('jump', 1)\n"
        "        except TypeError:\n"
        "            a['bad_kind'] = True\n"
        "    if f == 4:\n"
        "        a['clip'] = anim.clip\n"
        "        anim.trigger('jump')\n"
        "        a['jump_now'] = anim.get('jump')\n"
        "    if f == 6:\n"
        "        a['jumping'] = anim.state\n"
        "        a['jump_used'] = anim.get('jump')\n"
        "        anim.play_state('Wave', layer='Arms', fade=0.1)\n"
        "        a['arms_now'] = anim.layer_state('Arms')\n"
        "        anim.play('Dance')\n"
        "        a['dance_now'] = anim.state\n"
        "    if f == 8:\n"
        "        a['dance'] = anim.state\n"
        "        a['arms'] = anim.layer_state('Arms')\n"
        "        a['params'] = len(anim.parameters)\n"
    )
    scene = at.Scene()
    bot = robot(graph)
    bot.script = "bot.py"
    scene.add(bot)
    (tmp_path / "level.ron").write_text(scene.to_ron())
    attrs = {}
    at.run_scene(
        str(tmp_path / "level.ron"),
        headless=True,
        max_frames=10,
        update=lambda ctx: attrs.update(ctx.scene["bot"].attributes),
    )
    assert attrs["state"] == "Move" and attrs["layers"] == "Base,Arms"
    assert attrs["speed_now"] == 2.0, "reads see this frame's requests"
    assert attrs["grounded_now"] is False
    assert attrs["bad_name"] and attrs["bad_kind"]
    assert attrs["clip"] == "Walking"
    assert attrs["jump_now"] is True
    assert attrs["jumping"] == "Jump" and attrs["jump_used"] is False
    assert attrs["arms_now"] == "Wave" and attrs["dance_now"] == "Dance"
    assert attrs["dance"] == "Dance" and attrs["arms"] == "Wave"
    assert attrs["params"] == 5
