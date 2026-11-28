#!/usr/bin/env bash
# Spindle's production `sextime` calculator against the production SexTime crate.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-spindle-sextime.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
source "$(dirname "${BASH_SOURCE[0]}")/dreamroot_core_rlib.sh"
dreamroot_core_rlib "$TEST_DIR"
rustc --edition=2021 --crate-type rlib --crate-name sex_time --extern dreamroot_core="$TEST_DIR/libdreamroot_core.rlib" "$ROOT_DIR/crates/sex-time/src/lib.rs" -o "$TEST_DIR/libsex_time.rlib"
rustc --edition=2021 --extern sex_time="$TEST_DIR/libsex_time.rlib" -L dependency="$TEST_DIR" --test "$ROOT_DIR/tests/spindle_sextime.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[spindle.sextime.host] PASS scope=calculator_output_only_no_clock_source'
