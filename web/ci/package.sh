#!/usr/bin/env bash
# Puts the web player together: Pyodide (from npm, pinned), the page, the
# engine wheel built by build_wheel.sh, and a game, with player.json
# naming them. Writes the site to $SITE (default web/dist/site/) and the
# same as a zip beside it ($SITE.zip, ready for itch.io: index.html at the
# top).
#
#   package.sh                     the spike (web/spike/main.py)
#   package.sh path/to/game.py     a single script
#   package.sh path/to/folder      a staged game folder (with game.json,
#                                  from `automata-cli stage`), zipped
#   package.sh path/to/game.zip    such a folder, already zipped
set -euo pipefail

PYODIDE_VERSION="${PYODIDE_VERSION:-314.0.8}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
dist="$repo/web/dist"
site="${SITE:-$dist/site}"
game="${1:-$repo/web/spike/main.py}"

rm -rf "$site" "$site.zip"
mkdir -p "$site/pyodide"
if [ ! -d "$dist/pyodide-npm/package" ]; then
  mkdir -p "$dist/pyodide-npm"
  (cd "$dist/pyodide-npm" && npm pack --silent "pyodide@$PYODIDE_VERSION" >/dev/null && tar -xzf pyodide-*.tgz)
fi
# The runtime only: the core module, the standard library and the lock
# file (no REPL consoles, type stubs or source maps).
for f in pyodide.mjs pyodide.asm.mjs pyodide.asm.wasm python_stdlib.zip pyodide-lock.json package.json; do
  cp "$dist/pyodide-npm/package/$f" "$site/pyodide/"
done
cp "$repo/web/player/index.html" "$repo/web/player/player.js" "$repo/web/player/audio-worklet.js" "$site/"
wheel="$(basename "$(ls "$dist"/wheel/*.whl | head -1)")"
cp "$dist/wheel/$wheel" "$site/"
# The module must only import what Pyodide provides.
rm -rf "$dist/wheel-unpacked" && mkdir -p "$dist/wheel-unpacked"
(cd "$dist/wheel-unpacked" && unzip -q "$dist/wheel/$wheel")
node "$repo/web/ci/wasm_imports.mjs" "$(find "$dist/wheel-unpacked" -name '_core*.so' | head -1)"

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
