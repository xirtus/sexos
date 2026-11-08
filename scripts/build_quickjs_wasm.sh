#!/usr/bin/env bash
# Builds crates/sex-js/quickjs.wasm: QuickJS-NG (MIT) with the sex-js glue, for wasm32-wasip1.
# Needs wasi-sdk 25 (WASI_SDK=path) and a QuickJS-NG v0.10.1 checkout (QJS=path, commit
# 3c9afc9943323ee9c7dbd123c0cd991448f4b6c2). The output is reproducible:
# sha256 3c2a3f1411de25f946c9190c9129374ca63cd543c29704b2175976ca7af8792d.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
: "${WASI_SDK:?set WASI_SDK to a wasi-sdk-25.0 directory}"
: "${QJS:?set QJS to a quickjs-ng v0.10.1 checkout}"
OUT="${OUT:-$ROOT/crates/sex-js/quickjs.wasm}"
"$WASI_SDK/bin/clang" --target=wasm32-wasip1 --sysroot="$WASI_SDK/share/wasi-sysroot" \
  -Oz -flto -mexec-model=reactor -D_GNU_SOURCE -DNDEBUG -I"$QJS" \
  -Wno-implicit-const-int-float-conversion -Wno-unknown-warning-option \
  "$QJS/quickjs.c" "$QJS/libregexp.c" "$QJS/libunicode.c" "$QJS/cutils.c" "$QJS/xsum.c" \
  "$ROOT/crates/sex-js/guest/sexjs.c" \
  -Wl,--strip-all -Wl,--gc-sections -Wl,-z,stack-size=1048576 -Wl,--initial-memory=4194304 \
  -o "$OUT"
ls -l "$OUT"
