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
rustc --edition=2021 --test "$ROOT_DIR/tests/reply_credit.rs" -o "$TEST_DIR/credit-tests"
"$TEST_DIR/credit-tests"
# Compiled control: an obligation that is never spent must fail exact-once.
mkdir -p "$TEST_DIR/mutant/kernel/src/ipc" "$TEST_DIR/mutant/tests"
cp "$ROOT_DIR/kernel/src/ipc.rs" "$ROOT_DIR/kernel/src/app_resources.rs" "$TEST_DIR/mutant/kernel/src/"
cp "$ROOT_DIR/kernel/src/ipc/router.rs" "$TEST_DIR/mutant/kernel/src/ipc/"
cp "$ROOT_DIR/tests/reply_credit.rs" "$TEST_DIR/mutant/tests/"
python3 - "$ROOT_DIR/kernel/src/ipc/reply_credit.rs" "$TEST_DIR/mutant/kernel/src/ipc/reply_credit.rs" <<'PY'
import sys
source=open(sys.argv[1]).read();old='        *count -= 1;\n'
assert source.count(old)==1
open(sys.argv[2],'w').write(source.replace(old,''))
PY
rustc --edition=2021 --test "$TEST_DIR/mutant/tests/reply_credit.rs" -o "$TEST_DIR/credit-mutant"
set +e
"$TEST_DIR/credit-mutant" --exact each_accepted_invocation_permits_exactly_one_reply_to_its_caller > "$TEST_DIR/credit-mutant.log" 2>&1
CREDIT_STATUS=$?
set -e
if [[ "$CREDIT_STATUS" != 101 ]]; then cat "$TEST_DIR/credit-mutant.log"; echo '[gate.reply_sender.negative] FAIL'; exit 1; fi
echo '[gate.reply_sender.negative] PASS unspent_obligation_rejected=1'
echo '[gate.reply_sender.result] PASS scope=production_router_sender_retention_and_kernel_reply_obligations_not_hostile_containment'
