#!/usr/bin/env bash
# Rebuilds gain_msvc.clap from gain.c with CROSSDRESS_MSVC_CRT: the test plugin with its
# memory from the C runtime through api-ms-win-crt-heap and vcruntime140.dll, the way
# MSVC-built plugins import it. No Windows SDK: import libraries from kernel32.def and
# defs/*.def. Deterministic (/Brepro), so the gate can compare it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
OUT="${1:-gain_msvc.clap}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
llvm-dlltool -m i386:x86-64 -d kernel32.def -l "$T/kernel32.lib"
llvm-dlltool -m i386:x86-64 -d defs/api-ms-win-crt-heap.def -l "$T/crt-heap.lib"
llvm-dlltool -m i386:x86-64 -d defs/vcruntime140.def -l "$T/vcruntime140.lib"
clang --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-builtin -fno-stack-protector \
  -mno-avx -DCROSSDRESS_MSVC_CRT -I ../../../vendor/clap-fixture/include -c gain.c -o "$T/gain.obj"
lld-link /nologo /dll /entry:DllMain /nodefaultlib /base:0x180000000 /dynamicbase \
  /Brepro /out:"$T/gain_msvc.clap" /implib:"$T/gain.lib" "$T/gain.obj" "$T/kernel32.lib" \
  "$T/crt-heap.lib" "$T/vcruntime140.lib"
cp "$T/gain_msvc.clap" "$OUT"
