#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-paging-map.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/paging_authority_state.rs -o "$TEST_DIR/state"
"$TEST_DIR/state"
rustc --edition=2021 --test tests/paging_commit_runtime.rs -o "$TEST_DIR/runtime"
"$TEST_DIR/runtime"
python3 - <<'PY'
from pathlib import Path
s=Path('kernel/src/paging.rs').read_text()
for forbidden in ('allocate_va(', 'GLOBAL_ALLOCATOR', 'map_to(', 'map_physical_range(', 'write_bytes(', 'grant_capability('):
    assert forbidden not in s, forbidden
reclaim=s[s.index('pub fn reclaim('):]
assert reclaim.index('write_volatile(leaf,0)') < reclaim.index('tlb::flush(') < reclaim.index('state.ledger.retire(')
cleanup=s[s.index('pub fn cleanup('):]
assert cleanup.index('retire_dead_fault(') < cleanup.index('write_volatile(') < cleanup.index('tlb::flush(') < cleanup.index('close_exited_range(')
boot=Path('kernel/src/boot_memory.rs').read_text()
assert 'vas.mapper.unmap(' in boot
assert 'PAGING_BOOT_LEAF_PRESENT' in boot
assert 'crate::paging::initialize(' in boot
PY
echo '[paging.mapping.host.gate] PASS scope=production_state_commit_reclaim_cleanup_mock_fault_transport_tlb'
