#!/usr/bin/env bash
# Sets up a Rust toolchain and vendored dependencies in a network-restricted
# environment, from artifacts produced by the bootstrap-toolchain workflow.
#
# Requires: the GitHub CLI (`gh`), authenticated, and at least one successful
# bootstrap-toolchain run on this branch.
#
# Usage:
#   ./scripts/setup-offline-env.sh [branch]
#   export PATH="$HOME/.local/automata-rust/stable-x86_64-unknown-linux-gnu/bin:$PATH"
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
BRANCH="${1:-$(git branch --show-current)}"
TOOLCHAIN_HOME="${HOME}/.local/automata-rust"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

echo ">> Locating latest successful bootstrap-toolchain run on '${BRANCH}'"
RUN_ID="$(gh run list \
    --workflow bootstrap-toolchain.yml \
    --branch "${BRANCH}" \
    --status success \
    --limit 1 \
    --json databaseId \
    --jq '.[0].databaseId')"
if [ -z "${RUN_ID}" ]; then
    echo "!! No successful bootstrap-toolchain run found on branch '${BRANCH}'." >&2
    echo "!! Trigger one with: gh workflow run bootstrap-toolchain.yml --ref '${BRANCH}'" >&2
    exit 1
fi

echo ">> Downloading artifacts from run ${RUN_ID}"
gh run download "${RUN_ID}" --name rust-toolchain --dir "${WORK_DIR}/toolchain"
gh run download "${RUN_ID}" --name cargo-vendor --dir "${WORK_DIR}/vendor"

echo ">> Installing toolchain to ${TOOLCHAIN_HOME}"
mkdir -p "${TOOLCHAIN_HOME}"
tar -xzf "${WORK_DIR}/toolchain/rust-toolchain.tar.gz" -C "${TOOLCHAIN_HOME}"

echo ">> Extracting vendored dependencies into ${REPO_ROOT}/vendor"
tar -xzf "${WORK_DIR}/vendor/cargo-vendor.tar.gz" -C "${REPO_ROOT}"

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
