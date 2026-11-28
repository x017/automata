"""automata.steam against a fake steam_api library (fake_steam/), built
with the system's C compiler: achievements, stats, batching, the overlay,
older libraries, Steam missing or failing, and scenes using ctx.steam."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

from automata import steam as steam_mod

HERE = Path(__file__).parent
CC = shutil.which("cc") or shutil.which("gcc") or shutil.which("clang")

needs_cc = pytest.mark.skipif(CC is None or sys.platform == "win32",
                              reason="needs a C compiler (Linux or macOS)")


def build(folder: Path, *flags: str) -> Path:
    folder.mkdir(parents=True, exist_ok=True)
    out = folder / steam_mod.library_names()[0]
    subprocess.run([CC, "-shared", "-fPIC", "-O1", "-o", str(out),
                    str(HERE / "fake_steam" / "fake_steam_api.c"), *flags], check=True)
    return out


@pytest.fixture(scope="module")
def fake(tmp_path_factory) -> Path:
    if CC is None or sys.platform == "win32":
        pytest.skip("needs a C compiler (Linux or macOS)")
    return build(tmp_path_factory.mktemp("steam") / "new")


@pytest.fixture(scope="module")
def fake_old(tmp_path_factory) -> Path:
    if CC is None or sys.platform == "win32":
        pytest.skip("needs a C compiler (Linux or macOS)")
    return build(tmp_path_factory.mktemp("steam") / "old", "-DOLD_SDK")


def run(code: str, tmp_path: Path, lib: Path | None, **env: str) -> tuple[list[str], str]:
    """Runs ``code`` in a fresh Python (the fake keeps its state per
    process); returns the fake's call log and the output."""
    log = tmp_path / "calls.log"
    log.unlink(missing_ok=True)
    full = {k: v for k, v in os.environ.items()
            if k not in ("SteamAppId", "SteamGameId", "SteamDeck", "AUTOMATA_STEAM_API")}
    full.update(FAKE_STEAM_LOG=str(log), FAKE_STEAM_ACHIEVEMENTS="ACH_WIN,ACH_COINS",
                FAKE_STEAM_STATS="coins:i,distance:f", **env)
    if lib is not None:
        full["AUTOMATA_STEAM_API"] = str(lib)
    proc = subprocess.run([sys.executable, "-c", textwrap.dedent(code)], env=full, cwd=tmp_path,
                          capture_output=True, text=True, timeout=120)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    calls = log.read_text().splitlines() if log.exists() else []
    return calls, proc.stdout + proc.stderr


@needs_cc
def test_achievements_and_stats_reach_steam(fake, tmp_path):
    calls, out = run("""
        from automata import steam
        s = steam.init(480)
        assert s.available and s.status == "running", s.status
        assert s.app_id == 480 and s.user_name == "Fake Player" and s.language == "english"
        assert s.achievements == ["ACH_WIN", "ACH_COINS"]
        assert not s.unlocked("ACH_WIN")
        assert s.unlock("ACH_WIN") and s.unlocked("ACH_WIN")
        assert s.unlock("ACH_WIN")            # again: nothing new
        assert not s.unlock("ACH_NOPE")       # Steam doesn't know it
        assert s.set_stat("coins", 3) and s.stat("coins") == 3
        assert s.add_stat("coins", 2) == 5
        assert s.set_stat("distance", 12.5) and s.stat("distance") == 12.5
        assert s.set_stat("distance", 3) and s.stat("distance") == 3.0  # int to a float stat
        assert s.stat("missing", -1) == -1
        assert s.progress("ACH_COINS", 5, 10)
        s._flush()                            # end of frame: the unlock goes now
        s._flush()                            # nothing new
        s.set_stat("coins", 6)
        s._flush()                            # stats wait for STORE_INTERVAL
        print("stores before exit ok")
    """, tmp_path, fake)
    assert calls[0] == "InitFlat app=480"
    assert calls.count("SetAchievement ACH_WIN") == 1
    assert "SetStatInt32 coins 5" in calls and "SetStatFloat distance 12.5" in calls
    assert "IndicateAchievementProgress ACH_COINS 5" in calls
    stores = [c for c in calls if c.startswith("StoreStats")]
    # One for the unlock, one at exit for the stats changed since.
    assert stores == ["StoreStats 1", "StoreStats 2"], calls
    assert calls[-1] == "Shutdown"
    assert calls.index("StoreStats 2") < calls.index("Shutdown")
    assert "there's no achievement 'ACH_NOPE'" in out
    assert "steam: running as app 480 for Fake Player" in out


