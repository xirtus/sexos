#!/usr/bin/env bash
# Production userland DMA lifetime and extracted driver peer-check tests.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexdrive-dma.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/block_view.rs" "$TEST_DIR/read_path.rs" <<'PY'
from pathlib import Path
import sys
source = (Path(sys.argv[1]) / 'apps/sexdrive/src/main.rs').read_text()
start = source.index('fn block_buffer_view(')
end = source.index('\nfn ', start + 1)
Path(sys.argv[2]).write_text(source[start:end])
parts = []
for name in ['stop_uncertain_io', 'prepare_bounce', 'typed_read_handoff', 'nvme_read_into_mapped_va', 'nvme_write_one_block', 'write_guard_allows']:
    start = source.index('fn ' + name + '(')
    end = source.index('\nfn ', start + 1)
    parts.append(source[start:end])
assert 'BLOCK_READ => typed_read_handoff(offset,size,_buf_cap,block_view),' in source
assert 'bad_len_phase_b' not in source
Path(sys.argv[3]).write_text('\n'.join(parts))
PY
SEXDRIVE_BLOCK_VIEW_SOURCE="$TEST_DIR/block_view.rs" SEXDRIVE_READ_PATH_SOURCE="$TEST_DIR/read_path.rs" rustc --edition=2021 --test \
  "$ROOT_DIR/tests/sexdrive_dma_transfer.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests" --nocapture
echo '[gate.sexdrive_dma_host.result] PASS scope=production_lifetime_peer_check_not_device_DMA_containment'
python3 - "$ROOT_DIR" "$TEST_DIR/service.rs" "$TEST_DIR/service_abi.rs" <<'PY'
from pathlib import Path
import re,sys
s=(Path(sys.argv[1])/'apps/sexdrive/src/main.rs').read_text();parts=[]
for name in ['service_next_request','fill_initial_frame']:
 a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
 while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
 parts.append(s[a:i])
entry=s[s.index('pub extern "C" fn _start()'):]
assert entry.index('fill_initial_frame(shared_addr);')<entry.index('pdx_call(SLOT_SHELL, OP_SHELL_BIND_BUFFER, shared_addr, 0, 0);')
assert 'loop { service_next_request(); }' in entry
assert 'pdx_try_listen_raw'not in s and '0..2_000_000'not in entry
Path(sys.argv[2]).write_text('\n'.join(parts))
abi=(Path(sys.argv[1])/'crates/sex-pdx/src/lib.rs').read_text()
names=['BLOCK_READ','BLOCK_WRITE','BLOCK_SYNC','BLOCK_MAX_XFER','BLOCK_SECTOR_SIZE','BLOCK_ERR_BAD_LEN','BLOCK_ERR_NO_DEVICE','BLOCK_ERR_BAD_CMD','SLOT_BLOCK_BUFFER']
constants=[]
for name in names:
 match=re.search(r'pub const '+name+r'\s*:\s*u64\s*=\s*[^;]+;',abi)
 assert match,name
 constants.append(match.group())
Path(sys.argv[3]).write_text('\n'.join(constants))
PY
SEXDRIVE_SERVICE_SOURCE="$TEST_DIR/service.rs" SEXDRIVE_SERVICE_ABI_SOURCE="$TEST_DIR/service_abi.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/sexdrive_service_loop.rs" -o "$TEST_DIR/service"
"$TEST_DIR/service" --test-threads=1
echo '[gate.sexdrive_service.result] PASS scope=actual_dispatch_with_mock_IPC_and_device_not_kernel_wake_proof'
