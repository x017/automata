"""ctx.save and ctx.settings: the shared script API's save games and
player settings (standard library only, like the rest of script_api)."""

import json
import os

import pytest

from automata import script_api as api


class _Backend(api.Backend):
    def __init__(self, gfx="high"):
        self.logs = []
        self.gfx = gfx

    def log(self, level, message):
        self.logs.append((level, message))

    def graphics(self):
        return self.gfx


def _host(tmp_path, save_dir="saves", source="", gfx="high", keys=None):
    script = tmp_path / "s.py"
    script.write_text(source or "def update(entity, ctx):\n    pass\n")
    backend = _Backend(gfx)
    keys = keys if keys is not None else api.KeySource()
    host = api.ScriptHost(backend, keys,
                          save_dir=None if save_dir is None else tmp_path / save_dir)
    rec = {"id": 1, "name": "Player", "script": str(script), "p": [0, 0, 0], "r": [0, 0, 0],
           "s": [1, 1, 1], "attrs": {}}
    return host, keys, [rec]


def _frame(host, recs, n=1):
    out = None
    for i in range(n):
        out = host.frame(recs, delta=1 / 60, elapsed=i / 60, frame=i, screen=(800, 600))
        for w in out.writes:
            if "attrs" in w:
                recs[0]["attrs"] = dict(w["attrs"])
    return out


def test_save_round_trips_through_a_json_file(tmp_path):
    host, _, recs = _host(tmp_path, source=(
        "def start(entity, ctx):\n"
        "    ctx.save['coins'] = ctx.save.get('coins', 0) + 12\n"
        "    ctx.save['spawn'] = (1.0, 2.0, 3.0)\n"
    ))
    _frame(host, recs)
    path = tmp_path / "saves" / "saves" / "default.json"
    data = json.loads(path.read_text())
    assert data["format"] == 1
    assert data["data"] == {"coins": 12, "spawn": [1.0, 2.0, 3.0]}
    assert isinstance(data["saved_at"], float)
    # A second run (a new host) reads it back and adds to it.
    host2, _, recs2 = _host(tmp_path, source=(
        "def start(entity, ctx):\n"
        "    ctx.save['coins'] += 1\n"
    ))
    _frame(host2, recs2)
    assert host2.ctx.save["coins"] == 13
    assert host2.ctx.save["spawn"] == [1.0, 2.0, 3.0]  # tuples come back as lists
    assert json.loads(path.read_text())["data"]["coins"] == 13


def test_nothing_is_written_until_something_changes(tmp_path):
    host, _, recs = _host(tmp_path, source=(
        "def update(entity, ctx):\n"
        "    ctx.save.get('coins', 0)\n"
    ))
    _frame(host, recs, 3)
    assert not (tmp_path / "saves").exists()
    host.ctx.save["coins"] = 1
    _frame(host, recs)
    path = tmp_path / "saves" / "saves" / "default.json"
    first = path.stat().st_mtime_ns
    os.utime(path, ns=(first - 10**9, first - 10**9))
    host.ctx.save["coins"] = 1  # the same value: not rewritten
    _frame(host, recs)
    assert path.stat().st_mtime_ns == first - 10**9


def test_in_place_changes_to_lists_are_saved(tmp_path):
    host, _, recs = _host(tmp_path)
    host.ctx.save["items"] = ["sword"]
    _frame(host, recs)
    host.ctx.save["items"].append("shield")
    _frame(host, recs)
    data = json.loads((tmp_path / "saves" / "saves" / "default.json").read_text())
    assert data["data"]["items"] == ["sword", "shield"]


def test_slots_are_separate_files(tmp_path):
    host, _, recs = _host(tmp_path)
    save = host.ctx.save
    assert save.slot == "default"
    save["level"] = 1
    save.use(2)
    assert save.slot == "2"
    assert "level" not in save
    save["level"] = 5
    save.use("hard mode")
    save["level"] = 9
    _frame(host, recs)
    names = [s.name for s in save.slots()]
    assert names == ["2", "default", "hard mode"]
    assert all(s.keys == 1 and s.saved_at for s in save.slots())
    save.use("default")
    assert save["level"] == 1
    assert save.exists("2") and not save.exists("3")
    save.delete("2")
    assert [s.name for s in save.slots()] == ["default", "hard mode"]
    save.delete()  # the current slot: starts empty
    assert len(save) == 0 and not save.exists()
    assert save.path is not None and save.path.endswith(os.path.join("saves", "default.json"))


