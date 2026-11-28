"""Tests for the Night Lights scene (a torch and two headlights).

    pytest examples/oberon/night_lights/tests
"""

import os

import automata as at
from automata.scripting import load_script

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVEL = os.path.join(GAME, "level.ron")
build_level = load_script(os.path.join(GAME, "build_level.py"))


def components(scene, kind):
    return [c for e in scene.entities() for c in e.components() if isinstance(c, kind)]


def test_level_loads_and_runs():
    assert at.run_scene(LEVEL, headless=True, max_frames=30) == 30


def test_saved_level_matches_the_build_script():
    with open(LEVEL, encoding="utf-8") as f:
        assert f.read().strip() == build_level.build().to_ron().strip()


def test_lights_are_placed_lights_not_glowing_meshes():
    scene = at.Scene.load(LEVEL)
    (torch,) = components(scene, at.PointLight)
    spots = components(scene, at.SpotLight)
    assert len(spots) == 2
    assert torch.shadows and all(s.shadows for s in spots)
    # The flame and lenses glow for the look but add no light themselves.
    text = scene.to_ron()
    assert text.count("glow_light: 0.0") == 3
