#!/usr/bin/env bash
# Host gate for Corset (crates/corset-core):
# Runs cargo test --locked -p corset-core and validates x86_64-sex-app.json no_std target check.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "[corset.host.gate] Running host unit and integration tests..."
cargo test --locked -p corset-core

echo "[corset.host.gate] Checking SexOS target build (x86_64-sex-app.json)..."
cargo check --target x86_64-sex-app.json -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem -p corset-core

echo "[corset.host.gate] PASS"
