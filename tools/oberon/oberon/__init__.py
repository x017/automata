"""Oberon — the visual editor for the AUTOMATA game engine.

Oberon is a desktop app built on nexa-ui. It edits AUTOMATA scene files
(RON) through a document model, shows a live software-rendered viewport,
and can play/pause/stop the scene with the engine's headless stepper.

The package is split so the logic is testable headless (no window):

- :mod:`oberon.document` — scene document model (pure Python, no nexa).
- :mod:`oberon.component_info` — per-component-type metadata (pure Python).
- :mod:`oberon.viewport` — software 3D wireframe renderer (pure Python).
- :mod:`oberon.runner` — play-mode engine host (pure Python, uses automata).
- :mod:`oberon.app` — the nexa-ui application (needs a display/GPU).
"""

__version__ = "0.1.0"
