# Guide: Input

Keyboard, mouse, and gamepads — polled raw for toys, bound to named actions
for games.

## The snapshot

`ctx.input` is the device state at the start of the frame:

```python
ctx.input.pressed(at.Key.SPACE)          # held right now
ctx.input.just_pressed(at.Key.SPACE)     # went down this frame
ctx.input.just_released(at.Key.SPACE)    # went up this frame

ctx.input.mouse_pressed(at.MouseButton.LEFT)
ctx.input.mouse_just_pressed(at.MouseButton.LEFT)
ctx.input.cursor                          # (x, y) pixels or None
ctx.input.scroll                          # (dx, dy) this frame
ctx.input.modifiers.shift                 # shift/ctrl/alt/super_key
```

Key names are **typed enums** — `at.Key.SPACE`, `at.Key.ARROW_LEFT`,
`at.Key.F1` — that your IDE autocompletes. Every enum member is a plain
`str`, so `"space"` works identically; prefer enums for typo safety.

> Headless mode receives no OS input events — everything reads as
> unpressed. Drive headless tests through physics and logic instead (see
> [Testing](testing.md)).

## Action maps: the layer you'll actually use

```python
controls = at.ActionMap()
controls.bind("jump", at.Binding.key(at.Key.SPACE))
controls.bind("jump", at.Binding.gamepad_button(at.GamepadButton.SOUTH))
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.ARROW_LEFT, at.Key.ARROW_RIGHT))
controls.bind_axis("move_x", at.AxisBinding.gamepad_axis(at.GamepadAxis.LEFT_STICK_X))

# inside update:
if controls.just_pressed("jump", ctx.input): ...
dx = controls.axis("move_x", ctx.input)            # -1.0 ..= 1.0
mx, my = controls.axis_pair("move_x", "move_y", ctx.input)
```

Rules that matter:

1. **Bindings stack.** Three `bind_axis("move_x", ...)` calls mean A/D,
   arrows, and the stick all work at once; values sum, clamped to −1..1.
   (This was once a replace-not-stack bug — if your game only jumps but
   never walks, update your checkout.)
2. **Query against `ctx.input`** — the map itself is stateless, so it works
   with the frame snapshot.
3. **Remapping is game code** — `rebind(action, [bindings])`,
   `unbind(action)`, `unbind_axis(name)` work at runtime; a settings screen
   is ordinary game code.

### Binding kinds

| Constructor | Triggers on |
|---|---|
| `Binding.key(key)` | keyboard key |
| `Binding.mouse(button)` | mouse button |
| `Binding.gamepad_button(button)` | any connected pad's button |
| `Binding.gamepad_axis_positive(axis)` / `..._negative(axis)` | analog axis past ±0.5 (triggers-as-buttons) |
| `AxisBinding.key_pair(neg, pos)` | two keys → −1/0/+1 |
| `AxisBinding.button_pair(neg, pos)` | two bindings → −1/0/+1 |
| `AxisBinding.gamepad_axis(axis, scale=1.0)` | analog value × scale (`-1.0` inverts) |

## Gamepads

```python
pads = ctx.input.gamepads
if pads.any_connected():
    pad = pads.primary()
    pad.pressed(at.GamepadButton.SOUTH)
    lx, ly = pad.left_stick()
for event in pads.events:   # connections, disconnections, button/axis edges
    ...
```

Buttons: `SOUTH/EAST/NORTH/WEST` (A/B/Y/X or Cross/Circle/Triangle/Square),
triggers, `LEFT_THUMB`/`RIGHT_THUMB` (stick clicks), d-pad. Axes:
`LEFT_STICK_X/Y`, `RIGHT_STICK_X/Y`, and more — all in `GamepadButton` /
`GamepadAxis`.

## See also

- [The Game Loop](game-loop.md) — where the snapshot comes from
- [Cookbook](../cookbook.md#input) — double-tap dash, input buffering
- [API Reference](../api-reference.md#input)
