#!/usr/bin/env bash
# Puts the web player together: the web template (template.sh: Pyodide,
# the page, the engine wheel built by build_wheel.sh) and a game, with
# player.json naming them; a quick way to try a game or script without
# automata-cli (which does the same for projects: build --target web).
# Writes the site to $SITE (default web/dist/site/) and the same as a zip
# beside it ($SITE.zip, ready for itch.io: index.html at the top).
#
#   package.sh                     the spike (web/spike/main.py)
#   package.sh path/to/game.py     a single script
#   package.sh path/to/folder      a staged game folder (with game.json,
#                                  from `automata-cli stage`), zipped
#   package.sh path/to/game.zip    such a folder, already zipped
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
dist="$repo/web/dist"
site="$(realpath -m "${SITE:-$dist/site}")"
game="$(realpath -m "${1:-$repo/web/spike/main.py}")"

rm -rf "$site.zip"
"$repo/web/ci/template.sh" "$site"
wheel="$(basename "$(ls "$site"/*.whl)")"

if [ -d "$game" ]; then
  [ -f "$game/game.json" ] || { echo "$game has no game.json (stage it with automata-cli stage)" >&2; exit 1; }
  (cd "$game" && zip -qr -9 -X "$site/game.zip" . -x '*/__pycache__/*' '__pycache__/*')
  entry="\"game\": \"game.zip\""
elif [[ "$game" == *.zip ]]; then
  cp "$game" "$site/game.zip"
  entry="\"game\": \"game.zip\""
else
  cp "$game" "$site/main.py"
  entry="\"main\": \"main.py\""
fi
printf '{"wheel": "%s", %s}\n' "$wheel" "$entry" > "$site/player.json"
(cd "$site" && zip -qr -9 -X "$site.zip" .)
du -ab "$site" | sort -n | tail -20
ls -la "$site.zip"
