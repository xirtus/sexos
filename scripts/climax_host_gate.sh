#!/usr/bin/env bash
# Climax host gate: microbenchmarks, sustained torture runner, thermal throttling analyzer and report formatter.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=== [1/2] Cargo test --locked -p climax-core ==="
cargo test --locked -p climax-core

echo "=== [2/2] Target check: x86_64-sex-app.json ==="
cargo check --target x86_64-sex-app.json -Z build-std=core,alloc -p climax-core

echo "[climax_host_gate] PASS: all climax-core tests and target checks passed."
