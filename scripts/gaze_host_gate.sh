#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Gaze photo library, albums, faces-free smart search & non-destructive edits host gate.
# Runs gaze-core, batik-core, and gaze-codecs unit tests on host, builds release Gaze (apps/view)
# PIE binary for x86_64-sex-app.json, verifies 0 forbidden opcodes via key_bytes.py,
# checks 32 MiB program image limit, and stages to iso_root/programs/gaze.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
export PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | awk '/^host:/{print $2}')/bin:/opt/homebrew/opt/llvm/bin:$PATH"

echo "[gaze.host.gate] 1/3: Running gaze-core & shared batik-core host tests..."
cargo test --locked -p gaze-core
cargo test --locked -p batik-core

echo "[gaze.host.gate] 2/3: Checking gaze-codecs EXIF & format tests..."
cargo test --locked -p gaze-codecs --no-default-features

echo "[gaze.host.gate] 3/3: Building release Gaze (apps/view) for x86_64-sex-app.json..."
for seed in "" 1 2 3 4 5 6 7 8; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p view --target x86_64-sex-app.json \
    -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/view; then break; fi
  [[ "$seed" != "8" ]] || { echo "[FAIL] gaze (view) has refused key bytes"; exit 1; }
done

BIN_BYTES="$(wc -c < target/x86_64-sex-app/release/view | tr -d ' ')"
if (( BIN_BYTES > 33554432 )); then
  echo "[FAIL] gaze binary exceeds 32 MiB program image limit (${BIN_BYTES} bytes)"
  exit 1
fi

mkdir -p iso_root/programs
cp target/x86_64-sex-app/release/view iso_root/programs/gaze

echo "[gaze.host.gate] PASS (${BIN_BYTES} bytes)"
