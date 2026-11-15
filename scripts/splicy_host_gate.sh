#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Splicy window gate (S3): the editor's host tests, among them "delete a sentence on
# screen and its frames and sound are gone" (a click on the painted word, S, Delete, then
# the export walk shows no frame and lays no sound from the cut). Then the SexOS program
# builds as a PIE without refused key bytes and fits the 32 MiB program limit.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "[splicy_host_gate] cargo test -p splicy --no-default-features"
cargo test --locked -p splicy --no-default-features

echo "[splicy_host_gate] release build for x86_64-sex-app.json"
for seed in "" 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p splicy \
    --target x86_64-sex-app.json -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/splicy; then break; fi
  [[ "$seed" != "16" ]] || { echo "[FAIL] splicy binary has refused key bytes"; exit 1; }
done
size=$(stat -c %s target/x86_64-sex-app/release/splicy)
echo "[splicy_host_gate] splicy is $size bytes"
(( size < 32 * 1024 * 1024 )) || { echo "[FAIL] splicy is over the 32 MiB program limit"; exit 1; }

mkdir -p iso_root/programs
cp target/x86_64-sex-app/release/splicy iso_root/programs/splicy
echo "[splicy_host_gate] PASS"
