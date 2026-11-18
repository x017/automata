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

echo
echo ">> Done. Activate the toolchain with:"
echo "   export PATH=\"${TOOLCHAIN_HOME}/stable-x86_64-unknown-linux-gnu/bin:\$PATH\""
