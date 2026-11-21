#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds x264 (the H.264 encoder) for SexOS programs as a static archive for
# x86_64-sex-app.json, with the same flags as FFmpeg (scripts/build_ffmpeg.sh runs this
# first for the export profile). Splicy exports MP4 with H.264 + AAC through it.
# One thread; x264 checks CPUID and XCR0 at run time, so its AVX/AVX2/FMA code runs
# where the CPU has it (the kernel saves AVX state since #408) and SSE code elsewhere. C calls go to crates/sex-libc.
#
# x264 is GPL-2.0-or-later, so the export profile builds FFmpeg with --enable-gpl;
# SexOS is GPL-3.0. H.264 itself is patent-licensed in some countries.
#
# Output: target/x264/lib/libx264.a, target/x264/include and a pkg-config file.
#
# Needs clang, nasm, make and musl's headers.
#
#   MUSL_INCLUDE   musl's include directory (as in build_ffmpeg.sh)
#   X264_SRC       an x264 source tree (default: the archive of X264_COMMIT from
#                  code.videolan.org, checked to be that commit)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
X264_COMMIT=${X264_COMMIT:-b35605ace3ddf7c1a5d67a2eb553f034aef41d55}
OUT=$ROOT/target/x264
WORK=$OUT/work
mkdir -p "$WORK"
[[ -f "${MUSL_INCLUDE:-}/stdio.h" ]] || { echo "musl headers not found; set MUSL_INCLUDE" >&2; exit 1; }

if [[ -z "${X264_SRC:-}" ]]; then
  X264_SRC=$WORK/x264-$X264_COMMIT
  if [[ ! -d $X264_SRC ]]; then
    curl -sSfL -o "$WORK/x264.tar.bz2" \
      "https://code.videolan.org/videolan/x264/-/archive/$X264_COMMIT/x264-$X264_COMMIT.tar.bz2"
    [[ "$(bzcat "$WORK/x264.tar.bz2" | git get-tar-commit-id)" == "$X264_COMMIT" ]] \
      || { echo "[x264] the archive is not commit $X264_COMMIT" >&2; exit 1; }
    tar xjf "$WORK/x264.tar.bz2" -C "$WORK"
  fi
fi

RES=$(clang -print-resource-dir)/include
CFLAGS="--target=x86_64-unknown-none-elf -nostdinc -isystem $RES -isystem $MUSL_INCLUDE \
 -msse2 -mno-avx -mno-red-zone -fPIC -ffunction-sections -fdata-sections -fno-stack-protector -D_GNU_SOURCE"

BUILD=$WORK/build
rm -rf "$BUILD" && mkdir -p "$BUILD" && cd "$BUILD"
build() {
  CC=clang AR=llvm-ar RANLIB=llvm-ranlib STRIP=llvm-strip AS=nasm \
  "$X264_SRC/configure" --prefix="$OUT" --host=x86_64-linux-gnu --enable-static --enable-pic \
    --disable-cli --disable-opencl --disable-thread --disable-win32thread \
    --bit-depth=8 --chroma-format=420 --disable-avs --disable-swscale --disable-lavf \
    --disable-ffms --disable-gpac --disable-lsmash \
    --extra-cflags="$CFLAGS $1" \
    --extra-ldflags="--target=x86_64-unknown-none-elf -nostdlib -fuse-ld=lld -Wl,--unresolved-symbols=ignore-all" \
    >configure.log
  # Configure's link tests pass with no C library. sex-libc has no memalign or madvise:
  # use x264's own aligned malloc.
  sed -i 's/^#define HAVE_MALLOC_H 1$/#define HAVE_MALLOC_H 0/; s/^#define HAVE_THP 1$/#define HAVE_THP 0/' config.h
  make clean >/dev/null 2>&1 || true
  make -j"$(nproc)" lib-static >make.log
}
build ""

# As in build_ffmpeg.sh: the loader refuses code holding WRPKRU/XRSTOR(S) bytes. A frame
# pointer changes register choice; rebuild everything with one if any object is refused.
refused() { for o in $(find . -name '*.o' | sort); do python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null || echo "$o"; done; }
bad=$(refused)
if [[ -n $bad ]]; then
  echo "[x264] refused bytes in:" $bad "; rebuilding with a frame pointer" >&2
  build "-fno-omit-frame-pointer"
  bad=$(refused)
  [[ -z $bad ]] || { echo "[x264] still refused:" $bad >&2; exit 1; }
fi
make install-lib-static >install.log
echo "[x264] libx264.a -> $OUT"
