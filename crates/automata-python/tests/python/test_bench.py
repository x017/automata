"""The performance baseline's moving parts (``examples/bench``): fixed
frame deltas, the frame-stats hook in ``run_scene``, and the bench scenes
themselves running under it."""

import json
import math
import os
import sys

import pytest

import automata as at

ROOT = os.path.join(os.path.dirname(__file__), "..", "..", "..", "..")
BENCH = os.path.join(ROOT, "examples", "bench")


def test_fixed_delta_advances_every_frame_by_the_same_amount():
    game = at.Game(headless=True, fixed_delta=0.25)
    seen = []
    game.run(lambda ctx: seen.append((ctx.time.delta, ctx.time.elapsed)), max_frames=4)
    assert [round(d, 6) for d, _ in seen] == [0.25] * 4
    assert math.isclose(seen[-1][1], 1.0, abs_tol=1e-6)
    with pytest.raises(ValueError):
        at.Game(headless=True, fixed_delta=0.0)


def test_the_bench_scenes_run_and_record_frame_stats(tmp_path, monkeypatch):
    sys.path.insert(0, BENCH)
    try:
        import build_scenes
    finally:
        sys.path.remove(BENCH)
    written = build_scenes.write(str(tmp_path), kinds=("scripts", "physics"), sizes=(20,))
    assert [(k, n) for k, n, _ in written] == [("scripts", 20), ("physics", 20)]
    for kind, _, folder in written:
        stats = tmp_path / f"{kind}.json"
        monkeypatch.setenv("AUTOMATA_FRAME_STATS", str(stats))
        monkeypatch.setenv("AUTOMATA_FRAME_STATS_WARMUP", "2")
        heights = []

        def watch(ctx):
            unit = ctx.scene["Unit 0"]
            heights.append((unit.attributes["height"], unit.transform.position))

        frames = at.run_scene(
            os.path.join(folder, "bench.ron"),
            headless=True,
            max_frames=13,
            fixed_delta=1 / 60,
            update=watch,
        )
        assert frames == 13
        data = json.loads(stats.read_text())
        assert data["runner"] == "run_scene"
        assert len(data["frame_ms"]) == len(data["script_ms"]) == 10
        assert all(f >= s > 0 for f, s in zip(data["frame_ms"], data["script_ms"]))
        # The ray hit the ground, and the entity moved.
        height, position = heights[-1]
        assert 0.0 < height < 1.0
        assert position != heights[0][1]
