# Shape Signal

![Shape Signal playing: SCORE 0, the time bar and a red lives bar, the lifted, glowing Tower between the Cube and the Slab, and the 1 Cube, 2 Tower and 3 Slab buttons](../../../docs/assets/ui/shape_signal_hud.jpg)

A small game whose screens were made in Oberon's [UI editor](../../../docs/wiki/oberon/ui-editor.md).
One of three shapes lifts. Pick it with the HUD's buttons (or **1**, **2**,
**3**) before it drops. Quick picks and streaks score more. A wrong or slow
pick costs one of three lives. A round lasts 45 seconds.

```console
cargo run -p oberon --release examples/oberon/shape_signal/level.ron
```

Open the files in `ui/` in Oberon to see and change the screens:

| Document | Screen | Its widgets |
|---|---|---|
| `ui/title.ui.ron` | **title**: modal, pauses the game | `play`, a `volume` slider and its `volume_text`, a `hints` toggle, the `language` button, the `best` score |
| `ui/hud.ui.ron` | **hud**: a HUD, under every screen | `score`, `streak`, the `time` bar and its `clock`, the `lives` bar (`"{value} / {max} lives"`), the `picks` row of `pick_1`…`pick_3`, `verdict`, the `keys` hint |
| `ui/results.ui.ron` | **results**: modal, pauses the game | `heading`, `final`, `detail`, `best`, `again`, `title` |

Each document has its own theme: amber and navy menus with a pixel-font
title, and a teal HUD. They're laid out for 1280×720 and scale to any
window.

`scripts/game.py` doesn't draw anything. It runs the rules and talks to
the documents by name:

```python
hud = ctx.ui.screen("hud")
hud.set("score", ctx.tr("hud.score", score=s["score"]))   # a label
hud.set("lives", s["lives"])                 # a progress bar
if hud.clicked("pick_1") or ctx.input.just_pressed("1"): ...
title = ctx.ui.screen("title")
if title.changed("volume"):
    ctx.settings.volume = title.get("volume")
ctx.ui.screens.show("results")               # modal and pausing, as the document says
```

So you can move, restyle or reword anything in the editor, and the game
keeps working as long as the names stay. `tests/test_shape_signal.py`
checks the names the script needs are there. The volume, the hint toggle
and the best score are kept in `ctx.settings` between runs.
`scripts/shape.py` lifts the shape the Game entity's `lit` attribute
names, and holds still while a pausing screen is up.

## Languages

Every word is in `lang/strings.csv`, in English, Spanish and Japanese. The
game starts in the computer's language when it has it, and the title's
**Language** button switches it (`ctx.settings` remembers the choice). The
documents' fixed text is a key of the table, like `title.play` or
`hud.keys`, so it shows in the player's language by itself. In the UI
editor, the **Text** menu previews each language. The script fills in the
rest with `ctx.tr`:

```python
results.set("detail", ctx.tr("results.detail", right=s["right"], missed=s["missed"]))
```

Japanese draws with `fonts/MPLUSRounded1c-ShapeSignal.ttf`, the table's
`@font` for `ja`. It fills in the characters Press Start 2P and the
built-in font lack. It's a 113 KB subset of Rounded M+ 1c with kana,
punctuation and the table's kanji. See
[Languages](../../../docs/wiki/oberon/scripting.md#languages).

![Shape Signal's title in Japanese](../../../docs/assets/ui/shape_signal_ja.jpg)

![The title screen: SHAPE SIGNAL in amber pixel letters, Play, a volume slider, a key hints toggle and the best score, over the dark scene](../../../docs/assets/ui/shape_signal_title.jpg)

```console
python examples/oberon/shape_signal/build_level.py   # rewrite level.ron
pytest examples/oberon/shape_signal/tests
```

The pixel font is Press Start 2P by CodeMan38, under the SIL Open Font
License (`fonts/PressStart2P-OFL.txt`). The Japanese font is a subset of
Rounded M+ 1c by Coji Morishita and the M+ Fonts Project, under the same
licence (`fonts/MPLUSRounded1c-OFL.txt`).
