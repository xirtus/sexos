#!/usr/bin/env bash
# NVIDIA GSP-RM boot logic (crates/sex-gpu-nv) on the host: firmware containers,
# VBIOS/FWSEC, the VRAM layout and GspFwWprMeta (offsets pinned to NVIDIA's C
# structs), radix3, LibOS arguments, the RPC queues, the CPU sequencer, and the
# whole boot against a simulated GA104. NVIDIA's real 535.113.01 files are
# parsed too when they can be fetched. No GPU here: the chip is proven on the x17.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
FW_DIR="$ROOT_DIR/target/nvidia-firmware"
if bash scripts/fetch_nvidia_gsp_firmware.sh "$FW_DIR"; then
  export NV_GSP_FW_DIR="$FW_DIR"
else
  echo "[gate.nvidia] note: firmware not fetched; real-image test skipped"
fi
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-nv.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
R="rustc --edition=2021 -O -C overflow-checks=on"
$R --crate-type=rlib --crate-name sex_gpu_nv crates/sex-gpu-nv/src/lib.rs --out-dir "$TEST_DIR"
for t in crates/sex-gpu-nv/tests/*.rs; do
  name="sex-gpu-nv-$(basename "$t" .rs)"
  $R --test -L "$TEST_DIR" --extern sex_gpu_nv="$TEST_DIR/libsex_gpu_nv.rlib" "$t" -o "$TEST_DIR/$name"
  "$TEST_DIR/$name" -q
done
echo "[gate.nvidia.result] PASS scope=gsp_boot_logic_host_not_gpu"
