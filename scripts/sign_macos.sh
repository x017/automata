#!/usr/bin/env bash
# Signs an exported AUTOMATA macOS app (automata-cli build --target macos)
# so it can be notarised and opened without warnings. Run it on a Mac.
#
#   scripts/sign_macos.sh "Coin Run.app" "Developer ID Application: Your Name (TEAMID)"
#   scripts/sign_macos.sh "Coin Run.app" -     # ad hoc: for testing on your own Macs
#
# Every native file is signed, innermost first (`codesign --deep` doesn't
# look inside Contents/Resources, where Python's libraries live), then the
# app, which seals the game files. With a real identity it uses the hardened
# runtime and a secure timestamp, which notarisation requires, and the
# entitlements in macos.entitlements next to this script (override with
# ENTITLEMENTS=file).
set -euo pipefail

app=${1:?usage: sign_macos.sh APP IDENTITY}
identity=${2:?usage: sign_macos.sh APP IDENTITY}
app=${app%/}
[ -f "$app/Contents/Info.plist" ] || { echo "error: $app is not an app bundle" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
entitlements=${ENTITLEMENTS:-$here/macos.entitlements}

common=(--force --sign "$identity")
if [ "$identity" != "-" ]; then
  common+=(--timestamp --options runtime)
fi
programs=("${common[@]}")
if [ "$identity" != "-" ]; then
  programs+=(--entitlements "$entitlements")
fi

is_mach_o() { file -b "$1" | grep -q 'Mach-O'; }

count=0
# 1. Python's libraries and programs in Contents/Resources.
while IFS= read -r -d '' f; do
  is_mach_o "$f" || continue
  if [ -x "$f" ] && [[ "$f" == */bin/* ]]; then
    codesign "${programs[@]}" "$f"
  else
    codesign "${common[@]}" "$f"
  fi
  count=$((count + 1))
done < <(find "$app/Contents/Resources" -type f \( -name '*.so' -o -name '*.dylib' -o -perm -0100 \) -print0)

# 2. The interpreters next to the launcher.
launcher=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist")
for f in "$app/Contents/MacOS/"*; do
  if [ "$(basename "$f")" != "$launcher" ] && is_mach_o "$f"; then
    codesign "${programs[@]}" "$f"
    count=$((count + 1))
  fi
done

# 3. The app: the launcher, Info.plist and every other file, sealed.
codesign "${programs[@]}" "$app"
echo "signed $count files and $app"
codesign --verify --deep --strict --verbose=2 "$app"
