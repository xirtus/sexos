#!/usr/bin/env bash
# Per-core I/O lanes (crates/sex-lanes): shared rings and their wake-up handshake,
# adaptive polling, NVMe multi-queue commands, queue plans and RSS, on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-lanes.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --crate-type=rlib --crate-name sex_lanes -O -C overflow-checks=on \
  "$ROOT_DIR/crates/sex-lanes/src/lib.rs" --out-dir "$TEST_DIR"
for t in "$ROOT_DIR"/crates/sex-lanes/tests/*.rs; do
  name="$(basename "$t" .rs)"
  rustc --edition=2021 --test -O -C overflow-checks=on -L "$TEST_DIR" --extern sex_lanes="$TEST_DIR/libsex_lanes.rlib" "$t" -o "$TEST_DIR/$name"
  "$TEST_DIR/$name"
done
echo "[lanes.host.gate] PASS"
