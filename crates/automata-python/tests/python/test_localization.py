"""ctx.tr and ctx.settings.language: string tables (lang/*.csv) in the
shared script API (standard library only, like the rest of script_api)."""

import json

import pytest

from automata import script_api as api

TABLE = (
    "\ufeffkey,en,es,ja\n"
    "@name,English,Español,日本語\n"
    "@font,,,fonts/JP.ttf\n"
    "# menus\n"
    "menu.play,Play,Jugar,プレイ\n"
    '"menu.quit","Quit, now",Salir,終了\n'
    "hud.coins,Coins: {count},Monedas: {count},\n"
    'two.lines,"One\nTwo",Uno\\nDos,\n'
)


class _Backend(api.Backend):
    def __init__(self, base, system="en-US"):
        self.base = str(base)
        self.system = system
        self.logs = []

    def base_dir(self):
        return self.base

    def language(self):
        return self.system

    def log(self, level, message):
        self.logs.append((level, message))


def _project(tmp_path, table=TABLE, extra=None):
    (tmp_path / "lang").mkdir(exist_ok=True)
    (tmp_path / "lang" / "strings.csv").write_text(table, encoding="utf-8")
    for name, text in (extra or {}).items():
        (tmp_path / "lang" / name).write_text(text, encoding="utf-8")
    return tmp_path


def _host(tmp_path, system="en-US", save=True):
    backend = _Backend(tmp_path, system)
    host = api.ScriptHost(backend, api.KeySource(),
                          save_dir=tmp_path / "save" if save else None)
    return host, backend


def _warnings(backend):
    return [m for level, m in backend.logs if level == "warn"]


def test_tr_gives_text_in_the_language_and_fills_in_values(tmp_path):
    host, backend = _host(_project(tmp_path))
    tr = host.ctx.tr
    assert tr.languages == ("en", "es", "ja")
    assert tr.language == "en"
    assert tr("menu.play") == "Play"
    assert tr("menu.quit") == "Quit, now"
    assert tr("hud.coins", count=3) == "Coins: 3"
    assert tr("two.lines") == "One\nTwo"
    assert tr("menu.play", language="ja") == "プレイ"
    assert tr("two.lines", language="es") == "Uno\nDos"
    assert tr.name() == "English" and tr.name("es") == "Español" and tr.name("fr") == "fr"
    assert tr.font() is None and tr.font("ja") == "fonts/JP.ttf"
    assert tr.has("menu.play") and "menu.quit" in tr and not tr.has("nope")
    assert tr.keys() == ["hud.coins", "menu.play", "menu.quit", "two.lines"]
    assert tr.keys("menu.") == ["menu.play", "menu.quit"]
    assert _warnings(backend) == []


def test_gaps_and_mistakes_fall_back_and_warn_once(tmp_path):
    host, backend = _host(_project(tmp_path))
    host.ctx.settings.language = "ja"
    tr = host.ctx.tr
    # An empty cell: the first language's text.
    assert tr("hud.coins", count=2) == "Coins: 2"
    assert tr("hud.coins", count=5) == "Coins: 5"
    # A missing key: the key itself, with a suggestion.
    assert tr("menu.plya") == "menu.plya"
    tr("menu.plya")
    # A placeholder nobody filled stays as written.
    assert tr("hud.coins", amount=1) == "Coins: {count}"
    # A broken format spec: the text unfilled.
    host.ctx.settings.language = "en"
    assert tr("menu.play", x=1) == "Play"
    assert _warnings(backend) == [
        "lang/strings.csv:7: hud.coins has no ja text; using en",
        "ctx.tr: there's no key 'menu.plya' in lang/*.csv; did you mean 'menu.play'?",
        "ctx.tr('hud.coins'): the text has {count} but no count= was given",
    ]
    with pytest.raises(TypeError, match="takes a key"):
        tr(3)


def test_no_tables_gives_keys_back(tmp_path):
    host, backend = _host(tmp_path)
    assert host.ctx.tr.languages == ()
    assert host.ctx.tr("menu.play") == "menu.play"
    assert host.ctx.tr("hud.coins", count=1) == "hud.coins"
    assert host.ctx.settings.language == "en-US"
    host.ctx.settings.language = "de"
    assert host.ctx.settings.language == "de"
    assert _warnings(backend) == ["ctx.tr('menu.play'): there are no string tables yet "
                                  "(lang/*.csv)"]


def test_the_language_starts_as_the_systems_and_is_saved(tmp_path):
    _project(tmp_path)
    host, _ = _host(tmp_path, system="es-MX")
    assert host.ctx.settings.language == "es"
    assert host.ctx.tr("menu.play") == "Jugar"
    # A system language the tables lack: their first language.
    assert _host(tmp_path, system="fr-FR")[0].ctx.settings.language == "en"
    assert _host(tmp_path, system=None)[0].ctx.settings.language == "en"
    # The player's choice is saved with the other settings.
    host.ctx.settings.language = "JA"
    assert host.ctx.settings.language == "ja"
    host.ctx.settings._flush()
    data = json.loads((tmp_path / "save" / "settings.json").read_text())
    assert data["language"] == "ja"
    again, _ = _host(tmp_path, system="es-MX")
    assert again.ctx.settings.language == "ja"
    assert again.ctx.tr("menu.play") == "プレイ"
    assert "language='ja'" in repr(again.ctx.settings)
    again.ctx.settings.reset()
    assert again.ctx.settings.language == "es"


