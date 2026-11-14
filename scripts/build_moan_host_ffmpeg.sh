#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Native host qualification uses the same pinned FFmpeg7.1.2 + Shine encoder as
# the guest, not an unrelated system encoder with the same filename extension.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
OUT=$ROOT/target/moan-host-ffmpeg
PIN=f893221c8d89cb798b829bebe71d55e1a3f242fd
SRC=${MOAN_FFMPEG_SRC:-$ROOT/target/ffmpeg/work/source}
mkdir -p "$OUT/work"
if [[ ! -d "$SRC/.git" ]]; then
  git clone -q --depth=1 --branch n7.1.2 https://github.com/FFmpeg/FFmpeg.git "$SRC"
fi
[[ $(git -C "$SRC" rev-parse HEAD) == "$PIN" ]] || { echo 'Host FFmpeg source must be pinned n7.1.2' >&2; exit 1; }
SHINE_HOST=1 SHINE_OUT="$ROOT/target/moan-host-shine" bash scripts/build_shine.sh
SHINE=$ROOT/target/moan-host-shine
BUILD=$OUT/work/build
mkdir -p "$BUILD"
cd "$BUILD"
PKG_CONFIG_LIBDIR="$SHINE/lib/pkgconfig" PKG_CONFIG_PATH= "$SRC/configure" --prefix="$OUT" \
 --cc=clang --ar=llvm-ar --ranlib=llvm-ranlib --nm=llvm-nm --pkg-config=pkg-config --pkg-config-flags=--static \
 --disable-everything --disable-autodetect --disable-programs --disable-doc --disable-debug \
 --disable-network --disable-avdevice --disable-avfilter --disable-swscale --disable-swresample \
 --disable-pthreads --enable-pic --enable-static --disable-shared --enable-libshine \
 --enable-encoder=pcm_s16le,flac,libshine --enable-muxer=wav,flac,mp3 > configure.log
make -j"$(getconf _NPROCESSORS_ONLN)" > make.log
make install > install.log
cp "$SHINE/lib/libshine.a" "$OUT/lib/"
mkdir -p "$OUT/share/licenses/shine"
cp "$SHINE/share/licenses/shine/"* "$OUT/share/licenses/shine/"
printf '%s\n' "$PIN" > "$OUT/source-pin"
printf '%s\n' '1' > "$OUT/sexos-audio-export-v1"
echo "[moan.host.ffmpeg] $PIN -> $OUT"
