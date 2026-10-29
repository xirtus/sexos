#!/usr/bin/env bash
# Pure host regression tests only. No x86 assembly or alternate image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-pagefault-transport.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
python3 - <<'PY'
import pathlib
import re
import tomllib

contract = tomllib.loads(pathlib.Path("sexos_contract.toml").read_text())
fault = contract["ipc"]["page_fault"]
listen = contract["ipc"]["pdx_listen"]
codec = pathlib.Path("crates/sex-pdx/src/paging.rs").read_text()
opcode = int(re.search(r"pub const OP_FAULT_NOTICE: u64 = (0x[0-9a-fA-F]+|[0-9]+);", codec)[1], 0)
assert fault["opcode"] == opcode != listen["empty_type_id"]
assert fault["slot"] == contract["slots"]["sext"] == 2
for fault_key, listen_key in [("type_id_reg", "type_id_reg"), ("caller_pd_reg", "caller_pd_reg"), ("fault_addr_reg", "arg0_reg"), ("error_code_reg", "arg1_reg"), ("token_reg", "arg2_reg")]:
    assert fault[fault_key] == listen[listen_key], (fault_key, listen_key)
assert (fault["address_bits"], fault["error_code_bits"], fault["domain_id_bits"]) == (48, 32, 32)
assert fault["domain_zero_allowed"] is False
assert fault["capability_kind"] == "Domain-async-enqueue-only"
assert fault["target_policy"] == "nonzero-in-registry-and-distinct-from-caller"
handler = pathlib.Path("kernel/src/interrupts.rs").read_text().split('pub extern "C" fn page_fault_handler',1)[1].split('pub fn init_idt',1)[0]
assert handler.index("old_ctx.pkru =") < handler.index(".state.park_fault(") < handler.index("::forward_page_fault(")
assert "fault.kill [paging.fault.terminal]" in handler
assert "fault_transaction::eligible_user_fault" in handler
print("[pagefault.register_transport.contract] PASS")
PY
rustc --edition=2021 --test tests/pagefault_register_transport.rs -o "$TEST_DIR/pagefault-tests"
"$TEST_DIR/pagefault-tests"
echo '[pagefault.register_transport.gate.result] PASS (production transport/identity/custody; mapping and IRET require guest proof)'
