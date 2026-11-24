#!/usr/bin/env bash
# Builds an Android APK of a Python game with python-for-android.
#
#   android/build_apk.sh [GAME_DIR] [ARCH]
#
# GAME_DIR holds main.py (default: android/app, the SDL spike);
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

mkdir -p "$repo/dist"
cd "$repo/dist"
p4a apk \
  --private "$game" \
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
