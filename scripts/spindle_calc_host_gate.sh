#!/usr/bin/env bash
# Host tests of Spindle's exact decimal calculator (apps/spindle/src/calc_command.rs).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TEST_DIR="$(mktemp -d)"; trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/apps/spindle/src/calc_command.rs" -o "$TEST_DIR/calc-tests"
"$TEST_DIR/calc-tests"
