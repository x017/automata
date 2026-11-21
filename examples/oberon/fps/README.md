# Blaster Arena

A small first-person shooter built in Oberon with Kenney's CC0 models and
sounds. Hop between floating grass platforms and shoot down four drones before
they wear you down.

| Key | Action |
| --- | --- |
| Mouse | Look |
| W A S D | Move |
| Space | Jump (press again in the air for a double jump) |
| Left mouse button | Shoot |
| E | Switch between the Blaster and the Blaster Repeater |
| Esc | Free the mouse (click to capture it again) |

## Run it

```sh
# In the editor: open the scene and press Play (Ctrl+P)
cargo run -p oberon --release examples/oberon/fps/level.ron

# Without the editor
python examples/oberon/fps/main.py
python examples/oberon/fps/main.py --headless   # 600 frames, no window
```

## Export it

The folder has a `build.ron`, so it exports as is (also from Build → Build in Oberon):

```sh
automata-export build examples/oberon/fps --target all
# → build/linux/Blaster Arena/Blaster Arena
# → build/windows/Blaster Arena/Blaster Arena.exe
```

## What's inside

| Path | What |
| --- | --- |
| `level.ron` | The scene: platforms, walls, grass, clouds, drones, the player and both weapons. |
| `scripts/player.py` | Mouse look, movement, double jump, shooting, weapon switching, health and the HUD. |
| `scripts/enemy.py` | Drones turn to face you, bob in the air and shoot when they can see you. |
| `scripts/cloud.py` | Clouds drift up and down. |
| `models/` | Kenney GLB models. The `.glb.import.ron` files hold import settings (Inspector → Import). |
| `sounds/`, `sprites/` | Sound effects and the crosshair and hit-marker images. |
| `build_level.py` | Rebuilds `level.ron` from Python. Run it after changing the layout in code. |
| `tests/` | Headless tests: `pytest examples/oberon/fps/tests`. |

The walkthrough is in the wiki: [Tutorial: a first-person shooter](../../../docs/wiki/tutorials/fps.md).
Asset licences are in [CREDITS.md](CREDITS.md).
