#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds LLVM's C++ standard library (libc++ and libc++abi, Apache-2.0 with LLVM
# exception) as static archives for SexOS programs on x86_64-sex-app.json, over
# musl's headers and crates/sex-libc. No exceptions (V8 and Chromium build without
# them), no filesystem or random device: a program reaches files and entropy through
# its own grants, not through the C++ library.
#
# C++ is for ported engines (V8 first), each kept in its own organ. SexOS itself, its
# servers and its SDK stay Rust.
#
# Output: target/libcxx/lib/libc++.a, libc++abi.a and target/libcxx/include/c++/v1.
# Programs find them through SEXOS_LIBCXX (default: that directory).
#
#   MUSL_INCLUDE   musl's include directory (default: found below)
#   LLVM_SRC       an llvm-project checkout (default: cloned at LLVM_TAG, runtimes only)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
LLVM_TAG=${LLVM_TAG:-llvmorg-18.1.8}
OUT=$ROOT/target/libcxx
WORK=$OUT/work
mkdir -p "$WORK"

if [[ -z "${MUSL_INCLUDE:-}" ]]; then
  for d in /usr/lib/musl/include /usr/include/x86_64-linux-musl /usr/local/musl/include; do
    [[ -f $d/stdio.h ]] && MUSL_INCLUDE=$d && break
  done
fi
[[ -f "${MUSL_INCLUDE:-}/stdio.h" ]] || { echo "musl headers not found; set MUSL_INCLUDE" >&2; exit 1; }

LLVM_SRC=${LLVM_SRC:-$WORK/llvm-project}
if [[ ! -d $LLVM_SRC/runtimes ]]; then
  rm -rf "$LLVM_SRC"
  git clone -q --depth=1 --filter=blob:none --sparse --branch "$LLVM_TAG" https://github.com/llvm/llvm-project.git "$LLVM_SRC"
  git -C "$LLVM_SRC" sparse-checkout set libcxx libcxxabi libunwind runtimes cmake llvm/cmake llvm/utils/llvm-lit
fi

# musl's headers as-is: programs on this target have x87 and SSE.
INC=$OUT/include-c
rm -rf "$INC" && mkdir -p "$INC"
cp -rL "$MUSL_INCLUDE"/. "$INC"/ && chmod -R u+w "$INC"

RES=$(clang -print-resource-dir)/include
FLAGS="--target=x86_64-unknown-none-elf -nostdinc -isystem $RES -isystem $INC \
 -fPIC -mno-red-zone -fno-stack-protector -ffunction-sections -fdata-sections -D_GNU_SOURCE"

BUILD=$WORK/build
rm -rf "$BUILD"
cmake -G Ninja -S "$LLVM_SRC/runtimes" -B "$BUILD" \
  -DLLVM_ENABLE_RUNTIMES="libcxx;libcxxabi" -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_SYSTEM_NAME=Generic -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
  -DCMAKE_C_FLAGS="$FLAGS" -DCMAKE_CXX_FLAGS="$FLAGS" -DCMAKE_INSTALL_PREFIX="$OUT" \
  -DLIBCXX_ENABLE_SHARED=OFF -DLIBCXXABI_ENABLE_SHARED=OFF -DLIBCXX_HAS_MUSL_LIBC=ON \
  -DLIBCXX_ENABLE_EXCEPTIONS=OFF -DLIBCXXABI_ENABLE_EXCEPTIONS=OFF \
  -DLIBCXX_ENABLE_FILESYSTEM=OFF -DLIBCXX_ENABLE_RANDOM_DEVICE=OFF \
  -DLIBCXX_ENABLE_TIME_ZONE_DATABASE=OFF \
  -DLIBCXX_HAS_PTHREAD_API=ON -DLIBCXXABI_HAS_PTHREAD_API=ON \
  -DLIBCXX_INCLUDE_TESTS=OFF -DLIBCXX_INCLUDE_BENCHMARKS=OFF -DLIBCXXABI_INCLUDE_TESTS=OFF \
  -DLIBCXXABI_USE_LLVM_UNWINDER=OFF -DLIBCXX_CXX_ABI=libcxxabi -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=ON
ninja -C "$BUILD" install-cxx install-cxxabi
echo "libc++ for SexOS programs: $OUT/lib"
