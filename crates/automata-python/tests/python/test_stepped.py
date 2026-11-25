"""Stepped runs: how games run on the web, where the browser calls each
frame. `_core._use_stepped_runs(True)` makes windowed games run stepped with
a windowless driver, so this runs anywhere."""

import pytest

import automata as at
from automata import _core


@pytest.fixture
def stepped():
    _core._use_stepped_runs(True)
    yield
    _core._use_stepped_runs(False)


def test_run_returns_at_once_and_the_page_steps_frames(stepped):
    game = at.Game(title="stepped", target_fps=None)
    frames = []
    finished = []

    def update(ctx):
        frames.append(ctx.time.frame)
        if ctx.time.frame == 3:
            ctx.quit()

    assert game.run(update) == 0
    assert game.stepping and frames == []
    game.on_finish(lambda: finished.append(True))
    assert _core.step_running_game()
    assert _core.step_running_game()
    assert not _core.step_running_game(), "the third frame quits"
    assert frames == [1, 2, 3] and finished == [True]
    assert not game.stepping
    assert not _core.step_running_game(), "nothing runs any more"
    with pytest.raises(at.Error):
        game.run(update)


def test_max_frames_ends_a_stepped_run(stepped):
    game = at.Game(title="stepped", target_fps=None)
    game.run(lambda ctx: None, max_frames=2)
    assert _core.step_running_game()
    assert not _core.step_running_game()


def test_an_exception_ends_the_run_and_reaches_the_page(stepped):
    game = at.Game(title="stepped", target_fps=None)
    finished = []

    def update(ctx):
        raise ValueError("boom")

    game.run(update)
    game.on_finish(lambda: finished.append(True))
    with pytest.raises(ValueError, match="boom"):
        _core.step_running_game()
    assert finished == [True] and not game.stepping


def test_scene_scripts_stop_when_a_stepped_run_ends(stepped, tmp_path, monkeypatch):
    from automata import scripting

    path = tmp_path / "empty.scene.ron"
    at.Game(title="save", headless=True).save_scene(str(path))
    stops = []
    original = scripting.SceneScripts.stop
    monkeypatch.setattr(scripting.SceneScripts, "stop",
                        lambda self: (stops.append(True), original(self)))
    assert at.run_scene(str(path), max_frames=2, save_dir=False, target_fps=None) == 0
    assert stops == [], "the scripts keep running while the page steps"
    assert _core.step_running_game()
    assert not _core.step_running_game()
    assert stops == [True]


def test_headless_games_still_run_their_own_loop(stepped):
    game = at.Game(title="stepped", headless=True, target_fps=None)
    assert game.run(lambda ctx: None, max_frames=4) == 4
    assert not game.stepping


def test_on_finish_outside_a_stepped_run_calls_at_once():
    game = at.Game(title="plain", headless=True)
    called = []
    game.on_finish(lambda: called.append(True))
    assert called == [True]
