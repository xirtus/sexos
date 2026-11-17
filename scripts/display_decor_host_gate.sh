#!/usr/bin/env bash
# Rounded corners and shadows: Display's real decoration pass over a flat
# reference compositor, plus the shared window-shape math.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-display-decor.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/display_decor.rs" -o "$TEST_DIR/decor"
"$TEST_DIR/decor"
rustc --edition=2021 --test --crate-name window_shape "$ROOT_DIR/crates/sex-graphics/src/window_shape.rs" -o "$TEST_DIR/shape"
"$TEST_DIR/shape"
echo '[gate.display_decor.result] PASS scope=decor_pass_and_window_shape_host'
