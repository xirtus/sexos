#!/usr/bin/env bash
# Display model (crates/sex-kms) and the Intel Gen12 display engine driver
# logic (crates/sex-gpu-intel), checked on the host against fake registers.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-kms.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
R="rustc --edition=2021 -O -C overflow-checks=on"
$R --crate-type=rlib --crate-name sex_kms "$ROOT_DIR/crates/sex-kms/src/lib.rs" --out-dir "$TEST_DIR"
$R --crate-type=rlib --crate-name sex_gpu_intel -L "$TEST_DIR" --extern sex_kms="$TEST_DIR/libsex_kms.rlib" \
  "$ROOT_DIR/crates/sex-gpu-intel/src/lib.rs" --out-dir "$TEST_DIR"
for t in "$ROOT_DIR"/crates/sex-kms/tests/*.rs "$ROOT_DIR"/crates/sex-gpu-intel/tests/*.rs; do
  name="$(basename "$(dirname "$(dirname "$t")")")-$(basename "$t" .rs)"
  $R --test -L "$TEST_DIR" --extern sex_kms="$TEST_DIR/libsex_kms.rlib" \
    --extern sex_gpu_intel="$TEST_DIR/libsex_gpu_intel.rlib" "$t" -o "$TEST_DIR/$name"
  "$TEST_DIR/$name" -q
done
echo "[kms.host.gate] PASS"
