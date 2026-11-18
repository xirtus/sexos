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
# Crossdress CW1 + the CLAP plugin bridge: the PE loader, the cell (page tables, GDT/IDT/TSS, stubs), the KERNEL32
# subset and exception dispatch against a scripted vCPU; then the test .exe in a real
# VT-x/SVM cell on Linux KVM (tests/kvm.rs, SKIP without a usable /dev/kvm).
(cd "$ROOT_DIR" && cargo test --locked -p sex-crossdress -- --nocapture 2>&1 | grep -E '^\[crossdress|^test result|FAILED|panicked' ; exit "${PIPESTATUS[0]}")
CROSS="$ROOT_DIR/crates/sex-crossdress/test"
if command -v clang >/dev/null && command -v lld-link >/dev/null && command -v llvm-dlltool >/dev/null; then
  bash "$CROSS/build.sh" "$TEST_DIR/hello.exe"
  bash "$CROSS/build_gain.sh" "$TEST_DIR/gain.clap"
  # The checked-in .exe is byte-exact only for the LLVM major it was built with
  # (test/LLVM_MAJOR). Another LLVM must still build it, and that build must pass in the
  # KVM cell too (SKIP without a usable /dev/kvm).
  llvm_major=$(clang --version | sed -n 's/.*clang version \([0-9]*\)\..*/\1/p' | head -1)
  if [[ "$llvm_major" == "$(cat "$CROSS/LLVM_MAJOR")" ]]; then
    cmp "$TEST_DIR/hello.exe" "$CROSS/hello.exe" || { echo "[drag.host.gate] FAIL hello.exe is not what hello.c builds to"; exit 1; }
    echo "[drag.host.gate] crossdress test program matches its source"
    cmp "$TEST_DIR/gain.clap" "$CROSS/gain.clap" || { echo "[drag.host.gate] FAIL gain.clap is not what gain.c builds to"; exit 1; }
    echo "[drag.host.gate] crossdress test plugin matches its source"
  else
    echo "[drag.host.gate] crossdress test program rebuilt with LLVM $llvm_major (checked in: LLVM $(cat "$CROSS/LLVM_MAJOR")); running the rebuild"
    (cd "$ROOT_DIR" && CROSSDRESS_EXE="$TEST_DIR/hello.exe" CROSSDRESS_CLAP="$TEST_DIR/gain.clap" cargo test --locked -p sex-crossdress --test kvm -- --nocapture 2>&1 | grep -E '^\[crossdress\.kvm(\.clap)?\] (stop|SKIP|exe|plugin|ok)|^test result|FAILED|panicked' ; exit "${PIPESTATUS[0]}")
  fi
else
  echo "[drag.host.gate] SKIP no clang/lld-link: crossdress test program not rebuilt"
fi
echo "[drag.host.gate] PASS"
