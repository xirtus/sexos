#!/usr/bin/env bash
# Rebuilds hello.exe (checked in, include_bytes! in src/lib.rs) from hello.c with clang and
# lld-link. No Windows SDK and no C runtime: the KERNEL32 import library is made from
# kernel32.def. The output is deterministic (/Brepro), so the gate can compare it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
OUT="${1:-hello.exe}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
llvm-dlltool -m i386:x86-64 -d kernel32.def -l "$T/kernel32.lib"
clang --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-builtin -fno-stack-protector \
  -mno-avx -c hello.c -o "$T/hello.obj"
lld-link /nologo /entry:start /subsystem:console /nodefaultlib /base:0x140000000 /dynamicbase \
  /Brepro /out:"$T/hello.exe" "$T/hello.obj" "$T/kernel32.lib"
cp "$T/hello.exe" "$OUT"