@needs_cc
def test_cloud_is_on_when_steam_says_so(fake, tmp_path):
    code = """
        from automata import steam
        print("cloud", steam.init(480).cloud)
    """
    _, out = run(code, tmp_path, fake)
    assert "cloud False" in out
    _, out = run(code, tmp_path, fake, FAKE_STEAM_CLOUD="1")
    assert "cloud True" in out
    _, out = run(code, tmp_path, None)
    assert "cloud False" in out


@needs_cc
def test_the_overlay_opens_and_closes(fake, tmp_path):
    calls, _ = run("""
        import ctypes
        from automata import steam
        s = steam.init(480)
        seen = []
        s.on_overlay(seen.append)
        assert s.overlay_enabled and not s.overlay_open
        assert s.open_overlay("achievements")
        s._frame()
        assert s.overlay_open and seen == [True]
        ctypes.CDLL(steam.find_library()).FakeSteam_SetOverlay(0)
        s._frame()
        assert not s.overlay_open and seen == [True, False]
        assert s.open_store() and s.open_url("https://example.com", modal=True)
        try:
            s.open_overlay("nope")
        except ValueError as e:
            assert "achievements" in str(e)
        else:
            raise AssertionError("no error")
    """, tmp_path, fake)
    assert "ActivateGameOverlay achievements" in calls
    assert "ActivateGameOverlayToStore 480" in calls
    assert "ActivateGameOverlayToWebPage https://example.com" in calls


@needs_cc
def test_an_older_library_waits_for_the_stats(fake_old, tmp_path):
    calls, _ = run("""
        from automata import steam
        s = steam.init(480)
        assert s.available
        # Before the stats arrive: calls wait, reads see what's waiting.
        assert s.unlock("ACH_WIN") and s.unlocked("ACH_WIN")
        assert s.set_stat("coins", 4) and s.stat("coins") == 4
        s._flush()
        s._frame()                           # UserStatsReceived_t
        assert s.unlocked("ACH_WIN") and s.stat("coins") == 4
        s._flush()
    """, tmp_path, fake_old)
    assert calls[:3] == ["Init app=480", "ManualDispatch_Init", "RequestCurrentStats"]
    assert calls.index("SetAchievement ACH_WIN") > calls.index("RequestCurrentStats")
    assert "SetStatInt32 coins 4" in calls
    assert calls.count("StoreStats 1") == 1


@needs_cc
def test_started_by_steam_or_restarting(fake, tmp_path):
    calls, _ = run("""
        from automata import steam
        s = steam.init(480, restart=True)
        assert s.available and s.app_id == 999
    """, tmp_path, fake, SteamAppId="999")
    # Steam started it: no restart, and Steam's app id wins.
    assert calls == ["InitFlat app=999", "ManualDispatch_Init", "Shutdown"]
    calls, _ = run("""
        from automata import steam
        s = steam.init(480, restart=True)
        assert not s.available and s.status == "restarting through Steam"
    """, tmp_path, fake, FAKE_STEAM_RESTART="1")
    assert calls == ["RestartAppIfNecessary 480"]


@needs_cc
def test_steam_failing_or_missing_keeps_the_game_going(fake, tmp_path):
    _, out = run("""
        from automata import steam
        s = steam.init(480)
        assert not s.available
        assert s.status == "Steam didn't start: Steam isn't running", s.status
        assert s.unlock("ACH_WIN") and s.unlocked("ACH_WIN")
        assert s.set_stat("coins", 2) and s.add_stat("coins") == 3
        assert s.user_name is None and s.achievements == [] and not s.open_overlay()
        s.clear("ACH_WIN")
        assert not s.unlocked("ACH_WIN")
    """, tmp_path, fake, FAKE_STEAM_FAIL="Steam isn't running")
    assert "steam (offline): achievement 'ACH_WIN' unlocked" in out
    _, out = run("""
        from automata import steam
        s = steam.init(480, folders=["."])
        assert not s.available and s.status.startswith("no "), s.status
        assert s.app_id == 480
    """, tmp_path, None)


def test_offline_without_init():
    steam_mod._reset_for_tests()
    try:
        s = steam_mod.client()
        logged = []
        s._log = lambda level, msg: logged.append(msg)
        assert not s.available and s.status == "not started"
        assert s.unlock("ACH_X") and s.unlocked("ACH_X")
        s.unlock("ACH_X")
        assert logged.count("steam (offline): achievement 'ACH_X' unlocked") == 1
        with pytest.raises(TypeError):
            s.set_stat("x", "3")
        with pytest.raises(ValueError):
            s.unlock("")
        s._frame()
        s._flush(final=True)
        s.shutdown()
    finally:
        steam_mod._reset_for_tests()


