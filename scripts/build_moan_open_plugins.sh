#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Builds the open LADSPA and LV2 plugins Moan tests against, from the unmodified
# sources in vendor/ (see fixtures/moan-open-plugins/README.md).
#
#   build_moan_open_plugins.sh sexos OUT   freestanding x86_64 objects for the
#                                          Moan helper (no libc, no loader)
#   build_moan_open_plugins.sh host OUT    the same sources as host shared
#                                          libraries, for DSP comparison tests
#
# OUT receives t5_so_lowpass.so, t5_so_highpass.so, eg-amp.lv2/ and
# blop-lp4pole.lv2/ (bundle folders with manifest.ttl, data and library).
set -eu
mode=${1:?sexos or host}
out=${2:?output folder}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
: "${CLANG:=clang}"
: "${LD_LLD:=ld.lld}"
: "${CC:=cc}"
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)
work="$out/.work"
rm -rf "$work"
mkdir -p "$work"
fixture="$root/fixtures/moan-open-plugins"

case "$mode" in
sexos)
    resource=$("$CLANG" --print-resource-dir)/include
    cflags="--target=x86_64-unknown-none-elf -std=gnu11 -fPIC -O2 -DNDEBUG -ffreestanding
        -fno-stack-protector -mno-red-zone -msse2 -mno-avx -fno-builtin-malloc
        -fcx-limited-range -ffunction-sections -fdata-sections
        -nostdinc -isystem $fixture/include -isystem $resource"
    ;;
host)
    cflags="-std=gnu11 -fPIC -O2 -DNDEBUG -fcx-limited-range"
    ;;
*)
    echo "mode must be sexos or host" >&2
    exit 2
    ;;
esac
includes="-I $root/vendor/ladspa -I $root/vendor/lv2-headers"

# link NAME ENTRY OBJECTS... -> $out/NAME
link() {
    lib=$1
    entry=$2
    shift 2
    if [ "$mode" = sexos ]; then
        printf '{ global: %s; local: *; };\n' "$entry" > "$work/exports"
        "$LD_LLD" -shared -Bsymbolic --gc-sections --hash-style=sysv -z max-page-size=4096 \
            --version-script="$work/exports" -o "$out/$lib" "$@"
    else
        # shellcheck disable=SC2086
        $CC -shared -o "$out/$lib" "$@" -lm
    fi
}
compile() {
    c_file=$1
    o_file=$2
    shift 2
    if [ "$mode" = sexos ]; then
        # shellcheck disable=SC2086
        "$CLANG" $cflags $includes "$@" -c "$c_file" -o "$o_file"
    else
        # shellcheck disable=SC2086
        $CC $cflags $includes "$@" -c "$c_file" -o "$o_file"
    fi
}

# LADSPA: T5 filters. Copy beside the SexOS helpers.h so it replaces upstream's.
t5="$work/t5"
mkdir -p "$t5"
cp "$root"/vendor/ladspa-t5/*.c "$root"/vendor/ladspa-t5/*.h "$t5"/
cp "$fixture/ladspa-t5/helpers.h" "$t5/helpers.h"
for p in so_lowpass so_highpass; do
    compile "$t5/t5_$p.c" "$t5/$p.o"
    link "t5_$p.so" ladspa_descriptor "$t5/$p.o"
done

# LV2: eg-amp (LV2 examples) and lp4pole (blop-lv2).
amp="$out/eg-amp.lv2"
mkdir -p "$amp"
compile "$root/vendor/lv2-plugins/eg-amp.lv2/amp.c" "$work/amp.o"
link "eg-amp.lv2/amp.so" lv2_descriptor "$work/amp.o"
cp "$root/vendor/lv2-plugins/eg-amp.lv2/manifest.ttl" "$root/vendor/lv2-plugins/eg-amp.lv2/amp.ttl" "$amp/"

lp="$out/blop-lp4pole.lv2"
src="$root/vendor/lv2-plugins/blop-lp4pole.lv2"
mkdir -p "$lp"
for c in lp4pole lp4pole_filter; do
    compile "$src/$c.c" "$work/$c.o" -I "$src/include"
done
link "blop-lp4pole.lv2/lp4pole.so" lv2_descriptor "$work/lp4pole.o" "$work/lp4pole_filter.o"
cp "$src/manifest.ttl" "$src/lp4pole.ttl" "$lp/"

if [ "$mode" = sexos ]; then
    # Same rule as the program loader: no WRPKRU/XRSTOR(S) bytes in code.
    python3 "$root/scripts/key_bytes.py" "$out"/t5_so_lowpass.so "$out"/t5_so_highpass.so \
        "$amp/amp.so" "$lp/lp4pole.so"
fi
rm -rf "$work"
echo "[moan.open-plugins] mode=$mode out=$out"
