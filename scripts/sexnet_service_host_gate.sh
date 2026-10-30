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
