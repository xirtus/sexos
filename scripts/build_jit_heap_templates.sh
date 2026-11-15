#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Regenerate the audited, position-independent x86-64 helper templates.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
object="$(mktemp)"
trap 'rm -f "$object"' EXIT
"${CLANG:-clang}" --target=x86_64-unknown-linux-gnu -c "$root/crates/sex-js/guest/jit/heap.S" -o "$object"
# Local branches must be resolved; runtime code may not contain relocations.
"${READELF:-llvm-readelf}" -r "$object" | grep -q 'There are no relocations'
for kind in own compare concat; do
  "${OBJCOPY:-llvm-objcopy}" -O binary --only-section=".text.$kind" "$object" "$root/crates/sex-js/guest/jit/$kind.bin"
done
