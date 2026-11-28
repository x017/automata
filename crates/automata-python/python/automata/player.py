"""Runs an exported game: ``python -m automata.player <game folder> [options]``.

The exported game's launcher calls this with the bundled runtime. The game
folder holds ``game.json`` (written by ``automata-cli``) and the
project's scenes, scripts, and assets.

``game.json`` either names a ``main`` script (a code-first game such as
``oop_platformer.py``) or a ``start_scene`` (an Oberon project, run with its
behaviour scripts through ``automata.run_scene``).

``touch_controls`` (from ``build.ron``) sets up the on-screen thumbsticks
and buttons of a scene game (``ctx.input.touch_controls``).

``steam`` (``{"app_id": 480, "restart": false}``, from ``build.ron``)
connects to Steam before the window opens, so the overlay can draw over
it (see ``automata.steam``). With ``restart`` a copy started outside
Steam asks Steam to start it and ends.

Saves and settings (``ctx.save``, ``ctx.settings``) go to a folder named
after the game in the player's user-data folder (see
``automata.script_api.user_data_dir``; ``$AUTOMATA_SAVE_DIR`` overrides it).

Options: ``--headless`` (no window), ``--frames N`` (stop after N frames)
and ``--fixed-delta SECONDS`` (headless: every frame advances by exactly
that much; benchmarks use it). Anything else is passed on to a ``main``
script in ``sys.argv``.
"""

import json
import os
import runpy
import sys
from typing import Any, Dict, List, Optional


def _options(argv: List[str]) -> Dict[str, Any]:
    opts: Dict[str, Any] = {"headless": False, "frames": None, "fixed_delta": None, "rest": []}
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--headless":
            opts["headless"] = True
        elif arg == "--frames" and i + 1 < len(argv):
            opts["frames"] = int(argv[i + 1])
            i += 1
        elif arg.startswith("--frames="):
            opts["frames"] = int(arg.split("=", 1)[1])
        elif arg == "--fixed-delta" and i + 1 < len(argv):
            opts["fixed_delta"] = float(argv[i + 1])
            i += 1
        elif arg.startswith("--fixed-delta="):
            opts["fixed_delta"] = float(arg.split("=", 1)[1])
        else:
            opts["rest"].append(arg)
        i += 1
    return opts


def main(argv: Optional[List[str]] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv:
        print("usage: python -m automata.player <game folder> [--headless] [--frames N]", file=sys.stderr)
        return 2
    folder = argv.pop(0)
    # Windows verbatim paths (\\?\C:\...) break "/"-joined relative paths.
    if folder.startswith("\\\\?\\") and folder[5:6] == ":":
        folder = folder[4:]
    folder = os.path.abspath(folder)
    with open(os.path.join(folder, "game.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    opts = _options(argv)
    os.chdir(folder)
    if folder not in sys.path:
        sys.path.insert(0, folder)
    if _start_steam(manifest.get("steam"), folder) == "restarting":
        return 0

    main_script = manifest.get("main")
    if main_script:
        path = os.path.join(folder, main_script)
        sys.argv = [path] + (["--headless"] if opts["headless"] else []) + opts["rest"]
        if opts["frames"] is not None:
            _limit_frames(opts["frames"])
        runpy.run_path(path, run_name="__main__")
        return 0

    import automata as at

    scene = os.path.join(folder, manifest["start_scene"])
    game_options: Dict[str, Any] = {"title": manifest.get("name", "AUTOMATA game")}
    resolution = manifest.get("resolution")
    if resolution:
        game_options["resolution"] = (int(resolution[0]), int(resolution[1]))
    if opts["headless"]:
        game_options["headless"] = True
        if opts["fixed_delta"] is not None:
            game_options["fixed_delta"] = opts["fixed_delta"]
    from .script_api import user_data_dir

    save_dir = user_data_dir(manifest.get("name") or os.path.basename(folder))
    frames = at.run_scene(scene, max_frames=opts["frames"], save_dir=save_dir,
                          touch_controls=manifest.get("touch_controls"), **game_options)
    if opts["headless"]:
        print(f"ran {frames} frames of {manifest['start_scene']}")
    return 0


def _start_steam(config: Any, folder: str) -> Optional[str]:
    """Connects to Steam when ``game.json`` asks for it (before any window
    opens). Returns ``"restarting"`` when Steam will start the game instead."""
    if not isinstance(config, dict) or not config.get("app_id"):
        return None
    from . import steam

    client = steam.init(int(config["app_id"]), folders=[folder, os.path.dirname(folder)],
                        restart=bool(config.get("restart")))
    if client.status == "restarting through Steam":
        print("steam: starting the game through Steam")
        return "restarting"
    if not client.available:
        print(f"steam: {client.status}; playing without Steam")
    return None


def _limit_frames(frames: int) -> None:
    """Caps every `Game.run` in a code-first game at `frames` frames."""
    import automata as at

    original = at.Game.run

    def run(self: Any, update: Any = None, max_frames: Optional[int] = None) -> int:
        cap = frames if max_frames is None else min(max_frames, frames)
        return original(self, update, max_frames=cap)

    at.Game.run = run  # type: ignore[method-assign]


if __name__ == "__main__":
    sys.exit(main())
