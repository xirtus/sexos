#!/usr/bin/env bash
# sexusb isochronous transfers (endpoint parameters, Isoch TRBs) and the UVC camera scan, on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-usb-isoch.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on -A dead_code "$ROOT_DIR/tests/sexusb_isoch.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
