#!/usr/bin/env bash
# Rebuilds wine_msvc.exe (CW4 test program importing api-ms-win-crt-*.dll and
# vcruntime140.dll the way MSVC builds do) from wine_msvc.c with clang and lld-link. No
# Windows SDK: import libraries come from defs/*.def. Deterministic (/Brepro).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
OUT="${1:-wine_msvc.exe}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
libs=()
for d in defs/*.def; do
  n="$(basename "$d" .def)"
  llvm-dlltool -m i386:x86-64 -d "$d" -l "$T/$n.lib"
  libs+=("$T/$n.lib")
done
clang --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-builtin -fno-stack-protector \
  -mno-avx -c wine_msvc.c -o "$T/wine_msvc.obj"
lld-link /nologo /entry:start /subsystem:console /nodefaultlib /base:0x140000000 /dynamicbase \
  /Brepro /out:"$T/wine_msvc.exe" "$T/wine_msvc.obj" "${libs[@]}"
cp "$T/wine_msvc.exe" "$OUT"
