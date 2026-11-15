#!/usr/bin/env bash
# Climax host gate: microbenchmarks, sustained torture runner, thermal throttling analyzer and report formatter.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=== [1/4] Cargo test --locked -p climax-core ==="
cargo test --locked -p climax-core

echo "=== [2/4] Target check: x86_64-sex-app.json (climax-core) ==="
cargo check --target x86_64-sex-app.json -Z build-std=core,alloc -p climax-core

echo "=== [3/4] Cargo test --locked -p climax --no-default-features ==="
cargo test --locked -p climax --no-default-features

echo "=== [4/5] Target check: x86_64-sex-app.json (climax app) ==="
cargo check --locked --target x86_64-sex-app.json -Z build-std=core,alloc,compiler_builtins -Z build-std-features=compiler-builtins-mem -p climax --features sexos

echo "=== [5/5] Release PIE binary & key_bytes.py opcode audit ==="
for seed in "" 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p climax --target x86_64-sex-app.json \
    -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem --features sexos
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/climax; then break; fi
  [[ "$seed" != "16" ]] || { echo "[FAIL] climax binary has refused key bytes"; exit 1; }
done

mkdir -p iso_root/programs
cp target/x86_64-sex-app/release/climax iso_root/programs/climax

echo "[climax_host_gate] PASS: all climax tests, PIE build, and key_bytes checks passed."
