#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds dav1d (the AV1 decoder) for SexOS programs as a static archive for
# x86_64-sex-app.json, with the same flags as FFmpeg (scripts/build_ffmpeg.sh runs
# this first and links it as libdav1d). Bit depths 8 and 10 (and 12). dav1d's
# SSSE3/SSE4.1 assembly is built; its AVX2/AVX-512 code is never chosen at run time,
# because the kernel keeps XCR0 to x87 and SSE. FFmpeg asks it for one thread.
# C calls go to crates/sex-libc.
#
# Output: target/dav1d/lib/libdav1d.a, target/dav1d/include/dav1d and a pkg-config
# file (target/dav1d/lib/pkgconfig) for FFmpeg's configure.
#
# Needs clang, nasm, meson, ninja and musl's headers. dav1d is BSD-2-Clause
# (Copyright VideoLAN and dav1d authors); its licence (COPYING) is in its source.
#
#   MUSL_INCLUDE   musl's include directory (as in build_ffmpeg.sh)
#   DAV1D_SRC      a dav1d checkout (default: cloned at DAV1D_TAG)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
DAV1D_TAG=${DAV1D_TAG:-1.5.1}
OUT=$ROOT/target/dav1d
WORK=$OUT/work
mkdir -p "$WORK"
[[ -f "${MUSL_INCLUDE:-}/stdio.h" ]] || { echo "musl headers not found; set MUSL_INCLUDE" >&2; exit 1; }

if [[ -z "${DAV1D_SRC:-}" ]]; then
  DAV1D_SRC=$WORK/dav1d-$DAV1D_TAG
  [[ -d $DAV1D_SRC ]] || git clone -q --depth=1 --branch "$DAV1D_TAG" https://github.com/videolan/dav1d.git "$DAV1D_SRC"
fi

RES=$(clang -print-resource-dir)/include
CROSS=$WORK/cross.ini
cat >"$CROSS" <<EOF
[binaries]
c = 'clang'
ar = 'llvm-ar'
strip = 'llvm-strip'
nasm = 'nasm'
[built-in options]
c_args = ['--target=x86_64-unknown-none-elf', '-nostdinc', '-isystem', '$RES', '-isystem', '$MUSL_INCLUDE',
  '-msse2', '-mno-avx', '-mno-red-zone', '-fPIC', '-ffunction-sections', '-fdata-sections',
  '-fno-stack-protector', '-D_GNU_SOURCE']
c_link_args = ['--target=x86_64-unknown-none-elf', '-nostdlib', '-fuse-ld=lld', '-Wl,--unresolved-symbols=ignore-all']
[host_machine]
system = 'linux'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF

BUILD=$WORK/build
# Configure and build into BUILD; $1 is true or false for dav1d's assembly.
build() {
  rm -rf "$BUILD"
  meson setup "$BUILD" "$DAV1D_SRC" --cross-file "$CROSS" --prefix="$OUT" --libdir=lib \
    --buildtype=release --default-library=static -Dbitdepths=8,16 -Dlogging=false -Denable_asm="$1" \
    -Denable_tools=false -Denable_tests=false -Denable_examples=false -Denable_docs=false >"$WORK/configure.log"
  # As with FFmpeg, every link test passes with no C library. Turn off what sex-libc
  # does not offer (dlsym, memalign) and what is Linux's own (thread affinity, names).
  for f in DLSYM MEMALIGN PTHREAD_GETAFFINITY_NP PTHREAD_SETAFFINITY_NP PTHREAD_SETNAME_NP \
           PTHREAD_SET_NAME_NP GETAUXVAL ELF_AUX_INFO; do
    sed -i "s/^#define HAVE_$f 1$/#define HAVE_$f 0/" "$BUILD/config.h"
  done
  ninja -C "$BUILD" >"$WORK/make.log"
}

# The program loader refuses code holding WRPKRU, XRSTOR or XRSTORS bytes at any
# offset (scripts/key_bytes.py). Compile an offending C file again with other flags,
# as build_ffmpeg.sh does. Assembly cannot be changed that way: if any of dav1d's
# assembly holds them, dav1d is built again as plain C (slower, but clean).
clean() {
  local asm_hit=0 o rel cmd fixed extra
  for o in $(find "$BUILD" -name '*.o' | sort); do
    python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null && continue
    rel=${o#"$BUILD"/}
    cmd=$(cd "$BUILD" && ninja -t commands "$rel" | tail -1)
    if [[ $cmd != clang* ]]; then
      echo "[dav1d] $rel holds a refused instruction and is assembly" >&2
      python3 "$ROOT/scripts/key_bytes.py" "$o" >&2 || true
      asm_hit=1; continue
    fi
    fixed=0
    for extra in "-fno-omit-frame-pointer" "-O1 -fno-omit-frame-pointer" "-Os" "-O1"; do
      (cd "$BUILD" && eval "$cmd $extra")
      if python3 "$ROOT/scripts/key_bytes.py" "$o" >/dev/null; then echo "[dav1d] $rel: clean with $extra"; fixed=1; break; fi
    done
    [[ $fixed == 1 ]] || { echo "[dav1d] $rel: no flags give clean code" >&2; exit 1; }
  done
  return $asm_hit
}

rm -rf "$OUT/lib" "$OUT/include"
build true
if ! clean; then
  echo "[dav1d] building without assembly" >&2
  build false
  clean || { echo "[dav1d] plain C still holds refused bytes" >&2; exit 1; }
fi
# Archive the (possibly recompiled) objects again, then install.
rm -f "$BUILD/src/libdav1d.a"
ninja -C "$BUILD" >>"$WORK/make.log"
ninja -C "$BUILD" install >"$WORK/install.log"
echo "[dav1d] $(du -h "$OUT/lib/libdav1d.a" | cut -f1) -> $OUT/lib/libdav1d.a"
