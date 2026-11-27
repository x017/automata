# Pixel Peaks

A small 2D platformer built with AUTOMATA's 2D toolkit: a painted tilemap
with an auto-tiling ground, animated sprites, 2D physics and a follow camera.
Run and jump to the flag, collect the 11 coins on the way, and squash the
slimes by landing on them. If a slime touches you from the side, or you fall
into a pit, you go back to the start.

| Key | Action |
| --- | --- |
| A / D or ← / → | Run |
| Space, W or ↑ | Jump (hold it for a higher jump) |

The jump forgives a little. It still works for a moment after you run off a
ledge ("coyote time"), and a press just before landing counts ("jump buffer").

## Run it

```sh
# In the editor: open the scene and press Play (Ctrl+P)
cargo run -p oberon --release examples/oberon/platformer/level.ron

# Without the editor
python examples/oberon/platformer/main.py
python examples/oberon/platformer/main.py --headless   # 600 frames, no window
```

Oberon opens 2D scenes in its 2D view (toggle it with the **2D** button or the
`2` key). Select **Ground** to paint tiles with the brush in the Inspector's
palette.

## Export it

The folder has a `build.ron`, so it exports as is (you can also use Build → Build in Oberon):

```sh
automata-cli build examples/oberon/platformer --target all
# → build/linux/Pixel Peaks/Pixel Peaks
# → build/windows/Pixel Peaks/Pixel Peaks.exe
```

## What's inside

| Path | What |
| --- | --- |
| `level.ron` | The scene. **Sky** is a sprite backdrop. **Ground** is a solid tilemap; its `Ground` auto-tile picks edge pieces from the neighbouring cells. **Scenery** is a second tilemap with no collision. Also in the scene: the **Player** (sprite clips, a 2D box collider and a character controller), coins, slimes, the **Flag**, and a **Camera** (`Camera2D` plus `CameraFollow2D`, held inside the level). |
| `scripts/player.py` | Running with acceleration, a variable-height jump with coyote time and a jump buffer, picking the sprite clip (idle, run, jump, fall), coins, stomping slimes, respawning, the flag and the HUD. |
| `scripts/slime.py` | Slimes hop back and forth within their `range` attribute and face the way they go. |
| `scripts/sky.py` | The backdrop follows most of the camera's movement, for a parallax effect. |
| `sprites/` | The pixel art: the tileset, sprite sheets and the sky. |
| `build_art.py` | Draws everything in `sprites/` pixel by pixel, with no outside assets or dependencies. |
| `build_level.py` | Writes `level.ron` from an ASCII map. Edit `MAP` and run it again to change the layout. |
| `tests/` | Headless tests: `pytest examples/oberon/platformer/tests`. |

The walkthrough is in the wiki: [Tutorial: a 2D platformer](../../../docs/wiki/tutorials/platformer.md).
All art and code here is part of AUTOMATA and under its licence.
