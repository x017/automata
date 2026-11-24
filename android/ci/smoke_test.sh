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

log() { adb logcat -d -v brief; }
dump() { log > "$out/logcat.txt" || true; }
trap dump EXIT

fail_on_crash() {
  if log | grep -E "FATAL EXCEPTION|Fatal signal|panicked at|Traceback|PanicException" > "$out/crash.txt"; then
    echo "::error::the app crashed"
    cat "$out/crash.txt"
    exit 1
  fi
}

# Waits until $4 (default 1) log lines match $1 after the first line
# matching $3 (default: anywhere); $2 is the timeout in seconds.
wait_for() {
  local pattern="$1" timeout="$2" after="${3:-}" count="${4:-1}"
  for _ in $(seq 1 "$timeout"); do
    if log | awk -v p="$pattern" -v a="$after" -v n="$count" \
      'a == "" || $0 ~ a { seen = 1 } seen && $0 ~ p { found++ } END { exit found < n }'; then
      echo "ok: $pattern"
      return 0
    fi
    fail_on_crash
    sleep 1
  done
  echo "::error::timed out after ${timeout}s waiting for: $pattern"
  stacks
  log | grep -E "python|SDL|automata" | tail -60 || true
  exit 1
}

# Every thread's stack (native and Java) and the activity's state, to see
# where a hung app is stuck.
stacks() {
  local pid
  # The shell user may not trace the app; root may.
  adb root > /dev/null 2>&1 && adb wait-for-device || true
  pid=$(adb shell pidof "$pkg" | tr -d '\r' || true)
  {
    echo "== pid ${pid:-none}"
    [ -n "$pid" ] && adb shell debuggerd -b "$pid" 2>&1
    echo "== activity"
    adb shell dumpsys activity activities 2>&1 | grep -E -A3 "$pkg" | head -60
  } > "$out/stacks.txt" || true
}

shot() { adb exec-out screencap -p > "$out/$1.png"; }

launch() { adb shell monkey -p "$pkg" -c android.intent.category.LAUNCHER 1 > /dev/null; }

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
