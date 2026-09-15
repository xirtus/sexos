#!/usr/bin/env bash
# Sex AArch64 Phase-2 QEMU runner — generic bring-up ONLY.
# QEMU S1POE IS UNVERIFIED and is NOT probed or claimed in this phase.
# Expected runtime markers (in order) when QEMU is available:
#   [sex.arm.entry] current_el=2
#   [sex.arm.el1] ok=1
#   [sex.arm.dtb] ptr=0x4...
#   [sex.arm.dtb] magic=ok
#   [sex.arm.uart] pl011=ok
#   SEX_ARM_ALIVE
# Negative expectations:
#   no [sex.arm.panic]
#   no reboot loop
#   no S1POE claim of any kind
set -euo pipefail
cd "$(dirname "$0")/.."

IMG=target/aarch64-sex/release/sex-kernel-arm64
[ -f "$IMG" ] || { echo "[run_arm64] FATAL: image missing; run scripts/build_arm64_phase2.sh"; exit 1; }

if ! command -v qemu-system-aarch64 >/dev/null 2>&1; then
  echo "SKIP_NO_QEMU: qemu-system-aarch64 not found on this host"
  echo "Install QEMU, then run:"
  echo "  qemu-system-aarch64 -M virt,gic-version=3 -cpu max -m 512M -nographic -kernel $IMG"
  exit 77
fi

exec qemu-system-aarch64 \
  -M virt,gic-version=3 \
  -cpu max \
  -m 512M \
  -nographic \
  -kernel "$IMG"
