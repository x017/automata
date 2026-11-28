"""The APK's entry point for an exported game.

The game comes in one of two ways:

- `automata-cli build --target android` puts the game's files in the APK
  itself (`assets/automata/game/...`, from the template APK). They're
  unpacked into the app's storage the first time the game starts, and again
  whenever an update brings different files (`assets/automata/stamp` holds
  a hash of them).
- `build_apk.sh` with a staged game folder puts it next to this file as
  `game/` (python-for-android unpacks it with the app's other files).

Either way this runs it the way the desktop launchers do, with
`automata.player`. Saves and settings go to the app's private storage.
"""

import os
import shutil
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
GAME_PREFIX = "assets/automata/game/"
STAMP = "assets/automata/stamp"


def apk_path():
    """The installed APK: from the activity, else from this process's
    memory map (Android maps the APK to read its assets)."""
    path = os.environ.get("AUTOMATA_APK")
    if path and os.path.isfile(path):
        return path
    try:
        with open("/proc/self/maps", encoding="utf-8", errors="replace") as maps:
            for line in maps:
                parts = line.split()
                if parts and parts[-1].endswith(".apk") and os.path.isfile(parts[-1]):
                    return parts[-1]
    except OSError:
        pass
    return None


def unpack_game(apk, dest):
    """Unpacks the APK's game into `dest` unless it's there already.
    Returns `dest`, or None when the APK holds no game."""
    with zipfile.ZipFile(apk) as z:
        try:
            stamp = z.read(STAMP).decode().strip()
        except KeyError:
            return None
        stamp_file = os.path.join(dest, ".apk-stamp")
        try:
            with open(stamp_file, encoding="utf-8") as f:
                if f.read().strip() == stamp:
                    return dest
        except OSError:
            pass
        partial = dest + ".unpacking"
        shutil.rmtree(partial, ignore_errors=True)
        root = os.path.realpath(partial)
        for info in z.infolist():
            if not info.filename.startswith(GAME_PREFIX) or info.is_dir():
                continue
            target = os.path.realpath(os.path.join(partial, info.filename[len(GAME_PREFIX):]))
            if not target.startswith(root + os.sep):
                continue  # never write outside the game folder
            os.makedirs(os.path.dirname(target), exist_ok=True)
            with z.open(info) as src, open(target, "wb") as out:
                shutil.copyfileobj(src, out)
        with open(os.path.join(partial, ".apk-stamp"), "w", encoding="utf-8") as f:
            f.write(stamp)
        shutil.rmtree(dest, ignore_errors=True)
        os.replace(partial, dest)
        print(f"automata: unpacked the game ({stamp[:12]})")
    return dest


def find_game():
    """The game folder to run."""
    bundled = os.path.join(HERE, "game")
    if os.path.isfile(os.path.join(bundled, "game.json")) and not os.path.isfile(
        os.path.join(bundled, ".apk-stamp")
    ):
        return bundled
    apk = apk_path()
    game = unpack_game(apk, bundled) if apk else None
    if game is None:
        raise SystemExit(
            "automata: this APK holds no game (it's the template: build a game "
            "with `automata-cli build --target android`)"
        )
    return game


if __name__ == "__main__":
    from automata.player import main

    main([find_game()])
