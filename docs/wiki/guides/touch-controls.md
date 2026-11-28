# Guide: Touch controls

A phone has no keyboard. Touch controls put a game's keys on the screen as
thumbsticks and buttons, so a game written for the keyboard plays on a
touch screen without touch code. Scripts keep reading
`ctx.input.pressed("jump")` and `ctx.input.axis("left", "right")`, and the
controls hold those keys.

They show on touch screens only ([Android builds](../oberon/exporting.md#android)). On a
computer the keyboard and mouse play as usual.

## Set them up

In an exported game, put them in `build.ron`. **Build Settings → Touch
controls** writes a starting set (a stick holding WASD and a Jump button
holding Space) to edit:

```ron
touch_controls: Some((
    sticks: [
        (side: Left, keys: ["forward", "left", "back", "right"]),
        (side: Right, keys: ["q", "e"]),
    ],
    buttons: [(key: "jump", label: "Jump")],
)),
```

Or set them up from a script, which also works for games that pick their
controls at runtime:

```python
def start(entity, ctx):
    ctx.input.touch_controls.set(
        sticks=[{"side": "left", "keys": ["forward", "left", "back", "right"]}],
        buttons=[{"key": "jump", "label": "Jump"}],
    )
```

*Coin Run* does it this way, in `scripts/rules.py`.

## Sticks

A stick holds four keys (up, left, down, right) or two (left, right) as
it's pushed. There's one per half of the screen at most. It appears where
the thumb lands in the lower part of its half, so players don't have to
find it.

- `ctx.input.pressed` sees a key once the stick is pushed a little under
  half way toward it. Diagonals hold two keys.
- `ctx.input.axis` reads how far it's pushed, from −1 to 1, so a character
  can walk slowly and speed up smoothly. A keyboard still gives −1, 0 or 1.
- `ctx.input.touch_controls.stick("left")` is the raw push: `(x, y)` from
  −1 to 1, with +y up (away from the player).

## Buttons

Up to six buttons sit around the bottom-right corner, the first one (the
largest) in the corner. Each holds its key while a finger is on it. The
`label` is the text on it, or the key name when there's no label. A tap
shorter than a frame still counts as a press.

## Keys and actions

Keys can be key names (`"space"`, `"q"`) or actions bound with
[`ctx.settings.bind`](../oberon/scripting.md#player-settings) (`"jump"`).
An action also holds the keys bound to it. So a Jump button holding
`"jump"` makes `ctx.input.just_pressed("space")` true too, as if the player
had pressed Space. When the player rebinds Jump to K, the button follows.

## Mouse look

`look` makes dragging on the free part of one side move
`ctx.input.mouse_delta`, so a first-person camera turns with a finger as it
does with the mouse:

```ron
touch_controls: Some((
    sticks: [(side: Left, keys: ["w", "a", "s", "d"])],
    buttons: [(key: "mouse_left", label: "Fire"), (key: "r", label: "Reload")],
    look: Some(Right),
)),
```

A side can have a stick or `look`, not both. Buttons win where they sit.

## When they show

`show` decides when they're on the screen:

| `show` | Shown |
|---|---|
| `Auto` (the default) | On touch screens (Android) |
| `Always` | Everywhere. On a computer the mouse works them as a finger |
| `Never` | Only when a script sets `ctx.input.touch_controls.visible = True` |

A script can turn them on or off with
`ctx.input.touch_controls.visible`, for example from a settings screen. To
try them with the mouse without changing the game, start the game or Oberon
with the environment variable `AUTOMATA_TOUCH_CONTROLS=1` (`0` hides them).

Fingers on a control belong to it. They aren't in `ctx.input.touches`, and
the first one doesn't press the left mouse button. A finger that lands on
[`ctx.ui`](../oberon/scripting.md#game-ui) belongs to the UI, as before.

## Drawing your own

The controls are drawn with two built-in HUD images that any script can
use: `"builtin:circle"` (a filled disc) and `"builtin:ring"` (its outline),
both white, to tint with `color`:

```python
ctx.hud.image("builtin:circle", x, y, size=(80, 80), color=(1, 1, 1, 0.3))
```

For controls of your own, read `ctx.input.touches`
([Input](input.md#touch)) and leave `touch_controls` unset.

## See also

- [Input](input.md): keys, actions, touches and gamepads
- [Exporting your game](../oberon/exporting.md)
