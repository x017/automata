"""Grass, foliage scatter and the wind they sway in (headless)."""

import pytest

import automata as at


def test_grass_defaults_and_keywords():
    grass = at.Grass()
    assert grass.area == pytest.approx((0.0, 0.0))
    assert grass.density == pytest.approx(60.0)
    assert grass.height == pytest.approx(0.6)
    assert grass.draw_distance == pytest.approx(60.0)
    assert grass.strokes == 0
    assert grass.blade_count() == 0

    tall = at.Grass(area=(4.0, 2.0), height=1.2, tip_color=(0.9, 0.8, 0.2), seed=7)
    assert tall.area == pytest.approx((4.0, 2.0))
    assert tall.height == pytest.approx(1.2)
    assert tall.tip_color == pytest.approx((0.9, 0.8, 0.2))
    assert tall.seed == 7


def test_grass_area_grows_about_density_times_area():
    grass = at.Grass(area=(10.0, 10.0), density=20.0)
    blades = grass.blade_count()
    assert 1_600 < blades < 2_400
    # Thinner and it's fewer; the same seed grows the same field.
    assert at.Grass(area=(10.0, 10.0), density=5.0).blade_count() < blades / 2
    assert at.Grass(area=(10.0, 10.0), density=20.0).blade_count() == blades


def test_painting_and_erasing_grass_return_copies():
    bare = at.Grass(density=20.0)
    painted = bare.paint((0.0, 0.0, 0.0), radius=3.0)
    assert bare.strokes == 0 and bare.blade_count() == 0
    assert painted.strokes == 1
    assert painted.blade_count() > 100

    cut = painted.erase((0.0, 0.0, 0.0), radius=3.0)
    assert cut.strokes == 2
    assert cut.blade_count() < painted.blade_count() / 4

    # A half-strength dab grows a sparser patch than a full one.
    half = bare.paint((0.0, 0.0, 0.0), radius=3.0, strength=0.5)
    assert half.blade_count() < painted.blade_count()


def test_grass_equality():
    assert at.Grass(area=(2.0, 2.0)) == at.Grass(area=(2.0, 2.0))
    assert at.Grass(area=(2.0, 2.0)) != at.Grass(area=(3.0, 2.0))
    assert at.Grass().paint((1.0, 0.0, 1.0)) == at.Grass().paint((1.0, 0.0, 1.0))
    assert at.Grass().paint((1.0, 0.0, 1.0)) != at.Grass()


def test_scatter_paint_erase_and_count():
    scatter = at.Scatter(area=(20.0, 20.0), density=0.5, scale=(0.5, 2.0), seed=3)
    assert scatter.scale == pytest.approx((0.5, 2.0))
    count = scatter.count()
    assert 120 < count < 280

    assert at.Scatter().count() == 0
    dabbed = at.Scatter(density=1.0).paint((0.0, 0.0, 0.0), radius=4.0)
    assert dabbed.strokes == 1 and dabbed.count() > 10
    assert dabbed.erase((0.0, 0.0, 0.0), radius=6.0).count() < dabbed.count() / 4
    assert at.Scatter(seed=1) == at.Scatter(seed=1)
    assert at.Scatter(seed=1) != at.Scatter(seed=2)


def test_grass_and_scatter_round_trip_through_scene_files(tmp_path):
    grass = at.Grass(area=(6.0, 6.0), height=0.9).paint((5.0, 0.0, 0.0), radius=2.0)
    rock = at.Scatter(density=0.3).paint((1.0, 0.0, 1.0), radius=3.0)
    scene = (
        at.Scene()
        .add(at.Entity("meadow").add(at.Transform()).add(grass))
        .add(at.Entity("rocks").add(at.Transform()).add(at.MeshRenderer.cube()).add(rock))
    )
    text = scene.to_ron()
    assert "grass" in text and "scatter" in text and "strokes" in text
    assert at.Scene.from_ron(text).to_ron() == text

    path = tmp_path / "meadow.ron"
    scene.save(str(path))
    back = at.Scene.load(str(path))
    meadow = back.find("meadow")
    assert meadow is not None
    (loaded,) = [c for c in meadow.components() if isinstance(c, at.Grass)]
    assert loaded == grass
    assert loaded.blade_count() == grass.blade_count()


def test_environment_wind():
    env = at.Environment()
    assert env.wind == pytest.approx(0.0)
    breezy = at.Environment(wind=0.6, wind_direction=90.0)
    assert breezy.wind == pytest.approx(0.6)
    assert breezy.wind_direction == pytest.approx(90.0)
    text = at.Scene().add(at.Entity("look").add(breezy)).to_ron()
    assert "wind: 0.6" in text and "wind_direction: 90.0" in text


def test_a_windy_meadow_runs_headless():
    scene = (
        at.Scene()
        .add(at.Entity("look").add(at.Environment(wind=0.8)))
        .add(at.Entity("meadow").add(at.Transform()).add(at.Grass(area=(8.0, 8.0))))
        .add(
            at.Entity("rocks")
            .add(at.Transform())
            .add(at.MeshRenderer.cube())
            .add(at.Scatter(area=(8.0, 8.0)))
        )
    )
    for quality in ("low", "high"):
        game = at.Game(headless=True, graphics=quality, fixed_delta=1 / 60)
        game.load_scene(scene)
        assert game.run(lambda ctx: None, max_frames=3) == 3