def test_scripts_log_through_the_game_unless_init_had_a_log():
    from automata import script_api as api

    steam_mod._reset_for_tests()
    try:
        game_log = []
        s = api._steam_client(lambda level, msg: game_log.append(msg))
        assert s is steam_mod.client()
        s.unlock("ACH_LOG")
        assert "steam (offline): achievement 'ACH_LOG' unlocked" in game_log
        steam_mod._reset_for_tests()
        own = []
        steam_mod.init(480, folders=["/nonexistent"], log=lambda level, msg: own.append(msg))
        s = api._steam_client(lambda level, msg: game_log.append(msg))
        s.unlock("ACH_OWN")
        assert any("ACH_OWN" in m for m in own)
        assert not any("ACH_OWN" in m for m in game_log)
    finally:
        steam_mod._reset_for_tests()


def test_steam_deck_from_the_environment(monkeypatch):
    monkeypatch.setenv("SteamDeck", "1")
    assert steam_mod.on_steam_deck() and steam_mod.Steam().steam_deck
    monkeypatch.setenv("SteamDeck", "0")
    assert not steam_mod.Steam().steam_deck


GAME_SCRIPT = '''
def start(entity, ctx):
    ctx.log("steam:", ctx.steam.available, ctx.steam.user_name)

def update(entity, ctx):
    if ctx.time.frame == 2:
        ctx.steam.add_stat("coins", 3)
        ctx.steam.unlock("ACH_COINS")
    if ctx.time.frame == 3:
        ctx.steam.open_overlay("achievements")
    if ctx.time.frame == 4:
        ctx.log("overlay open:", ctx.steam.overlay_open)
'''


def _game(tmp_path: Path, steam: dict | None) -> Path:
    """A game folder as an export lays it out, with the script above."""
    folder = tmp_path / "game"
    folder.mkdir()
    (folder / "game.py").write_text(GAME_SCRIPT)
    (folder / "game.json").write_text(json.dumps(
        {"name": "Steam Test", "start_scene": "level.ron", "resolution": [320, 240],
         **({"steam": steam} if steam else {})}))
    subprocess.run([sys.executable, "-c", textwrap.dedent(f"""
        import automata as at
        p = at.Entity("Player").add(at.Transform())
        p.script = "game.py"
        at.Scene().add(p).save({str(folder / "level.ron")!r})
    """)], check=True)
    return folder


PLAY = """
from automata import player
player.main([{folder!r}, "--headless", "--frames", "6"])
"""


@needs_cc
def test_an_exported_game_uses_steam(fake, tmp_path):
    folder = _game(tmp_path, {"app_id": 480})
    shutil.copy(fake, folder / fake.name)  # where the export puts it
    env = {"AUTOMATA_SAVE_DIR": str(tmp_path / "saves")}
    log = tmp_path / "calls.log"
    calls, out = run(PLAY.format(folder=str(folder)), tmp_path, None, **env)
    assert calls[0] == "InitFlat app=480", calls
    assert "SetStatInt32 coins 3" in calls and "SetAchievement ACH_COINS" in calls
    # The unlock's frame sends it; the overlay opened from a script.
    assert calls.index("StoreStats 1") > calls.index("SetAchievement ACH_COINS")
    assert "ActivateGameOverlay achievements" in calls
    assert "steam: True Fake Player" in out
    assert "overlay open: True" in out
    assert calls[-1] == "Shutdown"
    assert log.exists()


@needs_cc
def test_an_exported_game_restarts_through_steam(fake, tmp_path):
    folder = _game(tmp_path, {"app_id": 480, "restart": True})
    shutil.copy(fake, folder / fake.name)
    calls, out = run(PLAY.format(folder=str(folder)), tmp_path, None, FAKE_STEAM_RESTART="1",
                     AUTOMATA_SAVE_DIR=str(tmp_path / "saves"))
    assert calls == ["RestartAppIfNecessary 480"]
    assert "starting the game through Steam" in out
    assert "steam:" not in out.replace("steam: starting", "")


def test_a_game_without_steam_runs_offline(tmp_path):
    folder = _game(tmp_path, None)
    calls, out = run(PLAY.format(folder=str(folder)), tmp_path, None,
                     AUTOMATA_SAVE_DIR=str(tmp_path / "saves"))
    assert calls == []
    assert "steam: False None" in out
    assert "steam (offline): achievement 'ACH_COINS' unlocked" in out
    assert "overlay open: False" in out
