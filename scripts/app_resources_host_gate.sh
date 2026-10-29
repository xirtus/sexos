#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-app-resources.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/app_resource_state.rs -o "$TEST_DIR/state"
"$TEST_DIR/state"
rustc --edition=2021 --test tests/app_resources_runtime.rs -o "$TEST_DIR/runtime"
"$TEST_DIR/runtime"
python3 - <<'PY'
from pathlib import Path
s=Path('kernel/src/app_resources.rs').read_text()
runtime=s[s.index('pub fn status('):]
for forbidden in ('allocate_va(', 'GLOBAL_ALLOCATOR', 'map_to(', 'map_physical_range(', 'write_bytes(', 'grant_capability(', 'free_pages('):
    assert forbidden not in runtime, forbidden
retire=runtime[runtime.index('pub fn retire('):runtime.index('pub fn poll_exit_notifications(')]
assert retire.index('dead(app)?') < retire.index('target_cleanup_complete(') < retire.index('retire_private_pages(') < retire.index('app.retired=true')
elf=Path('kernel/src/elf.rs').read_text()
assert '.update_flags(page, PageTableFlags::from_bits_retain(crate::app_resource_state::final_elf_flags(final_flags.bits(), pku_key)))' in elf
PY
echo '[app.resources.host.gate] PASS scope=production_authority_leaf_transition_runtime_mock_registry_mapper_tlb'
