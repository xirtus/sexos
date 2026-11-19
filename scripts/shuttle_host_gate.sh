#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Shuttle: text shaping, bidirectional layout, font fallback, and font picker for SexOS.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "[shuttle.host.gate] Running host test suite..."
cargo test --locked -p shuttle -p sex-font -p sex-shape --features sex-shape/shaping

echo "[shuttle.host.gate] Checking confined app target compilation (x86_64-sex-app)..."
cargo check -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
  --target x86_64-sex-app.json -p shuttle

echo "[shuttle.host.gate] PASS"
