#!/usr/bin/env bash
# Drag D1 on the host: the VM ABI's record layouts (sex_pdx::vm) and the hardware
# encodings the kernel's VMX code uses (EPT entries, VMX controls, I/O exit
# qualification, VMCS segment fields, TSS descriptor base).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-drag.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on --crate-name sex_pdx_vm \
  "$ROOT_DIR/crates/sex-pdx/src/vm.rs" -o "$TEST_DIR/vm"
"$TEST_DIR/vm"
echo "[drag.host.gate] PASS"
