#!/usr/bin/env bash
# Hot-plug rules (crates/sex-hotplug): slot life, PCIe register decode and
# window planning for docks and eGPUs, checked on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-hotplug.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
R="rustc --edition=2021 -O -C overflow-checks=on"
$R --crate-type=rlib --crate-name sex_hotplug "$ROOT_DIR/crates/sex-hotplug/src/lib.rs" --out-dir "$TEST_DIR"
for t in "$ROOT_DIR"/crates/sex-hotplug/tests/*.rs; do
  $R --test -L "$TEST_DIR" --extern sex_hotplug="$TEST_DIR/libsex_hotplug.rlib" "$t" -o "$TEST_DIR/$(basename "$t" .rs)"
  "$TEST_DIR/$(basename "$t" .rs)" -q
done
echo "[hotplug.host.gate] PASS"
