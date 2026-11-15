#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Burlesque presentations engine host gate (Slice T6):
# Runs locked host unit and integration tests (PPTX/ODP/Markdown, layout, rendering,
# presenter view, vector PDF export verified against vellum-edit) and checks
# clean compilation on the no_std + alloc x86_64-sex-app.json target.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "[burlesque_host_gate] Running cargo test --locked -p burlesque-core -p burlesque --no-default-features..."
cargo test --locked -p burlesque-core
cargo test --locked -p burlesque --no-default-features

echo "[burlesque_host_gate] Checking x86_64-sex-app.json build target (no_std + alloc)..."
cargo check --locked -p burlesque-core --target x86_64-sex-app.json -Z build-std=core,compiler_builtins,alloc
cargo check --locked -p burlesque --target x86_64-sex-app.json -Z build-std=core,compiler_builtins,alloc

echo "[burlesque_host_gate] PASS"
