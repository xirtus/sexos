#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds libaom's AV1 encoder for SexOS programs as a static archive for
# x86_64-sex-app.json, with the same flags as FFmpeg (scripts/build_ffmpeg.sh runs
# this first for the export profile). Splicy exports MP4 with AV1 + Opus through it.
# One thread; SSE2..SSE4.2 assembly and intrinsics, never AVX, whose state the kernel
# does not keep. C calls go to crates/sex-libc.
#
# Output: target/libaom/lib/libaom.a, target/libaom/include/aom and a pkg-config file.
#
# Needs clang, nasm, cmake, make and musl's headers. libaom is BSD-2-Clause with the
# Alliance for Open Media patent licence; both are in its source (LICENSE, PATENTS).
#
#   MUSL_INCLUDE   musl's include directory (as in build_ffmpeg.sh)
#   LIBAOM_SRC     a libaom source tree (default: the release tarball, checked by hash)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
LIBAOM_VERSION=${LIBAOM_VERSION:-3.13.1}
LIBAOM_SHA256=${LIBAOM_SHA256:-19e45a5a7192d690565229983dad900e76b513a02306c12053fb9a262cbeca7d}
OUT=$ROOT/target/libaom
WORK=$OUT/work
mkdir -p "$WORK"
[[ -f "${MUSL_INCLUDE:-}/stdio.h" ]] || { echo "musl headers not found; set MUSL_INCLUDE" >&2; exit 1; }

if [[ -z "${LIBAOM_SRC:-}" ]]; then
  LIBAOM_SRC=$WORK/libaom-$LIBAOM_VERSION
  if [[ ! -d $LIBAOM_SRC ]]; then
    curl -sSfL -o "$WORK/libaom.tar.gz" "https://storage.googleapis.com/aom-releases/libaom-$LIBAOM_VERSION.tar.gz"
    echo "$LIBAOM_SHA256  $WORK/libaom.tar.gz" | sha256sum -c --quiet
    tar xzf "$WORK/libaom.tar.gz" -C "$WORK"
  fi
fi

RES=$(clang -print-resource-dir)/include
CFLAGS="--target=x86_64-unknown-none-elf -nostdinc -isystem $RES -isystem $MUSL_INCLUDE \
 -msse2 -mno-avx -mno-red-zone -fPIC -ffunction-sections -fdata-sections -fno-stack-protector -D_GNU_SOURCE"

BUILD=$WORK/build
rm -rf "$BUILD" && mkdir -p "$BUILD" && cd "$BUILD"
cmake "$LIBAOM_SRC" -G "Unix Makefiles" -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$OUT" \
  -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=x86_64 -DAOM_TARGET_CPU=x86_64 \
  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ -DCMAKE_AR="$(command -v llvm-ar)" \
  -DCMAKE_RANLIB="$(command -v llvm-ranlib)" -DCMAKE_ASM_NASM_COMPILER="$(command -v nasm)"  \
  -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF \
  -DCONFIG_AV1_DECODER=0 -DCONFIG_MULTITHREAD=0 -DCONFIG_RUNTIME_CPU_DETECT=1 \
  -DENABLE_AVX=0 -DENABLE_AVX2=0 -DENABLE_AVX512=0 -DENABLE_NEON=0 \
  -DENABLE_DOCS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTS=0 -DENABLE_TESTDATA=0 -DENABLE_TOOLS=0 \
  -DCONFIG_TUNE_VMAF=0 -DCONFIG_TUNE_BUTTERAUGLI=0 -DCONFIG_DENOISE=1 \
  -DCONFIG_REALTIME_ONLY=0 -DCONFIG_INSPECTION=0 -DCONFIG_WEBM_IO=0 -DCONFIG_LIBYUV=0 \
  >configure.log
make -j"$(nproc)" aom >make.log

# As in build_ffmpeg.sh: the loader refuses code holding WRPKRU/XRSTOR(S) bytes.
bad=$(for o in $(find . -name '*.o' | sort); do python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null || echo "$o"; done)
if [[ -n $bad ]]; then
  echo "[libaom] objects holding refused instructions, rebuilt with a frame pointer:" $bad >&2
  for o in $bad; do rm -f "$o"; done
  make -j"$(nproc)" aom CFLAGS="$CFLAGS -fno-omit-frame-pointer" >>make.log
  for o in $bad; do python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null || { echo "[libaom] $o still refused" >&2; exit 1; }; done
fi
make install >install.log
sed -i 's/ -lm -lpthread$//; s/ -lpthread$//' "$OUT/lib/pkgconfig/aom.pc"
echo "[libaom] libaom.a -> $OUT"
