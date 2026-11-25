#!/usr/bin/env bash
# Builds an Android APK of a Python game with python-for-android.
#
#   android/build_apk.sh [GAME_DIR] [ARCH]
#
# GAME_DIR is either
# - a game folder from `automata-cli stage` (it has game.json): the APK
#   runs it with automata.player, like the desktop launchers; or
# - a folder with a main.py to run as is (default: android/app, the spike).
# ARCH is x86_64 (emulators, the default) or arm64-v8a (phones).
# Needs: python-for-android (develop), rustup, an Android SDK
# (ANDROID_HOME) and NDK r28c (ANDROID_NDK_HOME). The APK lands in dist/.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
game="$(cd "${1:-$repo/android/app}" && pwd)"
arch="${2:-x86_64}"
stage="${AUTOMATA_STAGE:-$repo/target/android-stage}"

# The recipe builds from a clean copy of the repository (tracked files only,
# so local build caches stay out), with the Android pyproject at its root.
rm -rf "$stage"
mkdir -p "$stage"
git -C "$repo" ls-files -z --cached --others --exclude-standard \
  | (cd "$repo" && xargs -0 cp --parents -t "$stage" 2>/dev/null || true)
cp "$repo/android/pyproject.toml" "$stage/pyproject.toml"
export P4A_automata_DIR="$stage"

# The app's files: an exported game goes in game/ next to the player's
# entry point.
private="$game"
if [ -f "$game/game.json" ]; then
  private="$stage-private"
  rm -rf "$private"
  mkdir -p "$private"
  cp "$repo/android/player/main.py" "$private/main.py"
  cp -a "$game" "$private/game"
fi

mkdir -p "$repo/dist"
cd "$repo/dist"
# Scripts ship as source: the engine loads behaviour scripts from their .py
# files, as on desktop.
p4a apk \
  --private "$private" \
  --no-byte-compile-python \
  --package "${AUTOMATA_PACKAGE:-org.automata.spike}" \
  --name "${AUTOMATA_NAME:-AUTOMATA}" \
  --version "${AUTOMATA_VERSION:-0.1.0}" \
  --bootstrap sdl3 \
  --requirements python3,automata \
  --local-recipes "$repo/android/recipes" \
  --add-source "$repo/android/java" \
  --android-entrypoint org.automata.AutomataActivity \
  --arch "$arch" \
  --orientation landscape \
  --android-api "${ANDROID_API:-35}" \
  --ndk-api 24 \
  --sdk-dir "${ANDROID_HOME:?set ANDROID_HOME}" \
  --ndk-dir "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME}" \
  --dist-name automata \
  --skip-prebuilt

# Name the APK after its package, so several builds can sit side by side.
built=$(ls -t ./*.apk | head -1)
mv -f "$built" "${AUTOMATA_PACKAGE:-org.automata.spike}-$arch.apk"
echo "built dist/${AUTOMATA_PACKAGE:-org.automata.spike}-$arch.apk"
