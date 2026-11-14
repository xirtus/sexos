#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds libvpx's VP9 encoder for SexOS programs as a static archive for
# x86_64-sex-app.json, with the same flags as FFmpeg (scripts/build_ffmpeg.sh runs
# this first for the export profile and links it as libvpx). Splicy exports WebM with
# it. One thread; SSE2..SSE4.1 assembly, never AVX, whose state the kernel does not keep.
# C calls go to crates/sex-libc.
#
# Output: target/libvpx/lib/libvpx.a, target/libvpx/include/vpx and a pkg-config file.
#
# Needs clang, nasm, make and musl's headers. libvpx is BSD-3-Clause (Copyright the
# WebM project authors) with Google's patent grant; both are in its source (LICENSE,
# PATENTS).
#
#   MUSL_INCLUDE   musl's include directory (as in build_ffmpeg.sh)
#   LIBVPX_SRC     a libvpx checkout (default: cloned at LIBVPX_TAG)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
LIBVPX_TAG=${LIBVPX_TAG:-v1.15.2}
OUT=$ROOT/target/libvpx
WORK=$OUT/work
mkdir -p "$WORK"
[[ -f "${MUSL_INCLUDE:-}/stdio.h" ]] || { echo "musl headers not found; set MUSL_INCLUDE" >&2; exit 1; }

if [[ -z "${LIBVPX_SRC:-}" ]]; then
  LIBVPX_SRC=$WORK/libvpx-$LIBVPX_TAG
  [[ -d $LIBVPX_SRC ]] || git clone -q --depth=1 --branch "$LIBVPX_TAG" https://github.com/webmproject/libvpx.git "$LIBVPX_SRC"
fi

RES=$(clang -print-resource-dir)/include
CFLAGS="--target=x86_64-unknown-none-elf -nostdinc -isystem $RES -isystem $MUSL_INCLUDE \
 -msse2 -mno-avx -mno-red-zone -fPIC -ffunction-sections -fdata-sections -fno-stack-protector -D_GNU_SOURCE"

BUILD=$WORK/build
rm -rf "$BUILD" && mkdir -p "$BUILD" && cd "$BUILD"
CC=clang AR=llvm-ar LD=clang AS=nasm STRIP=llvm-strip CFLAGS="$CFLAGS" CXXFLAGS="$CFLAGS" \
LDFLAGS="--target=x86_64-unknown-none-elf -nostdlib -fuse-ld=lld -Wl,--unresolved-symbols=ignore-all" \
"$LIBVPX_SRC/configure" --prefix="$OUT" --libdir="$OUT/lib" --target=x86_64-linux-gcc --as=nasm \
  --disable-avx --disable-avx2 --disable-avx512 \
  --disable-multithread --disable-vp8 --enable-vp9 --disable-vp9-decoder \
  --disable-examples --disable-tools --disable-docs --disable-unit-tests --disable-install-bins \
  --disable-install-srcs --enable-pic --enable-static --disable-shared --disable-debug \
  --disable-webm-io --disable-libyuv >configure.log

# Only the codec library: the separate rate-control library is C++.
make -j"$(nproc)" libvpx.a >make.log

# As in build_ffmpeg.sh: the loader refuses code holding WRPKRU/XRSTOR(S) bytes.
for o in $(find . -name '*.o' | sort); do
  python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null && continue
  echo "[libvpx] $o holds a refused instruction; rebuilding it with a frame pointer" >&2
  cmd=$(make -B -n V=1 "${o#./}" 2>/dev/null | grep -m1 -E '^(clang|nasm) ' || true)
  [[ $cmd == clang* ]] || { python3 "$ROOT/scripts/key_bytes.py" "$o" >&2; exit 1; }
  fixed=0
  for extra in "-fno-omit-frame-pointer" "-O1 -fno-omit-frame-pointer" "-Os" "-O1"; do
    eval "$cmd $extra"
    if python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null; then fixed=1; break; fi
  done
  [[ $fixed == 1 ]] || { echo "[libvpx] $o: no flags give clean code" >&2; exit 1; }
done
make -j"$(nproc)" libvpx.a >>make.log
rm -rf "$OUT/lib" "$OUT/include" && mkdir -p "$OUT/lib/pkgconfig" "$OUT/include/vpx"
cp libvpx.a "$OUT/lib/"
cp "$LIBVPX_SRC"/vpx/*.h "$OUT/include/vpx/"
cat >"$OUT/lib/pkgconfig/vpx.pc" <<PC
prefix=$OUT
includedir=\${prefix}/include
libdir=\${prefix}/lib
Name: vpx
Description: WebM Project VPx codec implementation
Version: ${LIBVPX_TAG#v}
Cflags: -I\${includedir}
Libs: -L\${libdir} -lvpx
PC
echo "[libvpx] $(ls "$OUT"/lib/*.a | xargs -n1 basename) -> $OUT"
