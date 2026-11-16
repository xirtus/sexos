#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail
vault_gate_root="$(cd "$(dirname "$0")/.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$vault_gate_root/target/vault-dependency-host}"
# Standalone manifests need their own pinned dev-dependency resolution. Both
# suites exercise the same local Argon2 patch; no registry copy substitutes it.
cargo test --locked --manifest-path "$vault_gate_root/vendor/rust-argon2-3.0.0/Cargo.toml" --lib --test integration_test
cargo test --locked --manifest-path "$vault_gate_root/vendor/keepass-0.17.0/Cargo.toml" \
    --config "patch.crates-io.rust-argon2.path=\"$vault_gate_root/vendor/rust-argon2-3.0.0\"" \
    --features save_kdbx4 --lib sexos
