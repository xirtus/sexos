#!/usr/bin/env bash
# Host tests of protection views' page-table rules (kernel/src/views_rules.rs).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-views.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/protection_views.rs" -o "$TEST_DIR/views-tests"
"$TEST_DIR/views-tests"
echo '[gate.protection_views_host.result] PASS'
