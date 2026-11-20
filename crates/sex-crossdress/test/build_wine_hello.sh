#!/usr/bin/env bash
# Rebuilds wine_hello.exe (CW3 test program, run on Wine's kernel32) from wine_hello.c with clang and
# lld-link. No Windows SDK and no C runtime: the KERNEL32 import library is made from
# kernel32.def. The output is deterministic (/Brepro), so the gate can compare it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
OUT="${1:-wine_hello.exe}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
llvm-dlltool -m i386:x86-64 -d kernel32.def -l "$T/kernel32.lib"
clang --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-builtin -fno-stack-protector \
  -mno-avx -c wine_hello.c -o "$T/wine_hello.obj"
lld-link /nologo /entry:start /subsystem:console /nodefaultlib /base:0x140000000 /dynamicbase \
  /Brepro /out:"$T/wine_hello.exe" "$T/wine_hello.obj" "$T/kernel32.lib"
cp "$T/wine_hello.exe" "$OUT"
