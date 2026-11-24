#!/usr/bin/env bash
# Emulator test for Coin Run, built as an exported game
# (`automata-export stage` + android/build_apk.sh):
#
# 1. it starts: the level loads and its scripts run (1-start.png);
# 2. keys reach the scripts: F2 cycles the graphics preset (2-playing.png);
# 3. the setting lands in the app's private storage;
# 4. after Android kills the app, the next start remembers it: F2 goes
#    on from the saved preset.
#
#   android/ci/coin_run_test.sh APK [OUT_DIR]
set -euo pipefail

apk="$1"
out="${2:-android-smoke/coin_run}"
pkg="${AUTOMATA_PACKAGE:-org.automata.coinrun}"
mkdir -p "$out"

# shellcheck source=android/ci/lib.sh
. "$(dirname "$0")/lib.sh"

# The preset F2 picked last, from rules.py's "Graphics: <preset>" line.
last_preset() { log | sed -n 's/.*Graphics: \([a-z]*\).*/\1/p' | tail -1; }

adb install -r "$apk"
adb logcat -c
launch

# First start unpacks Python and the game.
wait_for "Collect [0-9]+ coins" 240
wait_for "window and GPU context ready" 60
sleep 6
shot coin-1-start

adb shell input keyevent KEYCODE_SPACE
adb shell input keyevent KEYCODE_F2
wait_for "Graphics: " 30
first=$(last_preset)
echo "F2 chose: $first"
sleep 4
shot coin-2-playing
fail_on_crash

# ctx.settings is written in the frame it changes.
sleep 1
in_app() { adb shell run-as "$pkg" "$@" 2>&1 | tr -d '\r' || true; }
in_app find files -maxdepth 2 -not -path "files/app/*" > "$out/files.txt"
settings=$(in_app find files -name settings.json | grep "settings.json$" | head -1 || true)
if [ -z "$settings" ]; then
  echo "::error::no settings.json in the app's storage: $(grep -v "^$" "$out/files.txt" | head -30 | tr '\n' ' ' | cut -c1-900)"
  exit 1
fi
echo "settings: $settings"
in_app cat "'$settings'" | tee "$out/settings.json"
if ! grep -q "\"graphics\": *\"$first\"" "$out/settings.json"; then
  echo "::error::$settings doesn't hold graphics=$first: $(tr '\n' ' ' < "$out/settings.json" | cut -c1-400)"
  exit 1
fi

# Android kills apps in the background without warning; the next start
# must find the setting.
adb shell am force-stop "$pkg"
adb logcat -c
launch
wait_for "Collect [0-9]+ coins" 120
wait_for "window and GPU context ready" 60
sleep 3
adb shell input keyevent KEYCODE_F2
wait_for "Graphics: " 30
second=$(last_preset)
echo "F2 after the restart chose: $second"
presets=(low medium high ultra beautiful)
for i in "${!presets[@]}"; do
  [ "${presets[$i]}" = "$first" ] && expected="${presets[$(((i + 1) % ${#presets[@]}))]}"
done
if [ "$second" != "${expected:-}" ]; then
  echo "::error::after the restart F2 chose $second, expected ${expected:-?} (the setting wasn't kept)"
  exit 1
fi
fail_on_crash
echo "coin run test passed"
