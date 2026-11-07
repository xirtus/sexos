#!/usr/bin/env bash
# Program files are untrusted: the kernel's checker and the store directory, on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-program-image.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on -A dead_code "$ROOT_DIR/tests/program_image.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
