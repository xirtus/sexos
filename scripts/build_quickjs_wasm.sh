#!/usr/bin/env bash
# Builds crates/sex-js/quickjs.wasm: QuickJS-NG (MIT) with the sex-js glue, for wasm32-wasip1.
# Needs wasi-sdk 25 (WASI_SDK=path), patch(1) and a QuickJS-NG v0.10.1 checkout (QJS=path, commit
# 3c9afc9943323ee9c7dbd123c0cd991448f4b6c2). The output is reproducible:
# sha256 cfe274380d954d585f36a03f1c030d586410a3e72cb05a4a2a70792fd5bfb62f.
# -O2, not -Oz: page scripts run about 1.25x faster for 0.4 MB more.
# CC=clang (18 or newer, with the wasm32 target) builds with that compiler and wasi-sdk's
# sysroot and builtins instead of wasi-sdk's own clang; the output then differs in bytes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
: "${WASI_SDK:?set WASI_SDK to a wasi-sdk-25.0 directory}"
: "${QJS:?set QJS to a quickjs-ng v0.10.1 checkout}"
[[ "$(git -C "$QJS" rev-parse HEAD)" == 3c9afc9943323ee9c7dbd123c0cd991448f4b6c2 ]] || { echo "[FAIL] QuickJS source must match the pinned bytecode ABI" >&2; exit 1; }
OUT="${OUT:-$ROOT/crates/sex-js/quickjs.wasm}"
CC="${CC:-$WASI_SDK/bin/clang}"
# quickjs.c gets SexOS hooks for the baseline tier and dynamic import() (held until the
# host has fetched the module graph); the checkout itself is left unchanged.
PATCHED="$(mktemp -d)"
trap 'rm -rf "$PATCHED"' EXIT
cp "$QJS/quickjs.c" "$PATCHED/quickjs.c"
patch -s -d "$PATCHED" -p1 < "$ROOT/crates/sex-js/guest/quickjs-dynamic-import.patch"
patch -s -d "$PATCHED" -p1 < "$ROOT/crates/sex-js/guest/quickjs-baseline-jit.patch"
patch -s -d "$PATCHED" -p1 < "$ROOT/crates/sex-js/guest/quickjs-promise-events.patch"
patch -s -d "$PATCHED" -p1 < "$ROOT/crates/sex-js/guest/quickjs-parser-continuations.patch"
RESOURCE="$(ls -d "$WASI_SDK"/lib/clang/* | head -1)"
"$CC" -resource-dir "$RESOURCE" --target=wasm32-wasip1 --sysroot="$WASI_SDK/share/wasi-sysroot" \
  -O2 -flto -mexec-model=reactor -D_GNU_SOURCE -DNDEBUG -I"$QJS" \
  -Wno-implicit-const-int-float-conversion -Wno-unknown-warning-option \
  "$PATCHED/quickjs.c" "$QJS/libregexp.c" "$QJS/libunicode.c" "$QJS/cutils.c" "$QJS/xsum.c" \
  "$ROOT/crates/sex-js/guest/sexjs.c" \
  -Wl,--strip-all -Wl,--gc-sections -Wl,-z,stack-size=1048576 -Wl,--initial-memory=4194304 \
  -o "$OUT"
ls -l "$OUT"
