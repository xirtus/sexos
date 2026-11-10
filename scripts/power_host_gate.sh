#!/usr/bin/env bash
# Power ABI: sensor/info encodings, EISA ids, and the bounds-checked \_S5 reader, on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-power.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on -A dead_code "$ROOT_DIR/tests/power.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
