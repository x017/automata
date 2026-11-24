#!/usr/bin/env bash
# Emulator smoke test for the SDL spike APK (android/app):
#
# 1. it starts and draws frames (screenshot 1-running.png),
# 2. a tap reaches the game as a touch,
# 3. Home sends it to the background: the engine lets go of the surface,
# 4. coming back attaches a new surface and frames run again
#    (3-resumed.png), with no crash,
# 5. Back reaches the game, which quits.
#
#   android/ci/smoke_test.sh APK [OUT_DIR]
set -euo pipefail

apk="$1"
out="${2:-android-smoke}"
pkg="${AUTOMATA_PACKAGE:-org.automata.spike}"
mkdir -p "$out"

# shellcheck source=android/ci/lib.sh
. "$(dirname "$0")/lib.sh"

adb install -r "$apk"
adb logcat -c
launch

# First start unpacks Python and the game, which takes a while on an emulator.
wait_for "automata-spike start" 240
wait_for "GPU adapter selected" 60
wait_for "automata-spike frame=120" 120
sleep 1
shot 1-running

# Tap inside the screen whatever its size and rotation (`wm size` gives the
# unrotated panel; the game turns it to landscape).
side=$(adb shell wm size | tr -d '\r' |
  awk -F'[ x]' '/size/ { w = $(NF-1); h = $NF } END { print (w < h ? w : h) }')
adb shell input tap $((side / 2)) $((side / 2))
wait_for "automata-spike touch finger=0" 30

adb shell input keyevent KEYCODE_HOME
wait_for "entering the background" 30
sleep 3
shot 2-background

launch
wait_for "back in the foreground" 30
# Frames run again on the new surface: two progress lines after resuming.
wait_for "automata-spike frame=" 30 "back in the foreground" 2
shot 3-resumed
fail_on_crash

adb shell input keyevent KEYCODE_BACK
wait_for "automata-spike back pressed" 30
wait_for "automata-spike finished" 30
fail_on_crash
echo "smoke test passed"
