#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-workspace.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/sext_workspace.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.sext_workspace.result] PASS scope=production_private_pool_journal_real_memory'
