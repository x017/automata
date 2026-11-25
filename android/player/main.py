"""The APK's entry point for an exported game.

`build_apk.sh` puts the game folder that `automata-cli stage` writes
(scenes, scripts, assets and `game.json`) next to this file as `game/`, and
this runs it the way the desktop launchers do, with `automata.player`.
Saves and settings go to the app's private storage.
"""

import os

from automata.player import main

main([os.path.join(os.path.dirname(os.path.abspath(__file__)), "game")])
