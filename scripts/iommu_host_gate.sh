#!/usr/bin/env bash
# IOMMU data (crates/sex-iommu): DMAR/IVRS tables, VT-d fields, I/O page
# tables and the lending window, checked on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-iommu.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --crate-type=rlib --crate-name sex_iommu -O -C overflow-checks=on \
  "$ROOT_DIR/crates/sex-iommu/src/lib.rs" --out-dir "$TEST_DIR"
for t in "$ROOT_DIR"/crates/sex-iommu/tests/*.rs; do
  name="$(basename "$t" .rs)"
  rustc --edition=2021 --test -O -C overflow-checks=on -L "$TEST_DIR" --extern sex_iommu="$TEST_DIR/libsex_iommu.rlib" "$t" -o "$TEST_DIR/$name"
  "$TEST_DIR/$name"
done
echo "[iommu.host.gate] PASS"
