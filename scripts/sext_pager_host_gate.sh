#!/usr/bin/env bash
# Actual sext policy/allocator/protocol, mocked register transport and frame writes.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-sext-pager.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/sext_pager.rs" -o "$TEST_DIR/pager-tests"
"$TEST_DIR/pager-tests"
echo '[gate.sext_pager_host.result] PASS scope=production_pager_policy_allocator_mock_transport_not_guest_recovery'