def test_bad_values_and_slot_names_explain_themselves(tmp_path):
    host, _, _ = _host(tmp_path)
    save = host.ctx.save
    with pytest.raises(TypeError, match="can't save object"):
        save["thing"] = object()
    with pytest.raises(TypeError, match="keys must be strings"):
        save["d"] = {1: "x"}
    with pytest.raises(ValueError, match="finite"):
        save["x"] = float("nan")
    with pytest.raises(ValueError, match="save slot"):
        save.use("../escape")
    save["coins"] = 3
    with pytest.raises(KeyError, match="did you mean 'coins'"):
        save["coin"]
    assert save.get("missing", 7) == 7


def test_a_broken_save_file_is_set_aside(tmp_path):
    folder = tmp_path / "saves" / "saves"
    folder.mkdir(parents=True)
    (folder / "default.json").write_text("{not json")
    host, _, _ = _host(tmp_path)
    assert len(host.ctx.save) == 0
    assert (folder / "default.json.broken").exists()
    assert host.backend.logs and host.backend.logs[0][0] == "warn"


def test_without_a_folder_saves_live_in_memory(tmp_path):
    host, _, recs = _host(tmp_path, save_dir=None)
    host.ctx.save["coins"] = 2
    host.ctx.settings.volume = 0.5
    _frame(host, recs)
    assert host.ctx.save["coins"] == 2 and host.ctx.save.path is None
    assert [s.name for s in host.ctx.save.slots()] == ["default"]
    assert not (tmp_path / "saves").exists()
    assert {p.name for p in tmp_path.iterdir()} <= {"s.py", "__pycache__"}


def test_write_failures_are_logged_once_not_raised(tmp_path):
    (tmp_path / "saves").write_text("a file where the folder should be")
    host, _, recs = _host(tmp_path)
    host.ctx.save["coins"] = 1
    _frame(host, recs, 3)
    warnings = [m for level, m in host.backend.logs if level == "warn"]
    assert len(warnings) == 1 and "couldn't save" in warnings[0]


def test_stop_writes_what_is_left(tmp_path):
    host, _, recs = _host(tmp_path, source=(
        "def stop(entity, ctx):\n"
        "    ctx.save['quit_cleanly'] = True\n"
    ))
    _frame(host, recs)
    host.stop()
    data = json.loads((tmp_path / "saves" / "saves" / "default.json").read_text())
    assert data["data"] == {"quit_cleanly": True}


# --- settings ---------------------------------------------------------------


def test_volume_and_graphics_apply_and_persist(tmp_path):
    host, _, recs = _host(tmp_path, gfx="medium")
    settings = host.ctx.settings
    assert settings.volume == 1.0
    assert settings.graphics == "medium"  # what the game runs at
    out = _frame(host, recs)
    assert out.settings is None  # nothing to apply
    settings.volume = 1.7  # clamped
    settings.graphics = "Low"
    out = _frame(host, recs)
    assert out.settings == {"volume": 1.0, "graphics": "low"}
    assert _frame(host, recs).settings is None  # applied once
    settings.volume = 0.25
    _frame(host, recs)
    data = json.loads((tmp_path / "saves" / "settings.json").read_text())
    assert data["volume"] == 0.25 and data["graphics"] == "low"
    # The next run applies the saved settings on its first frame.
    host2, _, recs2 = _host(tmp_path)
    out = _frame(host2, recs2)
    assert out.settings == {"volume": 0.25, "graphics": "low"}
    with pytest.raises(ValueError, match="did you mean 'ultra'"):
        host2.ctx.settings.graphics = "ultr"


def test_settings_reset_goes_back_to_the_start(tmp_path):
    host, _, recs = _host(tmp_path, gfx="ultra")
    settings = host.ctx.settings
    settings.graphics = "low"
    settings.volume = 0.1
    settings["fov"] = 90
    _frame(host, recs)
    settings.reset()
    out = _frame(host, recs)
    assert out.settings == {"volume": 1.0, "graphics": "ultra"}
    assert "fov" not in settings and settings.volume == 1.0
    assert json.loads((tmp_path / "saves" / "settings.json").read_text())["values"] == {}


