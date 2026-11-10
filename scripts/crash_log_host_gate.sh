#!/usr/bin/env bash
# Crash record encode/decode and disk ring placement (sex_pdx::crash_log) on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crash-log.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/crash_log_record.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.crash_log_host.result] PASS'
