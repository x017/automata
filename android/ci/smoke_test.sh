#!/usr/bin/env bash
# Emulator smoke test for the SDL spike APK (android/app):
#
# 1. it starts and draws frames (screenshot 1-running.png),
# 2. a tap reaches the game as a touch,
# 3. Home sends it to the background: the engine lets go of the surface,
# 4. coming back attaches a new surface and frames run again
#    (3-resumed.png), with no crash,
# 5. a configuration change the manifest doesn't list (bold text) makes
#    Android re-create the activity, twice: the game keeps running in the
#    same process and draws on the new activity (4-recreated.png),
# 6. Back reaches the game, which quits.
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
wait_for "automata-spike stdlib (ok|missing)" 60
if log | grep -q "automata-spike stdlib missing"; then
  log | grep "automata-spike stdlib missing"
  echo "::error::standard library modules are missing from the APK"
  exit 1
fi
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

# Re-creation: the game must outlive the old activity (SDL alone would
# quit main(), which ends a python-for-android app).
pid=$(adb shell pidof "$pkg" | tr -d '\r')
for weight in 300 0; do
  n=$(log | grep -c "the new activity took over the running game" || true)
  adb shell settings put secure font_weight_adjustment "$weight"
  wait_for "activity re-created for configuration changes .*the game keeps running" 30 "" $((n + 1))
  wait_for "the new activity took over the running game" 60 "" $((n + 1))
  sleep 2
  now=$(adb shell pidof "$pkg" | tr -d '\r' || true)
  if [ "$now" != "$pid" ]; then
    echo "::error::the game didn't survive the activity re-creation (pid $pid -> ${now:-gone})"
    exit 1
  fi
  fail_on_crash
done
# Frames on the new activity: progress lines after the last take-over.
mark=$(log | grep -n "the new activity took over the running game" | tail -1 | cut -d: -f1)
for _ in $(seq 1 30); do
  [ "$(log | tail -n +"$mark" | grep -c "automata-spike frame=" || true)" -ge 2 ] && break
  sleep 1
done
[ "$(log | tail -n +"$mark" | grep -c "automata-spike frame=" || true)" -ge 2 ] ||
  { echo "::error::no frames after the activity was re-created"; exit 1; }
echo "ok: the game kept running through two activity re-creations"
sleep 4  # the loading screen goes once the new activity is set up
shot 4-recreated

adb shell input keyevent KEYCODE_BACK
wait_for "automata-spike back pressed" 30
wait_for "automata-spike finished" 30
fail_on_crash
echo "smoke test passed"
