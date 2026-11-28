"""Tests for the Campfire scene (fire, smoke and embers from particle emitters).

    pytest examples/oberon/campfire/tests
"""

import os

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))


def emitters(scene):
    return {
        e.name: c
        for e in scene.entities()
        for c in e.components()
        if isinstance(c, at.ParticleEmitter)
    }


def test_saved_level_matches_the_build_script():
    with open(LEVEL, encoding="utf-8") as f:
        assert f.read().strip() == build_level.build().to_ron().strip()


def test_the_fire_burns_smokes_and_throws_embers():
    found = emitters(at.Scene.load(LEVEL))
    assert set(found) == {"Flames", "Smoke", "Embers"}
    assert found["Flames"] == at.ParticleEmitter.preset("fire")
    smoke = found["Smoke"]
    assert smoke.lit and smoke.sprite == "puff" and smoke.gravity < 0, "rises"
    embers = found["Embers"]
    # Sparks turned into a steady trickle that rises instead of falling.
    assert embers.rate > 0 and not embers.one_shot and embers.gravity < 0
    assert embers.sprite == "spark" and embers.blend == "additive"


def test_the_fire_crackles():
    pops = []

    def after(ctx):
        pops.append(ctx.scene["Embers"].attributes.get("pops", 0))

    frames = at.run_scene(LEVEL, headless=True, max_frames=300, update=after,
                          fixed_delta=1 / 60)
    assert frames == 300
    assert pops[-1] >= 1, "a pop in five seconds"
