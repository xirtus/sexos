#!/usr/bin/env bash
# Hardware census table (crates/sex-hwdb): names, owners and warnings for the
# bare-metal test machine and the QEMU gate machine, checked on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-hwdb.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --crate-type=rlib --crate-name sex_hwdb -O -C overflow-checks=on \
  "$ROOT_DIR/crates/sex-hwdb/src/lib.rs" --out-dir "$TEST_DIR"
for t in "$ROOT_DIR"/crates/sex-hwdb/tests/*.rs; do
  name="$(basename "$t" .rs)"
  rustc --edition=2021 --test -O -L "$TEST_DIR" --extern sex_hwdb="$TEST_DIR/libsex_hwdb.rlib" "$t" -o "$TEST_DIR/$name"
  "$TEST_DIR/$name"
done
echo "[hwdb.host.gate] PASS"
