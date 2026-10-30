#!/usr/bin/env bash
# Production pure helpers and extracted exact runtime query; no kernel boot.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-memory-authority.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/memory_region_authority.rs -o "$TEST_DIR/regions"
"$TEST_DIR/regions"
rustc --edition=2021 --test tests/dma_region_pool.rs -o "$TEST_DIR/dma"
"$TEST_DIR/dma"
rustc --edition=2021 --test tests/pci_bar_bounds.rs -o "$TEST_DIR/pci"
"$TEST_DIR/pci"
rustc --edition=2021 --test tests/boot_frame_reservations.rs -o "$TEST_DIR/reservations"
"$TEST_DIR/reservations"
python3 - "$TEST_DIR/query.rs" <<'PY'
from pathlib import Path
import re
import sys
s=Path('kernel/src/syscalls/mod.rs').read_text()
a=s.index('pub(crate) fn region_query(')
b=s.index('\npub fn dispatch(',a)
Path(sys.argv[1]).write_text(s[a:b])
assert 'let reply = region_query(pd, rax, rdi, rsi, rdx, r10);' in s
for forbidden in ('allocate_va(', 'GLOBAL_ALLOCATOR.alloc(', 'map_physical_range(', 'grant_capability(', 'allocate_shared_buffer('):
    assert forbidden not in s, forbidden
assert '30 | 31 | 50 | 51 => u64::MAX' in s
assert '40 => 0' in s
boot=Path('kernel/src/boot_memory.rs').read_text()
assert boot.index('reserve_boot_region(') < boot.index('core::ptr::write_bytes(')
manager=Path('kernel/src/memory/manager.rs').read_text()
assert 'if self.owned_regions.contains(phys_addr) { continue' in manager
# Scan the shared runtime too: unsuffixed asm selectors were a real missed heap
# caller. This source guard complements the actual dispatcher/consumer tests.
raw = re.compile(r'(?:in|inlateout)\("rax"\)\s*(?:30|31|40|50|51)(?:u64|usize)?\b')
for path in [Path('sex-rt/src/lib.rs'), Path('apps/sexdrive/src/main.rs'),
             *Path('servers/sexusb/src').glob('*.rs'),
             *Path('servers/sexnet/src').glob('*.rs'),
             *Path('servers/sexfiles/src').rglob('*.rs')]:
    assert not raw.search(path.read_text()), f'obsolete ambient memory syscall: {path}'
PY
SEXOS_REGION_QUERY_RS="$TEST_DIR/query.rs" rustc --edition=2021 --test tests/memory_region_dispatch.rs -o "$TEST_DIR/dispatch"
"$TEST_DIR/dispatch"
python3 - "$TEST_DIR/document_boot.rs" <<'PYDOCUMENT'
from pathlib import Path
import sys
s=Path('kernel/src/boot_memory.rs').read_text();parts=[]
for name in ['grant','paired','paired_at','init_documents']:
    a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
    while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
    parts.append(s[a:i])
Path(sys.argv[1]).write_text('\n'.join(parts))
init=Path('kernel/src/init.rs').read_text()
assert init.index('boot_memory::init_documents(sexfiles_id, [quil_id, successor, quil_third])') < init.index('boot_memory::seal_all()')
PYDOCUMENT
SEXOS_REGION_QUERY_RS="$TEST_DIR/query.rs" DOCUMENT_BOOT_SOURCE="$TEST_DIR/document_boot.rs" rustc --edition=2021 --test tests/document_boot_views.rs -o "$TEST_DIR/document-boot"
"$TEST_DIR/document-boot"
python3 - "$TEST_DIR/document_resolver.rs" <<'PYRESOLVER'
from pathlib import Path
import sys
s=Path('crates/sex-pdx/src/lib.rs').read_text();a=s.index('pub fn document_snapshot_view(');i=s.index('{',a)+1;depth=1
while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
Path(sys.argv[1]).write_text(s[a:i])
PYRESOLVER
DOCUMENT_SNAPSHOT_RESOLVER_SOURCE="$TEST_DIR/document_resolver.rs" rustc --edition=2021 --test tests/document_snapshot_resolve.rs -o "$TEST_DIR/document-resolver"
"$TEST_DIR/document-resolver"
echo '[memory.region.authority.gate.result] PASS scope=production_helpers_extracted_query_runtime_boot_required'
