#!/usr/bin/env bash
# Qupid's "Keep watching" logic on the host: episodes from names, next file, skip and
# "Up next" cards (crates/sex-episodes/src/episodes.rs, segments.rs).
set -euo pipefail
cd "$(dirname "$0")/.."
TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 -A dead_code --test tests/qupid_binge.rs -o "$TEST_DIR/qupid-binge"
"$TEST_DIR/qupid-binge"
