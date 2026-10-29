#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-silk-memory.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import sys
source=(Path(sys.argv[1])/'crates/silk-client/src/lib.rs').read_text()
functions=[]
for name in ['memcpy','memset']:
    start=source.index('pub unsafe extern "C" fn '+name+'(')
    body=source.index('{',start);depth=1;end=body+1
    while depth:
        depth+=(source[end]=='{')-(source[end]=='}');end+=1
    functions.append(source[start:end])
text='\n'.join(functions)
Path(sys.argv[2]+'/production.rs').write_text('#![no_std]\n'+ '\n'.join('#[no_mangle]\n'+f for f in functions))
Path(sys.argv[2]+'/renamed.rs').write_text(text.replace('fn memcpy(', 'fn tested_memcpy(').replace('fn memset(', 'fn tested_memset('))
PY
rustc --edition=2021 --crate-type lib --emit=obj -O "$TEST_DIR/production.rs" -o "$TEST_DIR/production.o"
python3 "$ROOT_DIR/tests/silk_client_memory_codegen.py" "$TEST_DIR/production.o" "${OBJDUMP:-objdump}"
SILK_MEMORY_SOURCE="$TEST_DIR/renamed.rs" rustc --edition=2021 --test -O "$ROOT_DIR/tests/silk_client_memory.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
if [[ $# -gt 0 ]]; then
    python3 "$ROOT_DIR/tests/silk_client_memory_codegen.py" "$1" "${OBJDUMP:-objdump}"
fi
echo '[gate.silk_memory.result] PASS production_optimized_helpers'
