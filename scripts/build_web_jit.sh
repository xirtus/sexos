#!/usr/bin/env bash
# Experimental browser build; emits a separate binary, leaving the sealed ISO alone.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:?usage: build_web_jit.sh OUTPUT_PROGRAM}"
for SEED in $(seq 0 24); do
  SEX_LAYOUT_SEED="$SEED" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked \
    -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
    --manifest-path apps/web/Cargo.toml --bin web --features baseline-jit \
    --target x86_64-sex-app.json --release
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/web; then
    cp target/x86_64-sex-app/release/web "$OUT"
    exit 0
  fi
done
echo "[FAIL] experimental browser holds blocked key instructions at every layout" >&2
exit 1
