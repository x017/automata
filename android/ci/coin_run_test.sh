#!/usr/bin/env bash
# Emulator test for Coin Run, built as an exported game
# (`automata-export stage` + android/build_apk.sh):
#
# 1. it starts: the level loads and its scripts run (1-start.png);
# 2. keys reach the scripts: F2 cycles the graphics preset (2-playing.png);
# 3. the setting lands in the app's private storage;
# 4. after Android kills the app, the next start remembers it: F2 goes
#    on from the saved preset;
# 5. the touch controls show by themselves on Android, and the left stick
#    walks the player forward into the coin ahead (3-stick.png);
# 6. the Jump button lights up while it's held (4-jump.png).
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

# --- touch controls ---------------------------------------------------
log | grep -q "Left stick to move" ||
  { echo "::error::the touch controls didn't show (no touch hint at start)"; exit 1; }
echo "ok: touch controls shown"

# Where things are: the display (landscape) and the game's surface, which
# sits below the status bar.
read -r W H < <(adb shell wm size | tr -d '\r' | sed -n 's/.*: \([0-9]*\)x\([0-9]*\).*/\1 \2/p' | tail -1 |
  awk '{ if ($1 > $2) print $1, $2; else print $2, $1 }')
read -r w h < <(log | sed -n 's/.*surface configured.* width=\([0-9]*\) height=\([0-9]*\).*/\1 \2/p' | tail -1)
oy=$((H - h))
echo "display ${W}x${H}, surface ${w}x${h}, top ${oy}"
# Positions as touch_controls lays them out (fractions of the shorter side).
read -r sx sy push jx jy jr < <(awk -v w="$w" -v h="$h" -v oy="$oy" 'BEGIN {
  m = (w < h) ? w : h; r = 0.085 * m * 1.2; margin = 0.05 * m
  printf "%d %d %d %d %d %d\n", 0.2 * w, 0.75 * h + oy, 0.15 * m * 1.3, \
    w - margin - r, h - margin - r + oy, r }')

# The intro flies the camera for about 6 seconds; walk once it's behind
# the player, looking at coin 5 straight ahead.
sleep 8
adb shell input motionevent DOWN "$sx" "$sy"
for step in 1 2 3 4; do
  adb shell input motionevent MOVE "$sx" $((sy - push * step / 4))
done
sleep 0.5
shot coin-3-stick
sleep 2
adb shell input motionevent UP "$sx" $((sy - push))
wait_for "Score: 1" 15 "Collect [0-9]+ coins"
echo "ok: the stick walked the player into a coin"

# Jump: the button is drawn brighter while it's held.
sleep 2
shot coin-4-before
adb shell input motionevent DOWN "$jx" "$jy"
sleep 0.4
shot coin-4-jump
adb shell input motionevent UP "$jx" "$jy"
box="$((2 * jr))x$((2 * jr))+$((jx - jr))+$((jy - jr))"
before=$(convert "$out/coin-4-before.png" -crop "$box" -colorspace gray -format "%[fx:mean]" info:)
held=$(convert "$out/coin-4-jump.png" -crop "$box" -colorspace gray -format "%[fx:mean]" info:)
echo "Jump button brightness: $before, held $held"
if ! awk -v a="$before" -v b="$held" 'BEGIN { exit !(b > a + 0.015) }'; then
  echo "::error::the Jump button didn't light up when held ($before -> $held)"
  exit 1
fi
rm -f "$out/coin-4-before.png"
fail_on_crash
echo "coin run test passed"
