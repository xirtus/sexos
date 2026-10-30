#!/usr/bin/env bash
# Actual authorizer, typed invocation, BOOT constructor and port operations;
# hardware/registry/interrupt primitives are explicit host doubles.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-pci-command.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --crate-name sex_net_config --crate-type rlib "$ROOT_DIR/crates/sex-net-config/src/lib.rs" -o "$TEST_DIR/libsex_net_config.rlib"
cd "$ROOT_DIR"
python3 - "$TEST_DIR" <<'PY'
from pathlib import Path
import sys
out=Path(sys.argv[1])
def extract(source,names):
 s=Path(source).read_text();parts=[]
 for name in names:
  a=s.index('fn '+name);a=s.rfind('\n',0,a)+1;i=s.index('{',a)+1;depth=1
  while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
  parts.append(s[a:i])
 return '\n'.join(parts)
(out/'ports.rs').write_text(extract('kernel/src/hal/pci.rs',['config_address','config_locked','config_read_unlocked','config_write_unlocked','pci_config_read','pci_config_write','apply_command']))
(out/'boot.rs').write_text(extract('kernel/src/boot_memory.rs',['nic_command']))
s=Path('kernel/src/syscalls/mod.rs').read_text()
assert 'crate::pci_command::invoke(pd,rdi,rsi,rdx,r10,r8)'in s
for forbidden in ['allocate_va(', 'grant_capability(', 'GLOBAL_ALLOCATOR', 'map_physical_range(']:
 assert forbidden not in Path('kernel/src/pci_command.rs').read_text()
# Every legacy config address/data-port use shares the same HAL lock.
for p in Path('kernel/src').rglob('*.rs'):
 if p.as_posix()!='kernel/src/hal/pci.rs':
  assert not any(port in p.read_text().lower()for port in ['0xcf8','0xcfc']),p
s=Path('kernel/src/devmgr.rs').read_text();a=s.index('(0x02, 0x00)');part=s[a:s.index('            _ => {}',a)]
assert 'command: Some(crate::boot_memory::nic_command(pd,&dev,sex_pdx::SLOT_NIC,view))'in part
assert s[:a].count('command: None')==3
PY
PCI_COMMAND_PORT_SOURCE="$TEST_DIR/ports.rs" rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test tests/pci_command.rs -o "$TEST_DIR/command"
"$TEST_DIR/command" --test-threads=1
NIC_COMMAND_BOOT_SOURCE="$TEST_DIR/boot.rs" rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test tests/nic_command_boot.rs -o "$TEST_DIR/boot"
"$TEST_DIR/boot" --test-threads=1
echo '[pci.command.host.gate] PASS scope=actual_authority_constructor_port_mechanism_with_mock_config_space_not_live_dma_or_packets'
