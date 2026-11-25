# Helpers for the emulator tests, sourced after setting `pkg` (the app's
# package) and `out` (where logs and screenshots go). Saves the logcat to
# $out/logcat.txt when the test ends, however it ends.

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

# Appends how long a start took to $out/startup.txt: each milestone in
# seconds after Android started the app's process, from the logcat's
# timestamps. $1 names the start; the rest are "label=pattern" pairs, in
# the order they happen.
startup_times() {
  local name="$1"
  shift
  adb logcat -d -v epoch | awk -v name="$name" -v pkg="$pkg" -v specs="$*" '
    BEGIN { n = split(specs, pairs, " ") }
    !t0 && /Start proc [0-9]+:/ && index($0, ":" pkg "/") { t0 = $1 }
    t0 {
      for (i = 1; i <= n; i++) {
        split(pairs[i], kv, "=")
        if (!(i in at) && $0 ~ kv[2]) at[i] = $1
      }
    }
    END {
      printf "%s\n", name
      if (!t0) { print "  (no \"Start proc\" line in the log)"; exit }
      for (i = 1; i <= n; i++) {
        split(pairs[i], kv, "=")
        label = kv[1]; gsub(/_/, " ", label)
        if (i in at) printf "  %-28s %6.2f s\n", label, at[i] - t0
        else printf "  %-28s      -\n", label
      }
    }' | tee -a "$out/startup.txt"
}

launch() { adb shell monkey -p "$pkg" -c android.intent.category.LAUNCHER 1 > /dev/null; }

