# dev-artifacts

Automation-owned branch carrying the Rust toolchain and vendored
dependencies for network-restricted development environments.
Force-pushed by the `bootstrap-toolchain` workflow.

- Do not branch from it, do not merge it, do not develop on it.
- Consume it with `scripts/setup-offline-env.sh`.
- Rejoin split tarballs with: `cat <name>.part-* > <name>`
