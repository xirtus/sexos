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
# Shell side: the look switch and the rounded-corner hit mask.
cargo build -q --manifest-path "$ROOT_DIR/Cargo.toml" -p sex-graphics
rustc --edition=2021 --test --crate-name animations "$ROOT_DIR/servers/silk-shell/src/animations.rs" \
  --extern sex_graphics="$ROOT_DIR/target/debug/libsex_graphics.rlib" -L dependency="$ROOT_DIR/target/debug/deps" -o "$TEST_DIR/look"
"$TEST_DIR/look"
echo '[gate.display_decor.result] PASS scope=decor_pass_window_shape_and_shell_hit_mask_host'
