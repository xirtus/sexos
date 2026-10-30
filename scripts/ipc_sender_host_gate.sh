#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-ipc-sender.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/message.rs" "$TEST_DIR/syscall_return.rs" <<'PY'
from pathlib import Path
import json,re,sys
root=Path(sys.argv[1]);source=(root/'crates/sex-pdx/src/lib.rs').read_text()
start=source.index('#[repr(C)]\n#[derive(Clone, Copy, Debug)]\npub struct PdxMessage');end=source.index('\n}',start)+2
Path(sys.argv[2]).write_text(source[start:end]+'\n')
for name,next_name in [('pdx_listen_raw','pdx_try_listen_raw'),('pdx_try_listen_raw','pdx_listen')]:
 body=source.split('pub fn '+name+'(',1)[1].split('pub fn '+next_name+'(',1)[0]
 assert 'inlateout("rdi") slot => caller_task' in body and 'caller_task: u32::try_from(caller_task).unwrap_or(0)'in body
kernel=(root/'kernel/src/syscalls/mod.rs').read_text();listen=kernel.split('28 => { // SYSCALL_PDX_LISTEN',1)[1].split('29 => { // SYSCALL_PDX_REPLY',1)[0]
assert listen.count('s_caller_task = caller_task as u64')==2 and 's_caller_task = reply.caller_task as u64'in listen and 'regs.rdi = s_caller_task'in listen
invalid=kernel.split('if !crate::capability_slot::valid_syscall_selectors',1)[1].split('let error = match rax',1)[1].split('29 =>',1)[0]
assert 'regs.rdi = 0'in invalid
ipc=(root/'kernel/src/ipc.rs').read_text();assert 'sender::current_task(caller_pd_id).unwrap_or(0)'in ipc and 'if caller_task == 0 { return Err(sex_pdx::ERR_CAP_INVALID); }'in ipc
interrupts=(root/'kernel/src/interrupts.rs').read_text()
layout=re.findall(r'#\[repr\(C\)\]\s*pub struct SyscallRegs\s*\{.*?\n\}',interrupts,re.S)
entry=re.findall(r'pub unsafe extern "C" fn syscall_entry\(\)\s*\{(.*?)\n\}',interrupts,re.S)
assert len(layout)==len(entry)==1,'actual syscall layout/entry extraction changed'
instructions=[]
for line in entry[0].splitlines():
 instructions.extend(re.findall(r'"([^"\n]*)"',line.split('//',1)[0]))
invalid_listen=re.findall(r'28\s*=>\s*\{(.*?)\n\s*\}',invalid,re.S)
assert len(invalid_listen)==1,'actual invalid-listen clearing extraction changed'
Path(sys.argv[3]).write_text(layout[0]+'\nconst ENTRY_ASM:&[&str]=&'+json.dumps(instructions)+';\nfn actual_invalid_listen(regs:&mut SyscallRegs)->u64{'+invalid_listen[0]+'\n}\n')
print('[gate.ipc_sender.wiring] PASS scope=source_register_and_enqueue_wiring_not_executed_syscall')
PY
IPC_MESSAGE_SOURCE="$TEST_DIR/message.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/ipc_sender_runtime.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
SYSCALL_RETURN_SOURCE="$TEST_DIR/syscall_return.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/syscall_return_runtime.rs" -o "$TEST_DIR/syscall-return-tests"
"$TEST_DIR/syscall-return-tests"
echo '[gate.ipc_sender.result] PASS scope=actual_kernel_sender_catalog_message_layout_and_syscall_return_assembly_model_not_executed_x86_or_native_containment'
