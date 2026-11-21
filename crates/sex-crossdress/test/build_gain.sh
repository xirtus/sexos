#!/usr/bin/env bash
# Rebuilds gain.clap (checked in, include_bytes! in src/lib.rs) from gain.c: a 64-bit
# Windows CLAP plugin DLL, built with clang and lld-link against the upstream CLAP headers
# (vendor/clap-fixture/include). No Windows SDK and no C runtime: the KERNEL32 import
# library comes from kernel32.def. Deterministic (/Brepro), so the gate can compare it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
OUT="${1:-gain.clap}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
llvm-dlltool -m i386:x86-64 -d kernel32.def -l "$T/kernel32.lib"
clang --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-builtin -fno-stack-protector \
  -mno-avx -I ../../../vendor/clap-fixture/include -c gain.c -o "$T/gain.obj"
lld-link /nologo /dll /entry:DllMain /nodefaultlib /base:0x180000000 /dynamicbase \
  /Brepro /out:"$T/gain.clap" /implib:"$T/gain.lib" "$T/gain.obj" "$T/kernel32.lib"
cp "$T/gain.clap" "$OUT"
