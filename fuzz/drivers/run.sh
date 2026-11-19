#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Every driver target through the built-in mutator (any toolchain, no cargo-fuzz):
#   ./run.sh                    DRIVER_FUZZ_SECONDS (default 20) per target
#   DRIVER_FUZZ_ITERS=5000 DRIVER_FUZZ_SECONDS=0 ./run.sh drv_hda drv_xhci_usb
# Targets in REPORT_ONLY fail only with STRICT=1: their findings are in code we
# do not own yet (see README.md, "Known findings").
set -uo pipefail
cd "$(dirname "$0")"
REPORT_ONLY="drv_acpi_aml"
export DRIVER_FUZZ_SECONDS=${DRIVER_FUZZ_SECONDS-20}
export DRIVER_FUZZ_OUT=${DRIVER_FUZZ_OUT:-$PWD/crashes}
cargo build --release --locked --bins -q || exit 1
targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then mapfile -t targets < <(grep -oE '^name = "drv_[a-z0-9_]+"' Cargo.toml | cut -d'"' -f2); fi
fail=0
for t in "${targets[@]}"; do
  if out=$(./target/release/"$t" 2>&1); then
    echo "$out" | tail -1
  elif [[ " $REPORT_ONLY " == *" $t "* && "${STRICT:-0}" != 1 ]]; then
    echo "$out" | tail -3
    echo "[driver_fuzz.report_only] target=$t (known findings, see README.md)"
  else
    echo "$out" | tail -3
    fail=1
  fi
done
exit $fail
