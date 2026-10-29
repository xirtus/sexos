#!/usr/bin/env bash
# Host protocol and startup-wiring checks only; no kernel/image build route.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-shell-ready.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/shell_ready_protocol.rs -o "$TEST_DIR/shell-ready-tests"
"$TEST_DIR/shell-ready-tests"
echo '[shell.ready.protocol.gate] PASS (host and source; runtime ordering requires QEMU)'
