#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds Splicy, the video editor, with FFmpeg linked in (the decoder for its viewer and
# timeline, and with the export profile, FFMPEG_RECORDING=2, the encoders for Ctrl+E).
# Output: target/splicy/x86_64-sex-app/release/splicy, installed with `fetch` or
# scripts/install_program.py.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f target/ffmpeg/lib/libavcodec.a ]] || scripts/build_ffmpeg.sh
build() {
  SEX_LAYOUT_SEED="${1:-}" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR=target/splicy cargo build --locked --release \
    -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
    --target x86_64-sex-app.json -p splicy
}
build
# The loader refuses code holding WRPKRU/XRSTOR bytes: link again in another order.
seed=0
while ! python3 scripts/key_bytes.py target/splicy/x86_64-sex-app/release/splicy; do
  seed=$((seed + 1)); [[ $seed -le 24 ]] || { echo "no clean layout found" >&2; exit 1; }
  build "$seed"
done
ls -la target/splicy/x86_64-sex-app/release/splicy
