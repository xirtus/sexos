#!/usr/bin/env bash
# Build the pinned recorder image from explicitly provisioned FFmpeg inputs.
# This script never changes a decoder prefix into an encoder prefix. A decoder
# build remains useful for the actual unavailable-before-capture gate.
set -euo pipefail
cd "$(dirname "$0")/.."
RECORD_PREFIX="${SEXOS_FFMPEG:-$PWD/target/ffmpeg}"
RECORD_TARGET_DIR="${SEXOS_RECORD_TARGET_DIR:-target/record}"
[[ -f "$RECORD_PREFIX/lib/libavcodec.a" ]] || { echo 'Provide a built FFmpeg prefix through SEXOS_FFMPEG.' >&2; exit 1; }
export SEXOS_FFMPEG="$RECORD_PREFIX"
export SEXOS_RECORDER=1
build_record() {
  SEX_LAYOUT_SEED="$1" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" CARGO_TARGET_DIR="$RECORD_TARGET_DIR" cargo build --locked --release \
    -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem \
    --target x86_64-sex-app.json --manifest-path apps/record/Cargo.toml
}
build_record 0
RECORD_ELF="$RECORD_TARGET_DIR/x86_64-sex-app/release/record"
RECORD_SEED=0
while ! python3 scripts/key_bytes.py "$RECORD_ELF"; do
  RECORD_SEED=$((RECORD_SEED+1))
  [[ $RECORD_SEED -le 24 ]] || { echo 'No clean recorder layout found.' >&2; exit 1; }
  build_record "$RECORD_SEED"
done
python3 - "$RECORD_PREFIX" "$RECORD_ELF" <<'PY'
import hashlib,json,sys
from pathlib import Path
prefix,elf=map(Path,sys.argv[1:])
marker=prefix/'sexos-recording-v1'
print(json.dumps(dict(program=str(elf.resolve()),sha256=hashlib.sha256(elf.read_bytes()).hexdigest(),ffmpeg_prefix=str(prefix.resolve()),encoder_profile=marker.is_file() and marker.read_bytes()==b'1\n',authority_assigned=False),sort_keys=True))
PY
