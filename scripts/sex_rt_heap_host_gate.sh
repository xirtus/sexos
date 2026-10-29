#!/usr/bin/env bash
# Host-only tests of the production bounded byte allocator, not a kernel build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexrt-heap.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import json, re, sys
root, work = map(Path, sys.argv[1:])
(work / 'Cargo.lock').write_bytes((root / 'Cargo.lock').read_bytes())
source = (root / 'sex-rt/src/lib.rs').read_text()
if re.search(r'in(?:lateout)?\("rax"\)\s*(?:30|31)(?:u64)?\s*[,;]', source):
    raise SystemExit('[gate.sex_rt_heap] FAIL legacy ambient allocation/mapping syscall')
(work / 'Cargo.toml').write_text('''[package]
name = "sex-rt-heap-host-tests"
version = "0.0.0"
edition = "2021"
[dependencies]
linked_list_allocator = { version = "=0.10.6", default-features = false }
spin = "=0.9.9"
[[test]]
name = "production_heap"
path = ''' + json.dumps(str(root / 'tests/sex_rt_heap.rs')) + '\n')
PY
CARGO_NETWORK_ARGS=()
if [[ "${SEXOS_HOST_OFFLINE:-0}" == "1" ]]; then CARGO_NETWORK_ARGS+=(--offline); fi
cargo test ${CARGO_NETWORK_ARGS[@]+"${CARGO_NETWORK_ARGS[@]}"} --manifest-path "$TEST_DIR/Cargo.toml" --target-dir "$TEST_DIR/target" -- --nocapture
echo '[gate.sex_rt_heap.result] PASS scope=production_ELF_arena_allocator_not_new_memory_authority'
