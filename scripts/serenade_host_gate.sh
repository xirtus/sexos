#!/usr/bin/env bash
# Serenade host gate: audio recorder, metering, waveform mipmap, non-destructive editing and WAV container.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=== [1/2] Cargo test --locked -p serenade-core ==="
cargo test --locked -p serenade-core

echo "=== [2/2] Target check: x86_64-sex-app.json ==="
cargo check --target x86_64-sex-app.json -Z build-std=core,alloc -p serenade-core

echo "[serenade_host_gate] PASS: all serenade-core tests and target checks passed."
