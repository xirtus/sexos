#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-reply-sender.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/reply_sender_runtime.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
python3 - "$ROOT_DIR" <<'PY'
from pathlib import Path
import sys
source=(Path(sys.argv[1])/'kernel/src/syscalls/mod.rs').read_text()
listen=source.split('28 => { // SYSCALL_PDX_LISTEN',1)[1].split('29 => { // SYSCALL_PDX_REPLY',1)[0]
assert '(0x1, reply.caller_pd as u64, reply.value, 0, 0)' in listen, 'listen loses authenticated reply sender'
assert '(0x1, 1, reply.value, 0, 0)' not in listen, 'legacy placeholder reply sender remains'
assert 's_caller_task = reply.caller_task as u64' in listen and 'regs.rdi = s_caller_task' in listen, 'listen loses actual reply task'
print('[gate.reply_sender.listen] PASS source_register_tuple_with_actual_task')
PY
echo '[gate.reply_sender.result] PASS scope=production_router_sender_retention_not_return_capability_authority'
