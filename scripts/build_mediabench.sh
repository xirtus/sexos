#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds mediabench (apps/mediabench: FFmpeg decode/encode speed, SSE only vs
# AVX/AVX2/FMA3) with FFmpeg linked in: FFmpeg first if it is not built yet, then the
# program as the trace builds programs (position-independent).
# Output: target/mediabench/x86_64-sex-app/release/mediabench. Not in the boot image;
# scripts/program_mediabench_gate.py installs it.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f target/ffmpeg/lib/libavcodec.a ]] || scripts/build_ffmpeg.sh
RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR=target/mediabench cargo build --release \
  -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
  --target x86_64-sex-app.json --manifest-path apps/mediabench/Cargo.toml
# The loader refuses code holding WRPKRU/XRSTOR bytes (scripts/key_bytes.py); link
# again with the read-only data in another order until none are left.
seed=0
while ! python3 scripts/key_bytes.py target/mediabench/x86_64-sex-app/release/mediabench; do
  seed=$((seed + 1)); [[ $seed -le 24 ]] || { echo "no clean layout found" >&2; exit 1; }
  SEX_LAYOUT_SEED=$seed RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR=target/mediabench cargo build --release \
    -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
    --target x86_64-sex-app.json --manifest-path apps/mediabench/Cargo.toml
done
ls -la target/mediabench/x86_64-sex-app/release/mediabench
