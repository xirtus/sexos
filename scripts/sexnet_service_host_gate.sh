#!/usr/bin/env bash
# Actual passive service dispatch; no NIC, packet or transport proof.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexnet-service.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/dispatch.rs" <<'PY'
from pathlib import Path
import sys
root=Path(sys.argv[1]);s=(root/'servers/sexnet/src/main.rs').read_text()
a=s.index('const SEXNET_GET_STATUS:');b=s.index('\n#[allow(dead_code)]',a);parts=[s[a:b]]
a=s.index('fn handle_call(');i=s.index('{',a)+1;depth=1
while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
parts.append(s[a:i]);Path(sys.argv[2]).write_text('\n'.join(parts))
assert 'pub mod net;'not in(root/'kernel/src/syscalls/mod.rs').read_text()
assert not(root/'kernel/src/syscalls/net.rs').exists()
PY
SEXNET_SERVICE_SOURCE="$TEST_DIR/dispatch.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/sexnet_service.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests" --test-threads=1
echo '[gate.sexnet_service.result] PASS scope=actual_passive_scalar_dispatch_not_live_network_or_hostile_containment'
python3 - "$ROOT_DIR" "$TEST_DIR/profile.rs" <<'PYPROFILE'
from pathlib import Path
import sys
s=(Path(sys.argv[1])/'kernel/src/hal/pci.rs').read_text();a=s.index('fn legacy_nic_diagnostics(');b=s.index('\n}',a)+2
Path(sys.argv[2]).write_text(s[a:b])
guard=s.index('if !legacy_nic_diagnostics(dev.vendor_id,dev.device_id,option_env!("SEXOS_BOOT_NIC_DIAGNOSTICS"))')
assert guard<s.index('pci_config_write(dev.bus, dev.dev, dev.func, 0x04, new_cmd_status)')
assert 'continue;'in s[guard:s.index('// Ensure NIC PCI command',guard)]
PYPROFILE
NIC_BOOT_PROFILE_SOURCE="$TEST_DIR/profile.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/nic_boot_profile.rs" -o "$TEST_DIR/profile"
"$TEST_DIR/profile"
python3 - "$ROOT_DIR" "$TEST_DIR/boot_memory.rs" <<'PYMEM'
from pathlib import Path
import sys
root=Path(sys.argv[1]);s=(root/'kernel/src/boot_memory.rs').read_text();parts=[]
for name in ['nic_bar0','pci_view']:
 a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
 while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
 parts.append(s[a:i])
Path(sys.argv[2]).write_text('\n'.join(parts))
s=(root/'kernel/src/devmgr.rs').read_text();a=s.index('(0x02, 0x00)');part=s[a:s.index('            _ => {}',a)]
assert part.index('pd.find_capability(sex_pdx::SLOT_NIC as u32).is_some()')<part.index('crate::boot_memory::nic_bar0(')<part.index('pd.grant_capability(')
assert 'bar0_view: Some(view)'in part
PYMEM
NIC_BOOT_MEMORY_SOURCE="$TEST_DIR/boot_memory.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/nic_boot_memory.rs" -o "$TEST_DIR/boot-memory"
"$TEST_DIR/boot-memory" --test-threads=1
bash "$ROOT_DIR/scripts/pci_command_host_gate.sh"
rustc --edition=2021 --test "$ROOT_DIR/tests/sexnet_nic_prepare.rs" -o "$TEST_DIR/nic-prepare"
"$TEST_DIR/nic-prepare" --test-threads=1
echo '[sexnet.nic.prepare.host] PASS scope=actual_bounded_state_machine_with_explicit_command_register_clock_doubles_no_dma_or_packets'
rustc --edition=2021 --test "$ROOT_DIR/tests/sexnet_ethernet.rs" -o "$TEST_DIR/ethernet"
"$TEST_DIR/ethernet" --test-threads=1
echo '[sexnet.ethernet.host] PASS scope=actual_fixed_queue_driver_with_explicit_device_dma_clock_doubles_not_live_wire_or_hostile_dma_containment'
bash "$ROOT_DIR/scripts/sexnet_transport_host_gate.sh"

bash "$ROOT_DIR/scripts/sexnet_jobs_host_gate.sh"
