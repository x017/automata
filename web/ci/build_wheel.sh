#!/usr/bin/env bash
# Builds the engine's Pyodide wheel (web/engine) into web/dist/wheel/.
#
# Needs a host Python matching Pyodide's (3.14) with pyodide-build
# installed, and rustup. Installs the cross-build environment, Emscripten
# and the Rust toolchain that Pyodide's ABI asks for, then runs
# `pyodide build` in a staged copy of the repository with
# web/pyproject.toml at its root.
set -euo pipefail

PYODIDE_VERSION="${PYODIDE_VERSION:-314.0.8}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
out="$repo/web/dist/wheel"
stage="${STAGE:-$(mktemp -d)}"

pyodide xbuildenv install "$PYODIDE_VERSION"
pyodide xbuildenv install-emscripten
emsdk_dir="$(pyodide config get emsdk_dir 2>/dev/null || true)"
if [ -z "$emsdk_dir" ] || [ ! -f "$emsdk_dir/emsdk_env.sh" ]; then
  emsdk_dir="$(dirname "$(find "$HOME" -path '*xbuildenv*' -name emsdk_env.sh -print -quit)")"
fi
# shellcheck disable=SC1091
source "$emsdk_dir/emsdk_env.sh"
echo "emcc: $(emcc --version | head -1)"

toolchain="$(pyodide config get rust_toolchain)"
target_url="$(pyodide config get rust_emscripten_target_url || true)"
echo "Rust toolchain $toolchain, emscripten target ${target_url:-from rustup}"
rustup toolchain install "$toolchain" --profile minimal
rustup default "$toolchain"
if [ -z "$target_url" ]; then
  rustup target add wasm32-unknown-emscripten --toolchain "$toolchain"
else
  # Pyodide's own build of the target's standard library (wasm exceptions),
  # in place of rustup's, as `pyodide build-recipes` installs it.
  rustlib="$(dirname "$(dirname "$(rustup which --toolchain "$toolchain" rustc)")")/lib/rustlib"
  rm -rf "$rustlib/wasm32-unknown-emscripten"
  curl -fsSL "$target_url" | tar -xj -C "$rustlib" 2>/dev/null \
    || curl -fsSL "$target_url" | tar -xz -C "$rustlib"
fi
# Export only the module's entry point. rustc exports every `#[no_mangle]`
# function of every linked crate from a cdylib, and wgpu's GL backend links
# wasm-bindgen on any wasm32 target (unused under Emscripten): its exports
# would keep imports from wasm-bindgen's placeholder module, which Pyodide
# can't provide. The later -sEXPORTED_FUNCTIONS wins over rustc's list.
CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUSTFLAGS="$(pyodide config get rustflags) -C link-arg=-sEXPORTED_FUNCTIONS=_PyInit__core"
export CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUSTFLAGS
echo "RUSTFLAGS for the target: $CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUSTFLAGS"

# The staged copy: no build outputs, the web pyproject at the root.
rsync -a --exclude target --exclude vendor --exclude .cargo --exclude web/dist \
  --exclude .git "$repo/" "$stage/"
cp "$repo/web/pyproject.toml" "$stage/pyproject.toml"
mkdir -p "$out"
(cd "$stage" && pyodide build --outdir "$out")
ls -l "$out"
