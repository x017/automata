#!/usr/bin/env bash
# Puts the web export template together: the player page and its scripts,
# Pyodide's runtime (from npm, pinned) and the engine wheel built by
# build_wheel.sh; everything a web build has except the game and
# player.json, which `automata-cli build --target web` adds. Writes it to
# $1 (default web/dist/template/); CI packs it with
# `automata-cli templates pack DIR --target web`.
#
# Trimmed for download size and start-up time (see web/README.md):
# Python's standard library without the modules stdlib-drop.txt names and
# with the ones a game imports at start compiled (stdlib.mjs), the engine
# wheel's Python compiled and its type stubs left out (slim_wheel.mjs), and
# a package index that lists no packages (web builds install only the
# engine, by URL).
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
npm_pkg="$dist/pyodide-npm/package"
# The runtime only: the core module and the standard library (no REPL
# consoles, type stubs, source maps or package.json).
for f in pyodide.mjs pyodide.asm.mjs pyodide.asm.wasm; do
  cp "$npm_pkg/$f" "$out/pyodide/"
done
node "$repo/web/ci/stdlib.mjs" "$npm_pkg" "$npm_pkg/python_stdlib.zip" "$out/pyodide/python_stdlib.zip"
# Pyodide reads its package index at start; web builds load no packages
# from it, so the index keeps only its header (119 KB less to fetch).
python3 -c '
import json, sys
info = json.load(open(sys.argv[1]))["info"]
json.dump({"info": info, "packages": {}}, open(sys.argv[2], "w"), separators=(",", ":"))
' "$npm_pkg/pyodide-lock.json" "$out/pyodide/pyodide-lock.json"
cp "$repo/web/player/index.html" "$repo/web/player/player.js" "$repo/web/player/audio-worklet.js" "$out/"
wheels=("$dist"/wheel/*.whl)
[ "${#wheels[@]}" -eq 1 ] && [ -f "${wheels[0]}" ] || {
  echo "expected one wheel in $dist/wheel (run build_wheel.sh)" >&2
  exit 1
}
node "$repo/web/ci/slim_wheel.mjs" "$npm_pkg" "${wheels[0]}" "$out/$(basename "${wheels[0]}")"
# The module must only import what Pyodide provides.
rm -rf "$dist/wheel-unpacked" && mkdir -p "$dist/wheel-unpacked"
(cd "$dist/wheel-unpacked" && unzip -q "${wheels[0]}")
node "$repo/web/ci/wasm_imports.mjs" "$(find "$dist/wheel-unpacked" -name '_core*.so' | head -1)"
chmod 644 "$out/pyodide/python_stdlib.zip" "$out"/*.whl
python3 -c '
import sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
print("engine wheel, largest files (bytes, compressed):")
for i in sorted(z.infolist(), key=lambda i: -i.compress_size)[:8]:
    print(f"  {i.compress_size:>9} {i.file_size:>10}  {i.filename}")
' "$out/$(basename "${wheels[0]}")"
# How much Binaryen's size optimizer would still take off the engine module
# (a measurement only; rustc already has emcc optimize it with -Os).
wasm_opt="$(command -v wasm-opt || find "$HOME" -path '*upstream/bin/wasm-opt' -print -quit 2>/dev/null || true)"
core="$(find "$dist/wheel-unpacked" -name '_core*.so' | head -1)"
if [ -n "$wasm_opt" ] && [ -n "$core" ]; then
  if "$wasm_opt" -Oz --enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int \
       --enable-sign-ext --enable-mutable-globals --enable-multivalue --enable-reference-types \
       --enable-exception-handling "$core" -o "$dist/core-oz.wasm" 2> "$dist/wasm-opt.log"; then
    echo "engine module: $(stat -c %s "$core") bytes; wasm-opt -Oz: $(stat -c %s "$dist/core-oz.wasm") bytes" \
      "(gzipped $(gzip -9c "$core" | wc -c) -> $(gzip -9c "$dist/core-oz.wasm" | wc -c))"
  else
    echo "wasm-opt -Oz failed: $(head -3 "$dist/wasm-opt.log")"
  fi
fi
echo "web template in $out ($(du -sb "$out" | cut -f1) bytes)"
