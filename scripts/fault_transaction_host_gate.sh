#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-fault-transaction.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/fault_transaction.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.fault_transaction.result] PASS scope=production_identity_custody_not_mapping_execution'
