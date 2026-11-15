#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Burlesque presentations engine host gate (Slice T6 / G-24):
# Runs locked host unit and integration tests (PPTX/ODP/Markdown, layout, rendering,
# themes, transitions, speaker notes, presenter view, vector PDF export verified against
# vellum-edit) and builds the release x86_64-sex-app.json PIE binary, checking for 0 forbidden opcodes.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "[burlesque_host_gate] Running cargo test --locked -p burlesque-core..."
cargo test --locked -p burlesque-core

echo "[burlesque_host_gate] Running cargo test --locked -p burlesque --no-default-features..."
cargo test --locked -p burlesque --no-default-features

echo "[burlesque_host_gate] Checking x86_64-sex-app.json build target (no_std + alloc)..."
cargo check --locked -p burlesque-core --target x86_64-sex-app.json -Z build-std=core,compiler_builtins,alloc
cargo check --locked -p burlesque --target x86_64-sex-app.json -Z build-std=core,compiler_builtins,alloc

echo "[burlesque_host_gate] Building release x86_64-sex-app.json PIE binary..."
for seed in "" 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p burlesque --target x86_64-sex-app.json \
    -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/burlesque; then break; fi
  [[ "$seed" != "16" ]] || { echo "[FAIL] burlesque binary has refused key bytes"; exit 1; }
done

mkdir -p iso_root/programs
cp target/x86_64-sex-app/release/burlesque iso_root/programs/burlesque

echo "[burlesque_host_gate] PASS"
