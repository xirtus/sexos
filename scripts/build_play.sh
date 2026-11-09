#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds Play, the video player, with FFmpeg linked in: FFmpeg first if it is not
# built yet, then the program as the trace builds programs (position-independent).
# Output: target/play/x86_64-sex-app/release/play. Install it with `fetch` or
# scripts/install_program.py; it is not in the boot image yet.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f target/ffmpeg/lib/libavcodec.a ]] || scripts/build_ffmpeg.sh
RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR=target/play cargo build --release \
  -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
  --target x86_64-sex-app.json --manifest-path apps/play/Cargo.toml
ls -la target/play/x86_64-sex-app/release/play
