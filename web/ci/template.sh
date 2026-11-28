#!/usr/bin/env bash
# Puts the web export template together: the player page and its scripts,
# Pyodide's runtime (from npm, pinned) and the engine wheel built by
# build_wheel.sh; everything a web build has except the game and
# player.json, which `automata-cli build --target web` adds. Writes it to
# $1 (default web/dist/template/); CI packs it with
# `automata-cli templates pack DIR --target web`.
set -euo pipefail

PYODIDE_VERSION="${PYODIDE_VERSION:-314.0.8}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
dist="$repo/web/dist"
out="$(realpath -m "${1:-$dist/template}")"

rm -rf "$out"
mkdir -p "$out/pyodide"
if [ ! -d "$dist/pyodide-npm/package" ]; then
  mkdir -p "$dist/pyodide-npm"
  (cd "$dist/pyodide-npm" && npm pack --silent "pyodide@$PYODIDE_VERSION" >/dev/null && tar -xzf pyodide-*.tgz)
fi
# The runtime only: the core module, the standard library and the lock
# file (no REPL consoles, type stubs or source maps).
for f in pyodide.mjs pyodide.asm.mjs pyodide.asm.wasm python_stdlib.zip pyodide-lock.json package.json; do
  cp "$dist/pyodide-npm/package/$f" "$out/pyodide/"
done
cp "$repo/web/player/index.html" "$repo/web/player/player.js" "$repo/web/player/audio-worklet.js" "$out/"
wheels=("$dist"/wheel/*.whl)
[ "${#wheels[@]}" -eq 1 ] && [ -f "${wheels[0]}" ] || {
  echo "expected one wheel in $dist/wheel (run build_wheel.sh)" >&2
  exit 1
}
cp "${wheels[0]}" "$out/"
# The module must only import what Pyodide provides.
rm -rf "$dist/wheel-unpacked" && mkdir -p "$dist/wheel-unpacked"
(cd "$dist/wheel-unpacked" && unzip -q "${wheels[0]}")
node "$repo/web/ci/wasm_imports.mjs" "$(find "$dist/wheel-unpacked" -name '_core*.so' | head -1)"
echo "web template in $out ($(du -sb "$out" | cut -f1) bytes)"
