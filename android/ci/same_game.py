#!/usr/bin/env python3
"""Fails unless an APK carries the same game as a game.zip: the files under
``assets/automata/game/`` (what ``automata-cli build --target android``
adds) must be the files of the zip (what ``automata-cli stage --zip``
writes), byte for byte. CI checks that game.apk and game.zip at the top
of the repo are one game.

    same_game.py game.apk game.zip
"""

import sys
import zipfile

PREFIX = "assets/automata/game/"


def files(archive: zipfile.ZipFile, prefix: str = "") -> dict:
    return {
        info.filename[len(prefix):]: archive.read(info)
        for info in archive.infolist()
        if info.filename.startswith(prefix) and not info.is_dir()
    }


def main() -> int:
    with zipfile.ZipFile(sys.argv[1]) as apk, zipfile.ZipFile(sys.argv[2]) as game:
        in_apk, in_zip = files(apk, PREFIX), files(game)
    missing = sorted(set(in_zip) - set(in_apk))
    extra = sorted(set(in_apk) - set(in_zip))
    changed = sorted(n for n in set(in_apk) & set(in_zip) if in_apk[n] != in_zip[n])
    for label, names in (("only in the zip", missing), ("only in the APK", extra), ("different", changed)):
        if names:
            print(f"{label}: {', '.join(names)}")
    if missing or extra or changed:
        print(f"::error::{sys.argv[1]} and {sys.argv[2]} carry different games")
        return 1
    print(f"ok: the APK carries the zip's game ({len(in_zip)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
