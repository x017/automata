#!/usr/bin/env bash
# Rebuilds game.zip at the top of the repo: Coin Run (examples/oberon/
# coin_run), exported the way users export it (`automata-cli init`, then
# `automata-cli stage --zip`): the game folder and game.json the web
# player loads. The zip is reproducible, so `--check` can tell whether the
# committed one still matches the example.
#
#   scripts/game_zip.sh            rewrite game.zip
#   scripts/game_zip.sh --check    fail if game.zip is out of date
#
# CLI defaults to target/debug/automata-cli (built if missing).
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
cli="${CLI:-$repo/target/debug/automata-cli}"
[ -x "$cli" ] || (cd "$repo" && cargo build -q -p automata-cli)

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# A clean copy: no editor state, no bytecode, no build.ron (as the CI
# Android build: `init` writes one).
cp -r "$repo/examples/oberon/coin_run" "$work/coin_run"
rm -rf "$work/coin_run/.oberon" "$work/coin_run/build.ron"
find "$work/coin_run" -name __pycache__ -prune -exec rm -rf {} +
"$cli" init "$work/coin_run" --scene level.ron --name "Coin Run" --package org.automata.coinrun >/dev/null
"$cli" stage "$work/coin_run" --out "$work/stage" --zip "$work/game.zip"

if [ "${1:-}" = "--check" ]; then
  if ! cmp -s "$work/game.zip" "$repo/game.zip"; then
    echo "::error::game.zip is out of date with examples/oberon/coin_run: run scripts/game_zip.sh and commit it" >&2
    exit 1
  fi
  echo "ok: game.zip matches examples/oberon/coin_run"
else
  cp "$work/game.zip" "$repo/game.zip"
  echo "wrote $repo/game.zip ($(wc -c < "$repo/game.zip") bytes)"
fi
