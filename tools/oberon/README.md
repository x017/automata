# Oberon

The visual editor for the [AUTOMATA](../../README.md) game engine.

Oberon is a desktop app built with [nexa-ui](https://pypi.org/project/nexa-ui/).
It edits AUTOMATA scene files (RON) through a document model, shows a live
software-rendered wireframe viewport, and can simulate the scene headless
with the engine. The design spec is [`docs/design/oberon.md`](../../docs/design/oberon.md).

Oberon — dark-white sci-fi editor.

## Features

- **Hierarchy** — the scene's entities; add, rename, select, delete.
- **Viewport** — a live perspective wireframe render of the scene (drag to
  orbit, scroll to zoom), with a reference grid and editor gizmos for lights
  and cameras.
- **Inspector** — edit the selected entity's components: transforms,
  meshes, lights, cameras, physics bodies, colliders.
- **Play** — simulate the scene headless through the engine (physics +
  scripts run), then restore the pre-play snapshot (Unity-style: play-mode
  changes are discarded).
- **Console** — an editor action log.
- File open/save, undo/redo, keyboard shortcuts, dark-white sci-fi theme.

## Layout of the code

The package separates the nexa-free logic (unit-testable headless) from the
UI:

| Module | Role | Needs a display? |
| ------ | ---- | ---------------- |
| `oberon/document.py` | Scene document model (entities, components, undo, save/load) | no |
| `oberon/component_info.py` | Per-component display/edit metadata | no |
| `oberon/viewport.py` | Software 3D wireframe renderer | no |
| `oberon/runner.py` | Play-mode engine host (snapshot ▸ simulate ▸ restore) | no |
| `oberon/app.py` | The nexa-ui application (panels, menus, viewport widget) | yes |

## Running

Oberon needs the engine extension built first (it is a local maturin wheel,
not on PyPI) plus nexa-ui:

```sh
# 1. build + install the engine (from the repo root)
maturin build --release --manifest-path crates/automata-python/Cargo.toml
pip install target/wheels/automata_engine-*.whl

# 2. install Oberon (pulls nexa-ui)
pip install -e tools/oberon

# 3. run (needs a display — it is a desktop window, not a headless tool)
oberon                                # or: python -m oberon
oberon path/to/scene.ron              # open a scene
python tools/oberon/oberon/app.py     # direct-script run also works
```

If you prefer not to install the package, run it as a module from the
`tools/oberon` directory (`python -m oberon`) or the script directly —
both resolve imports correctly.

## Tests

```sh
pytest tools/oberon/tests            # logic layer, headless
```

## Status and the engine hooks it grows into

Oberon's document and play model are complete against the shipped engine API.
The viewport currently renders its own wireframe; when the engine's
`game.step(dt)` / `game.render_frame((w, h))` hooks land (the M1/M2 items in
the design spec), the viewport will show the engine's real GPU frames and
Play will drive a live, pausable simulation instead of a run-and-report.