def test_unknown_languages_are_refused_with_the_list(tmp_path):
    host, _ = _host(_project(tmp_path))
    with pytest.raises(ValueError, match=r"no language 'fr' in lang/\*\.csv \(languages: en, es, ja\)"):
        host.ctx.settings.language = "fr"
    with pytest.raises(TypeError):
        host.ctx.settings.language = 3
    assert host.ctx.settings.language == "en"


def test_regional_codes_find_their_language(tmp_path):
    _project(tmp_path, "key,en,pt-BR,zh_Hans\nk,A,B,C\n")
    host, _ = _host(tmp_path, system="pt_PT.UTF-8")
    assert host.ctx.settings.language == "pt-BR"
    host.ctx.settings.language = "zh-hans"
    assert host.ctx.settings.language == "zh_Hans"
    assert host.ctx.tr("k") == "C"
    assert host.ctx.tr("k", language="en-GB") == "A"


def test_tables_merge_and_problems_are_reported_with_lines(tmp_path):
    _project(tmp_path, "key,en\nmenu.play,Play\n@colour,red\n,orphan\n", extra={
        "z_more.csv": "id,ja,en\nmenu.play,プレイ,Again\nmenu.quit,終了,Quit\nx,a,b,c\n",
        "notes.txt": "key,fr\nx,y\n",
    })
    host, backend = _host(tmp_path)
    tr = host.ctx.tr
    assert tr.languages == ("en", "ja")
    assert tr("menu.play") == "Play"
    assert tr("menu.quit", language="ja") == "終了"
    assert _warnings(backend) == [
        "lang/strings.csv:3: unknown row @colour (the special rows are @name and @font)",
        "lang/strings.csv:4: text without a key in the first column",
        "lang/z_more.csv:2: menu.play is already in lang/strings.csv:2; keeping that one",
        'lang/z_more.csv:4: more cells than languages (a comma inside text needs "quotes")',
    ]


def test_hud_text_carries_the_languages_font(tmp_path):
    host, _ = _host(_project(tmp_path))
    hud = host.ctx.hud
    hud.text("Play", 0, 0)
    assert "fallback" not in hud._items[-1]
    host.ctx.settings.language = "ja"
    hud.text("プレイ", 0, 0)
    hud.text("プレイ", 0, 0, font="fonts/Pixel.ttf")
    hud.text("プレイ", 0, 0, font="fonts/JP.ttf")
    assert hud._items[-3]["fallback"] == "fonts/JP.ttf"
    assert hud._items[-2]["fallback"] == "fonts/JP.ttf"
    assert hud._items[-2]["font"] == "fonts/Pixel.ttf"
    assert "fallback" not in hud._items[-1]


def test_edited_tables_reload(tmp_path, monkeypatch):
    host, backend = _host(_project(tmp_path))
    host.hot_reload = True
    clock = [100.0]
    monkeypatch.setattr(api.time, "monotonic", lambda: clock[0])
    assert host.ctx.tr("menu.play") == "Play"
    (tmp_path / "lang" / "strings.csv").write_text("key,en\nmenu.play,Start game\n")
    assert host.ctx.tr("menu.play") == "Play", "checked at most twice a second"
    clock[0] += 1.0
    assert host.ctx.tr("menu.play") == "Start game"
    assert ("info", "Reloaded the string tables (lang/*.csv)") in backend.logs


UI_DOC = """(
    design_size: (1280.0, 720.0),
    hud: true,
    nodes: [
        (
            kind: Column,
            anchor: "center",
            title: "menu.title",
            panel: true,
            children: [
                (kind: Label, name: "heading", text: "menu.play"),
                (kind: Button, name: "quit", text: "menu.quit"),
                (kind: Label, name: "plain", text: "Not a key"),
            ],
        ),
    ],
)
"""


def test_document_text_that_is_a_key_is_translated(tmp_path):
    _project(tmp_path, TABLE + "menu.title,Menu,Menú,メニュー\n")
    (tmp_path / "ui").mkdir()
    (tmp_path / "ui" / "menu.ui.ron").write_text(UI_DOC, encoding="utf-8")
    host, backend = _host(tmp_path)
    host.ctx.settings.language = "ja"
    out = host.frame([], delta=1 / 60, elapsed=0.0, frame=1, screen=(1280, 720))
    texts = [i["text"] for i in out.hud if i["k"] == "text"]
    assert "メニュー" in texts and "プレイ" in texts and "終了" in texts
    assert "Not a key" in texts
    assert not any(t.startswith("menu.") for t in texts)
    assert all(i.get("fallback") == "fonts/JP.ttf" for i in out.hud if i["k"] == "text")
    # The script still reads the document's own value.
    assert host.ctx.ui.screen("menu").get("heading") == "menu.play"
    assert _warnings(backend) == []


def test_full_width_text_is_estimated_wider():
    assert api._text_ems("Play") == pytest.approx(2.2)
    assert api._text_ems("プレイ") == pytest.approx(3.0)
    assert api._text_ems("コイン: 3") == pytest.approx(4.65)
    assert api._text_ems("Привет") == pytest.approx(3.3)


def test_the_system_language_comes_from_the_environment(monkeypatch):
    for var in ("LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"):
        monkeypatch.delenv(var, raising=False)
    monkeypatch.setenv("LANG", "de_AT.UTF-8")
    assert api._system_language() == "de-AT"
    monkeypatch.setenv("LANGUAGE", "fr_CA:fr")
    assert api._system_language() == "fr-CA"
    monkeypatch.setenv("LANGUAGE", "")
    monkeypatch.setenv("LC_ALL", "C")
    assert api._system_language() == "de-AT"
