#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Batik image editor and digital painting engine host gate.
# Runs unit & golden pixel tests on host, builds release binary for x86_64-sex-app.json,
# checks for forbidden opcodes via key_bytes.py, and stages to iso_root/programs/batik.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
export PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | awk '/^host:/{print $2}')/bin:/opt/homebrew/opt/llvm/bin:$PATH"

echo "[batik.host.gate] 1/3: Running batik-core host tests..."
cargo test --locked -p batik-core

echo "[batik.host.gate] 2/3: Running batik app window tests..."
cargo test --locked -p batik --no-default-features

echo "[batik.host.gate] 3/3: Building release batik app for x86_64-sex-app.json..."
for seed in "" 1 2 3 4 5 6 7 8; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p batik --target x86_64-sex-app.json \
    -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/batik; then break; fi
  [[ "$seed" != "8" ]] || { echo "[FAIL] batik has refused key bytes"; exit 1; }
done

mkdir -p iso_root/programs
cp target/x86_64-sex-app/release/batik iso_root/programs/batik

echo "[batik.host.gate] PASS"
