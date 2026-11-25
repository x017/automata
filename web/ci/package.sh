#!/usr/bin/env bash
# Puts the web player together in web/dist/site/: Pyodide (from npm,
# pinned), the page, the engine wheel built by build_wheel.sh, and a game
# script (default: the spike), with player.json naming them.
set -euo pipefail

PYODIDE_VERSION="${PYODIDE_VERSION:-314.0.8}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
dist="$repo/web/dist"
site="$dist/site"
main="${1:-$repo/web/spike/main.py}"

rm -rf "$site"
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
cp "$repo/web/player/index.html" "$repo/web/player/player.js" "$site/"
wheel="$(basename "$(ls "$dist"/wheel/*.whl | head -1)")"
cp "$dist/wheel/$wheel" "$site/"
cp "$main" "$site/main.py"
printf '{"wheel": "%s", "main": "main.py"}\n' "$wheel" > "$site/player.json"
du -ab "$site" | sort -n | tail -20
