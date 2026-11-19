"""The Oberon play-mode runner.

Runs the document through a headless ``automata.Game``. The shipped engine
API (M0) has no incremental ``game.step`` — ``Game.run()`` is a one-shot
loop that shuts down when it returns — so "Play" works like this:

1. snapshot the document to RON,
2. spawn a headless game, load the scene, attach the play-script's
   ``update`` callback, and run the simulation for ``seconds`` of game time,
3. when it finishes, restore the snapshot (Unity-style: play-mode changes
   are discarded) and surface a summary (frames run, final positions).

This is real play mode — physics and scripts genuinely execute — presented
as a run-and-report until the engine-side ``game.step``/``render_frame``
hooks (docs/design/oberon.md M1) let the editor drive the loop itself.

Nexa-free, so it is unit-testable headless.
"""

from __future__ import annotations

import enum
from typing import Callable, List, Optional, Tuple


class PlayState(enum.Enum):
    EDITING = "editing"
    RUNNING = "running"
    DONE = "done"


class PlayModeRunner:
    """Owns the headless play session and the snapshot/restore lifecycle."""

    def __init__(self) -> None:
        self.state = PlayState.EDITING
        self._snapshot_ron: Optional[str] = None
        self._update_fn: Optional[Callable] = None
        #: Frames to simulate per Play, from ``seconds`` at 60 fps.
        self.seconds = 4.0
        self.last_result: Optional[str] = None

    @property
    def playing(self) -> bool:
        return self.state is PlayState.RUNNING

    def set_update(self, fn: Optional[Callable]) -> None:
        """Installs the play-script's ``update(ctx)`` callback."""
        self._update_fn = fn

    def play(self, document, seconds: Optional[float] = None) -> None:
        """Snapshots the document and simulates it headless.

        Blocks for the simulation (a few seconds of game time, fast when
        headless), then restores the document and records a summary. Safe to
        call again — each Play is a fresh snapshot.
        """
        import automata as at  # deferred: keeps the module importable sans engine

        secs = self.seconds if seconds is None else seconds
        max_frames = max(1, int(secs * 60))

        snapshot = document.to_ron()
        names_before = [e.name for e in document.entities]

        game = at.Game(title="Oberon play", headless=True, target_fps=60)
        game.load_scene(document.to_scene())
        game.update(self._update_fn or (lambda ctx: None))
        self.state = PlayState.RUNNING

        frames = 0
        error: Optional[Exception] = None
        try:
            frames = game.run(max_frames=max_frames)
        except Exception as exc:  # surface script errors to the editor log
            error = exc

        # Restore the snapshot (play-mode changes are discarded).
        restored = type(document).from_ron(snapshot)
        document._entities = restored._entities  # noqa: SLF001
        document._dirty = True  # noqa: SLF001

        self.state = PlayState.DONE
        if error is not None:
            self.last_result = f"play error after {frames} frames: {error}"
        else:
            self.last_result = (
                f"played {len(names_before)} entities for {frames} frames "
                f"({secs:.1f}s game time); changes discarded on stop"
            )

    def stop(self) -> None:
        """Returns to the editing state (restores happen automatically)."""
        self.state = PlayState.EDITING
