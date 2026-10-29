#!/usr/bin/env bash
# Native tests of actual kernel selector decoding; no kernel/image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-authority-slots.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/authority_slot_validation.rs -o "$TEST_DIR/authority-tests"
"$TEST_DIR/authority-tests"
echo '[authority.slot_validation.gate.result] PASS'
