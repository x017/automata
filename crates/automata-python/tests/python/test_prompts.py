"""ctx.input.device, controller and prompt: button labels for the device
the player uses (keyboard, gamepad families, the Steam Deck)."""

import pytest

from automata import script_api as api


class _Backend(api.Backend):
    def log(self, level, message):
        pass

    def graphics(self):
        return "high"


def _host(tmp_path, source):
    script = tmp_path / "s.py"
    script.write_text(source)
    keys = api.KeySource()
    host = api.ScriptHost(_Backend(), keys)
    rec = {"id": 1, "name": "Player", "script": str(script), "p": [0, 0, 0], "r": [0, 0, 0],
           "s": [1, 1, 1], "attrs": {}}
    return host, keys, rec


SCRIPT = """
seen = []
def start(entity, ctx):
    ctx.settings.bind("jump", "space", "pad_south")
    ctx.settings.bind("pause", "escape", "pad_start")
    ctx.settings.bind("fire", "mouse_left", "pad_rt")
    ctx.settings.bind("dash", "pad_west")
def update(entity, ctx):
    seen.append((ctx.input.device, ctx.input.controller, ctx.input.prompt("jump"),
                 ctx.input.prompt("pause"), ctx.input.prompt("fire"), ctx.input.prompt("dash")))
"""


def _frame(host, keys, rec, frame, pressed=(), pads=None, buttons=(), delta=(0.0, 0.0)):
    keys.update(pressed, buttons, delta, False, None, pads)
    host.frame([rec], delta=0.016, elapsed=frame * 0.016, frame=frame)


def _seen(host):
    return next(iter(host.scripts.values())).module.seen


def test_prompts_follow_the_device_and_the_controller(tmp_path, monkeypatch):
    monkeypatch.delenv("SteamDeck", raising=False)
    monkeypatch.delenv("AUTOMATA_TOUCH_CONTROLS", raising=False)
    host, keys, rec = _host(tmp_path, SCRIPT)
    _frame(host, keys, rec, 1)
    assert _seen(host)[-1] == ("keyboard", None, "Space", "Esc", "Left click", "X")
    # A pad connects: prompts switch to it before it's touched.
    pad = ["Xbox Wireless Controller"]
    _frame(host, keys, rec, 2, pads=pad)
    _frame(host, keys, rec, 3, pads=pad)
    assert _seen(host)[-1] == ("gamepad", "xbox", "A", "☰", "RT", "X")
    # A key goes down: back to the keyboard, until a pad button.
    _frame(host, keys, rec, 4, ["w"], pads=pad)
    _frame(host, keys, rec, 5, [], pads=pad)
    assert _seen(host)[-1][0] == "keyboard"
    _frame(host, keys, rec, 6, ["pad_east"], pads=pad)
    _frame(host, keys, rec, 7, [], pads=pad)
    assert _seen(host)[-1][0] == "gamepad"
    # The mouse moving counts as the keyboard.
    _frame(host, keys, rec, 8, [], pads=pad, delta=(14.0, 3.0))
    _frame(host, keys, rec, 9, [], pads=pad)
    assert _seen(host)[-1][0] == "keyboard"
    _frame(host, keys, rec, 10, ["pad_south"], pads=["Sony DualSense Wireless Controller"])
    _frame(host, keys, rec, 11, [], pads=["Sony DualSense Wireless Controller"])
    assert _seen(host)[-1] == ("gamepad", "playstation", "×", "Options", "R2", "□")
    _frame(host, keys, rec, 12, [], pads=["Nintendo Switch Pro Controller"])
    assert _seen(host)[-1] == ("gamepad", "switch", "B", "+", "ZR", "Y")


def test_on_a_steam_deck(tmp_path, monkeypatch):
    monkeypatch.setenv("SteamDeck", "1")
    host, keys, rec = _host(tmp_path, SCRIPT)
    _frame(host, keys, rec, 1)
    # No pad named yet: the Deck's own controls.
    assert _seen(host)[-1] == ("gamepad", "steam_deck", "A", "☰", "R2", "X")
    _frame(host, keys, rec, 2, pads=["Steam Virtual Gamepad"])
    assert _seen(host)[-1][1] == "steam_deck"
    # A PlayStation pad plugged into the Deck keeps its own labels.
    _frame(host, keys, rec, 3, pads=["PS5 Controller"])
    assert _seen(host)[-1][1:3] == ("playstation", "×")


def test_the_players_rebinding_and_explicit_devices(tmp_path, monkeypatch):
    monkeypatch.delenv("SteamDeck", raising=False)
    host, keys, rec = _host(tmp_path, SCRIPT)
    _frame(host, keys, rec, 1)
    inp, settings = host.ctx.input, host.ctx.settings
    settings.rebind("jump", "w", "pad_north")
    assert inp.prompt("jump") == "W"
    assert inp.prompt("jump", device="gamepad") == "Y"
    settings.rebind("jump")
    assert inp.prompt("jump") == ""
    assert inp.prompt("arrow_up") == "↑" and inp.prompt("f5") == "F5"
    assert inp.prompt("pad_lb", device="gamepad") == "LB"


@pytest.mark.parametrize("name, deck, family", [
    ("Xbox 360 Controller (XInput STANDARD GAMEPAD)", False, "xbox"),
    ("054c-0ce6-Wireless Controller", False, "playstation"),
    ("Wireless Controller", False, "playstation"),
    # Browsers: Chrome's and Firefox's Gamepad.id for the same pads.
    ("DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)", False,
     "playstation"),
    ("Xbox Wireless Controller (STANDARD GAMEPAD Vendor: 045e Product: 0b13)", False, "xbox"),
    ("045e-0b13-Xbox Wireless Controller", False, "xbox"),
    ("Pro Controller (STANDARD GAMEPAD Vendor: 057e Product: 2009)", False, "switch"),
    ("057e-2009-Pro Controller", False, "switch"),
    ("Steam Deck", False, "steam_deck"),
    ("28de-1205-Steam Deck Controller", False, "steam_deck"),
    ("Steam Virtual Gamepad", False, "xbox"),
    ("Microsoft X-Box 360 pad", True, "steam_deck"),
    ("Logitech Dual Action", False, "generic"),
    (None, False, None),
    (None, True, "steam_deck"),
])
def test_controller_families(name, deck, family):
    assert api.controller_family(name, deck) == family


def test_labels():
    assert api.key_label("space") == "Space" and api.key_label("q") == "Q"
    assert api.key_label("numpad_7") == "Num 7" and api.key_label("page_down") == "Page Down"
    assert api.key_label("insert") == "Insert" and api.key_label("backslash") == "\\"
    assert api.pad_label("pad_south", "playstation") == "×"
    assert api.pad_label("pad_select", "steam_deck") == "⧉"
    assert api.pad_label("pad_up", None) == "↑"
    # Every family labels every pad key.
    for family, labels in api.PAD_LABELS.items():
        assert set(api.PAD_KEYS) <= set(labels), family
    assert set(api.PAD_LABELS) == set(api.CONTROLLERS)
