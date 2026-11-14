#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Shine3.1.1, fixed-point MP3 encoder. Upstream COPYING (LGPL2) retained verbatim.
# No driver/kernel bridge: this archive executes inside the exporting app organ.
# SHINE_HOST=1 builds a native host oracle; otherwise musl headers are required.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
PIN=97f188efc3dc90315b79a2af1f477c0c18d85e82
OUT=${SHINE_OUT:-$ROOT/target/shine}
SRC=${SHINE_SRC:-$OUT/work/source}
mkdir -p "$OUT/work" "$OUT/lib" "$OUT/include/shine" "$OUT/lib/pkgconfig" "$OUT/share/licenses/shine"
if [[ ! -d "$SRC/.git" ]]; then
  git clone -q --depth=1 --branch 3.1.1 https://github.com/toots/shine.git "$SRC"
fi
[[ $(git -C "$SRC" rev-parse HEAD) == "$PIN" ]] || { echo 'Shine source is not the pinned3.1.1 commit' >&2; exit 1; }
MATH=-lm
FLAGS=(-O2 -fPIC -ffunction-sections -fdata-sections -fno-strict-aliasing -fwrapv -fsigned-char -I"$SRC/src/lib")
if [[ ${SHINE_HOST:-0} != 1 ]]; then
  MATH="" # guest math symbols come from sex-libc; there is no libm archive.
  [[ -f ${MUSL_INCLUDE:-}/stdio.h ]] || { echo 'Set MUSL_INCLUDE to musl headers' >&2; exit 1; }
  RES=$(clang -print-resource-dir)/include
  FLAGS+=(--target=x86_64-unknown-none-elf -nostdinc -isystem "$RES" -isystem "$MUSL_INCLUDE" -msse2 -mno-avx -mno-red-zone -fno-stack-protector)
fi
objects=()
for name in bitstream huffman l3bitstream l3loop l3mdct l3subband layer3 reservoir tables; do
  object=$OUT/work/$name.o
  clang "${FLAGS[@]}" -c "$SRC/src/lib/$name.c" -o "$object"
  if [[ ${SHINE_HOST:-0} != 1 ]] && ! python3 scripts/key_bytes.py "$object" >/dev/null; then
    clean=0
    for extra in -fno-omit-frame-pointer -O1 -Os; do
      clang "${FLAGS[@]}" "$extra" -c "$SRC/src/lib/$name.c" -o "$object"
      if python3 scripts/key_bytes.py "$object" >/dev/null; then clean=1;break;fi
    done
    [[ $clean == 1 ]] || { echo "Shine $name has forbidden instruction bytes" >&2; exit 1; }
  fi
  objects+=("$object")
done
rm -f "$OUT/lib/libshine.a"
llvm-ar crs "$OUT/lib/libshine.a" "${objects[@]}"
cp "$SRC/src/lib/layer3.h" "$OUT/include/shine/"
cp "$SRC/COPYING" "$OUT/share/licenses/shine/COPYING.upstream-LGPL2"
cp "$ROOT/LICENSE" "$OUT/share/licenses/shine/COPYING.GPL3"
cat > "$OUT/share/licenses/shine/NOTICE" <<NOTICE
Shine3.1.1, source https://github.com/toots/shine
Commit $PIN. No upstream source changes.
For this built copy, SexOS selects ordinary GNU GPL version3 using upstream
COPYING section3, which expressly permits a version newer than GPLversion2.
The applicable license is COPYING.GPL3; COPYING.upstream-LGPL2 is preserved
verbatim as the original permission and source provenance.
NOTICE
cat > "$OUT/lib/pkgconfig/shine.pc" <<PC
prefix=$OUT
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: shine
Description: pinned3.1.1 fixed-point MP3 encoding
Version: 3.1.1
Libs: -L\${libdir} -lshine $MATH
Cflags: -I\${includedir}
PC
printf '%s\n' "$PIN" > "$OUT/source-pin"
echo "[shine] $PIN -> $OUT/lib/libshine.a"