def test_custom_settings_are_shared_by_every_slot(tmp_path):
    host, _, recs = _host(tmp_path)
    host.ctx.settings["subtitles"] = True
    host.ctx.save.use(3)
    _frame(host, recs)
    host2, _, _ = _host(tmp_path)
    assert host2.ctx.settings["subtitles"] is True
    assert host2.ctx.settings.get("missing", 1) == 1


def test_bound_actions_work_with_input(tmp_path):
    host, keys, recs = _host(tmp_path)
    settings, inp = host.ctx.settings, host.ctx.input
    settings.bind("jump", "space", "w")
    settings.bind("fire", "mouse_left")
    assert settings.keys("jump") == ("space", "w")
    keys.update({"w"})
    assert inp.pressed("jump") and inp.just_pressed("jump")
    assert not inp.pressed("fire")
    keys.update(set(), buttons={"left"})
    assert inp.pressed("fire") and inp.just_pressed("fire")
    assert inp.just_released("jump")
    # Plain key names still work, and an unbound name is just a key.
    keys.update({"space"})
    assert inp.pressed("space") and inp.pressed("jump")
    assert inp.axis("jump", "fire") == -1.0


def test_rebinding_wins_over_defaults_and_persists(tmp_path):
    host, keys, recs = _host(tmp_path)
    settings = host.ctx.settings
    settings.bind("jump", "space")
    settings.rebind("jump", "k")
    keys.update({"space"})
    assert not host.ctx.input.pressed("jump")
    keys.update({"k"})
    assert host.ctx.input.pressed("jump")
    _frame(host, recs)
    host2, _, _ = _host(tmp_path)
    host2.ctx.settings.bind("jump", "space")  # the game's start() does this again
    assert host2.ctx.settings.keys("jump") == ("k",)
    assert host2.ctx.settings.bindings() == {"jump": ("k",)}
    host2.ctx.settings.reset_keys("jump")
    assert host2.ctx.settings.keys("jump") == ("space",)
    with pytest.raises(ValueError, match="did you mean 'space'"):
        host2.ctx.settings.rebind("jump", "spce")


def test_any_just_pressed_finds_the_new_key(tmp_path):
    host, keys, _ = _host(tmp_path)
    inp = host.ctx.input
    keys.update(set())
    assert inp.any_just_pressed() is None
    keys.update({"k"})
    assert inp.any_just_pressed() == "k"
    keys.update({"k"})  # still held: not new
    assert inp.any_just_pressed() is None
    keys.update({"k"}, buttons={"right"})
    assert inp.any_just_pressed() == "mouse_right"
    assert set(api.KEY_NAMES) >= {"space", "arrow_up", "f12", "shift_left", "z", "0"}


def test_a_rebinding_screen_end_to_end(tmp_path):
    source = (
        "def start(entity, ctx):\n"
        "    ctx.settings.bind('jump', 'space')\n"
        "    entity.attributes['waiting'] = False\n"
        "def update(entity, ctx):\n"
        "    if entity.attributes['waiting']:\n"
        "        key = ctx.input.any_just_pressed()\n"
        "        if key and key != 'escape':\n"
        "            ctx.settings.rebind('jump', key)\n"
        "            entity.attributes['waiting'] = False\n"
        "        return\n"
        "    if ctx.input.just_pressed('f1'):\n"
        "        entity.attributes['waiting'] = True\n"
        "    if ctx.input.just_pressed('jump'):\n"
        "        ctx.save['jumps'] = ctx.save.get('jumps', 0) + 1\n"
    )
    host, keys, recs = _host(tmp_path, source=source)
    for held in [set(), {"space"}, set(), {"f1"}, set(), {"j"}, set(), {"space"}, set(), {"j"}]:
        keys.update(held)
        _frame(host, recs)
    assert host.ctx.settings.keys("jump") == ("j",)
    # Space jumped once before the rebind, J once after (the J that set
    # it doesn't jump: the screen returns while waiting).
    assert host.ctx.save["jumps"] == 2


