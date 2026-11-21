#!/usr/bin/env bash
# Rebuilds wine_crt.exe (CW4 test program, linked against Wine's ucrtbase.dll) from
# wine_crt.c with clang and lld-link. No Windows SDK: the import library is made from
# ucrtbase.def. Deterministic (/Brepro), so the gate can compare it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
OUT="${1:-wine_crt.exe}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
llvm-dlltool -m i386:x86-64 -d ucrtbase.def -l "$T/ucrtbase.lib"
clang --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-builtin -fno-stack-protector \
  -mno-avx -c wine_crt.c -o "$T/wine_crt.obj"
lld-link /nologo /entry:start /subsystem:console /nodefaultlib /base:0x140000000 /dynamicbase \
  /Brepro /out:"$T/wine_crt.exe" "$T/wine_crt.obj" "$T/ucrtbase.lib"
cp "$T/wine_crt.exe" "$OUT"
