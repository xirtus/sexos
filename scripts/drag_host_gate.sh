#!/usr/bin/env bash
# Drag on the host.
# D1: the VM ABI's record layouts (sex_pdx::vm) and the hardware encodings the kernel's
# VMX code uses (EPT entries, VMX controls, I/O exit qualification, VMCS segment
# fields, TSS descriptor base).
# D2: drag-vmm, the drag program's VMM core: bzImage loading (synthetic setup headers,
# zero page, E820, command line, initrd, identity tables, 64- and 32-bit entry), the
# 16550, 8259 pair, 8254, virtio-mmio virtio-blk (hostile chains and rings), the MMIO
# instruction decoder and guest page walks, CPUID/MSR policy, and exit dispatch against a
# scripted vCPU. Then the in-tree test guest: it must be what its source assembles to
# (when nasm is here), and it must boot and use a real 16550 and virtio-blk under QEMU
# microvm (TCG; skipped without qemu). The guest under VMX itself is drag_gate.py --accel kvm.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-drag.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on --crate-name sex_pdx_vm \
  "$ROOT_DIR/crates/sex-pdx/src/vm.rs" -o "$TEST_DIR/vm"
"$TEST_DIR/vm"
(cd "$ROOT_DIR" && cargo test --locked -p drag-vmm)
GUEST="$ROOT_DIR/crates/drag-vmm/guest/drag_test_guest"
if command -v nasm >/dev/null; then
  nasm -f bin -o "$TEST_DIR/guest.bin" "$GUEST.asm"
  cmp "$TEST_DIR/guest.bin" "$GUEST.bin" || { echo "[drag.host.gate] FAIL drag_test_guest.bin is not what drag_test_guest.asm assembles to"; exit 1; }
  echo "[drag.host.gate] test guest matches its source"
else
  echo "[drag.host.gate] SKIP no nasm: test guest not reassembled"
fi
python3 "$ROOT_DIR/scripts/drag_guest_qemu.py"
echo "[drag.host.gate] PASS"
