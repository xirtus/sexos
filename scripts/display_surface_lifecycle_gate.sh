#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-display-lifecycle.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/display_surface_lifecycle.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.display_surface_lifecycle.result] PASS scope=production_inline_resources_not_kernel_attestation_or_guest_pixels'
