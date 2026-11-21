"""Every Zero to Hero module and exercise runs, and the tutorial shows these exact files.

    pytest examples/python/zero_to_hero
"""

import glob
import os
import re
import runpy

import pytest

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))
PAGE = os.path.join(HERE, "..", "..", "..", "docs", "wiki", "zero-to-hero.md")
PROGRAMS = sorted(
    glob.glob(os.path.join(HERE, "module_*.py")) + glob.glob(os.path.join(HERE, "exercises", "exercise_*.py"))
)
FRAMES = 30


@pytest.fixture
def headless(monkeypatch):
    """Runs every `Game` headless and stops it after FRAMES frames."""
    original_new = at.Game.__new__
    original_run = at.Game.run

    def new(cls, **options):
        options["headless"] = True
        return original_new(cls, **options)

    def run(self, update=None, max_frames=None):
        return original_run(self, update, max_frames=min(max_frames or FRAMES, FRAMES))

    monkeypatch.setattr(at.Game, "__new__", staticmethod(new))
    monkeypatch.setattr(at.Game, "run", run)


@pytest.mark.parametrize("path", PROGRAMS, ids=lambda p: os.path.relpath(p, HERE))
def test_program_runs(path, headless, capsys):
    runpy.run_path(path, run_name="__main__")


def test_exercise_00_prints_the_frame_count(headless, capsys):
    runpy.run_path(os.path.join(HERE, "exercises", "exercise_00.py"), run_name="__main__")
    assert "ran 30 frames" in capsys.readouterr().out


def _page_blocks():
    with open(PAGE, encoding="utf-8") as f:
        return re.findall(r"```python\n(.*?)```", f.read(), re.S)


def _source(name):
    with open(os.path.join(HERE, name), encoding="utf-8") as f:
        return f.read()


@pytest.mark.parametrize(
    "module", ["module_%02d.py" % i for i in range(10)] + ["test_module_10.py"]
)
def test_the_page_shows_each_module_in_full(module):
    assert _source(module).strip() in [b.strip() for b in _page_blocks()]


def test_every_solution_on_the_page_comes_from_its_file():
    page = open(PAGE, encoding="utf-8").read()
    solutions = re.findall(r"\*\*Solution\*\* \(\[`exercises/([\w.]+)`\].*?```python\n(.*?)```", page, re.S)
    assert len(solutions) == 10
    for name, code in solutions:
        assert code.strip() in _source(os.path.join("exercises", name)), name
