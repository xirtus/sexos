#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds Music, the music player, with FFmpeg linked in: FFmpeg first if it is not
# built yet, then the program as the trace builds programs (position-independent).
# Output: target/music/x86_64-sex-app/release/music. Install it with `fetch` or
# scripts/install_program.py. The boot image builds its own copy (stage build_music).
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f target/ffmpeg/lib/libavcodec.a ]] || scripts/build_ffmpeg.sh
RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR=target/music cargo build --release \
  -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
  --target x86_64-sex-app.json --manifest-path apps/music/Cargo.toml
# The loader refuses code holding WRPKRU/XRSTOR bytes (scripts/key_bytes.py); link
# again with the read-only data in another order until none are left.
seed=0
while ! python3 scripts/key_bytes.py target/music/x86_64-sex-app/release/music; do
  seed=$((seed + 1)); [[ $seed -le 24 ]] || { echo "no clean layout found" >&2; exit 1; }
  SEX_LAYOUT_SEED=$seed RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR=target/music cargo build --release \
    -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
    --target x86_64-sex-app.json --manifest-path apps/music/Cargo.toml
done
ls -la target/music/x86_64-sex-app/release/music
