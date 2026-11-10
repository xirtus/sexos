#!/usr/bin/env bash
# Intel RAID on the host: IMSM metadata reader (crates/sex-imsm, fixtures checked
# with mdadm --examine) and the VMD bus driver logic (crates/sex-vmd) against a
# simulated VMD domain.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-raid.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
for crate in sex-imsm sex-vmd; do
  name="${crate//-/_}"
  rustc --edition=2021 --crate-type=rlib --crate-name "$name" -O -C overflow-checks=on \
    "$ROOT_DIR/crates/$crate/src/lib.rs" --out-dir "$TEST_DIR"
  for t in "$ROOT_DIR"/crates/$crate/tests/*.rs; do
    bin="$TEST_DIR/$name-$(basename "$t" .rs)"
    CARGO_MANIFEST_DIR="$ROOT_DIR/crates/$crate" rustc --edition=2021 --test -O -L "$TEST_DIR" \
      --extern "$name=$TEST_DIR/lib$name.rlib" "$t" -o "$bin"
    "$bin"
  done
done
echo "[raid.host.gate] PASS"