def test_user_data_dir_follows_the_platform(monkeypatch, tmp_path):
    monkeypatch.delenv("AUTOMATA_SAVE_DIR", raising=False)
    monkeypatch.delenv("ANDROID_PRIVATE", raising=False)
    monkeypatch.setattr(api.sys, "platform", "linux")
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "xdg"))
    assert api.user_data_dir("Coin Run") == str(tmp_path / "xdg" / "Coin Run")
    monkeypatch.setenv("XDG_DATA_HOME", "relative/ignored")
    assert api.user_data_dir("a/b:c") == os.path.join(
        os.path.expanduser("~"), ".local", "share", "abc")
    monkeypatch.setattr(api.sys, "platform", "win32")
    monkeypatch.setenv("APPDATA", str(tmp_path / "roaming"))
    assert api.user_data_dir("Coin Run") == str(tmp_path / "roaming" / "Coin Run")
    monkeypatch.setattr(api.sys, "platform", "darwin")
    assert api.user_data_dir("..") == os.path.join(
        os.path.expanduser("~"), "Library", "Application Support", "AUTOMATA game")
    monkeypatch.setenv("AUTOMATA_SAVE_DIR", str(tmp_path / "override"))
    assert api.user_data_dir("Coin Run") == str(tmp_path / "override")
    assert api.dev_save_dir("/anything") == str(tmp_path / "override")
    monkeypatch.delenv("AUTOMATA_SAVE_DIR")
    assert api.dev_save_dir(str(tmp_path)) == str(tmp_path / ".oberon" / "save")


# --- the engine (run_scene) -------------------------------------------------


def _engine_scene(tmp_path, source):
    at = pytest.importorskip("automata")
    (tmp_path / "game.py").write_text(source)
    player = at.Entity("Player").add(at.Transform())
    player.script = "game.py"
    at.Scene().add(player).save(str(tmp_path / "level.ron"))
    return at, str(tmp_path / "level.ron")


def test_run_scene_saves_and_applies_settings(tmp_path):
    at, level = _engine_scene(tmp_path, (
        "def start(entity, ctx):\n"
        "    ctx.save['runs'] = ctx.save.get('runs', 0) + 1\n"
        "    if ctx.save['runs'] == 1:\n"
        "        ctx.settings.graphics = 'low'\n"
        "        ctx.settings.volume = 0.5\n"
        "def update(entity, ctx):\n"
        "    entity.attributes['gfx'] = ctx.raw.screen.graphics\n"
        "    entity.attributes['runs'] = ctx.save['runs']\n"
    ))
    saves = tmp_path / "data"
    seen = {}

    def after(ctx):
        seen.update(dict(ctx.scene["Player"].attributes))

    at.run_scene(level, headless=True, max_frames=4, update=after, save_dir=str(saves))
    assert seen == {"gfx": "low", "runs": 1}
    assert json.loads((saves / "settings.json").read_text())["graphics"] == "low"
    # The second run starts at the saved graphics preset.
    seen.clear()
    at.run_scene(level, headless=True, max_frames=4, update=after, save_dir=str(saves))
    assert seen == {"gfx": "low", "runs": 2}


def test_headless_run_scene_keeps_nothing_by_default(tmp_path):
    at, level = _engine_scene(tmp_path, (
        "def start(entity, ctx):\n"
        "    ctx.save['x'] = 1\n"
        "    entity.attributes['path'] = str(ctx.save.path)\n"
    ))
    seen = {}
    at.run_scene(level, headless=True, max_frames=2,
                 update=lambda ctx: seen.update(dict(ctx.scene["Player"].attributes)))
    assert seen["path"] == "None"
    assert not (tmp_path / ".oberon").exists()


def test_the_player_saves_into_the_user_data_folder(tmp_path, monkeypatch):
    at, level = _engine_scene(tmp_path, (
        "def start(entity, ctx):\n"
        "    ctx.save['coins'] = 12\n"
    ))
    (tmp_path / "game.json").write_text(json.dumps(
        {"name": "Save Test", "start_scene": "level.ron", "resolution": [320, 240]}))
    data = tmp_path / "xdg"
    monkeypatch.delenv("AUTOMATA_SAVE_DIR", raising=False)
    monkeypatch.setenv("XDG_DATA_HOME", str(data))
    monkeypatch.setenv("APPDATA", str(data))
    monkeypatch.setattr(api.sys, "platform", "linux")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(api.sys, "path", list(api.sys.path))
    from automata import player

    player.main([str(tmp_path), "--headless", "--frames", "2"])
    saved = json.loads((data / "Save Test" / "saves" / "default.json").read_text())
    assert saved["data"] == {"coins": 12}
