#!/usr/bin/env bash
# Sets up a Rust toolchain and vendored dependencies in a network-restricted
# environment, from the automation-owned `dev-artifacts` branch (produced by
# the bootstrap-toolchain workflow). Plain `git fetch` is enough — no
# crates.io or rustup access required.
#
# Usage:
#   ./scripts/setup-offline-env.sh
#   export PATH="$HOME/.local/automata-rust/stable-x86_64-unknown-linux-gnu/bin:$PATH"
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
TOOLCHAIN_HOME="${HOME}/.local/automata-rust"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

echo ">> Fetching dev-artifacts branch"
git -C "${REPO_ROOT}" fetch --depth 1 origin dev-artifacts

echo ">> Extracting artifact payloads"
git -C "${REPO_ROOT}" archive FETCH_HEAD | tar -x -C "${WORK_DIR}"
cd "${WORK_DIR}"
for name in rust-toolchain.tar.gz cargo-vendor.tar.gz; do
    if [ ! -f "${name}" ]; then
        cat "${name}".part-* > "${name}"
    fi
done

echo ">> Installing toolchain to ${TOOLCHAIN_HOME}"
mkdir -p "${TOOLCHAIN_HOME}"
tar -xzf rust-toolchain.tar.gz -C "${TOOLCHAIN_HOME}"

echo ">> Extracting vendored dependencies into ${REPO_ROOT}/vendor"
tar -xzf cargo-vendor.tar.gz -C "${REPO_ROOT}"

echo ">> Writing .cargo/config.toml (offline source replacement)"
mkdir -p "${REPO_ROOT}/.cargo"
cat > "${REPO_ROOT}/.cargo/config.toml" <<'EOF'
# Local offline configuration — intentionally git-ignored.
# Points Cargo at the vendored dependency copies. See docs/development.md.
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

# gilrs (gamepad input) links libudev on Linux via pkg-config. Sandboxes have
# the runtime library but neither the dev symlink nor pkg-config, so provide
# both locally. CI installs libudev-dev instead.
if [ -e /usr/lib/x86_64-linux-gnu/libudev.so.1 ]; then
    echo ">> Setting up libudev link shim (~/.local)"
    mkdir -p "${HOME}/.local/bin" "${HOME}/.local/lib"
    ln -sf /usr/lib/x86_64-linux-gnu/libudev.so.1 "${HOME}/.local/lib/libudev.so"
    cat > "${HOME}/.local/bin/pkg-config" <<'EOF'
#!/bin/sh
# Minimal pkg-config shim for offline sandboxes (answers for libudev only).
want_libs=""
for arg in "$@"; do
  case "$arg" in
    --libs) want_libs="1" ;;
    --modversion) echo 251; exit 0 ;;
    --variable=*) echo /usr; exit 0 ;;
    --exists) exit 0 ;;
  esac
done
[ -n "$want_libs" ] && printf '%s\n' "-L${HOME}/.local/lib -ludev"
exit 0
EOF
    chmod +x "${HOME}/.local/bin/pkg-config"
    echo "   Add to your environment:"
    echo "   export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo "   export PKG_CONFIG=\"\$HOME/.local/bin/pkg-config\""
    echo "   export LIBRARY_PATH=\"\$HOME/.local/lib\""
fi

echo
echo ">> Done. Activate the toolchain with:"
echo "   export PATH=\"${TOOLCHAIN_HOME}/stable-x86_64-unknown-linux-gnu/bin:\$PATH\""
