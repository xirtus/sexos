#!/usr/bin/env bash
# Host tests of the scheduler's deadline class (kernel/src/scheduler_deadline.rs).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-deadline.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/scheduler_deadline.rs" -o "$TEST_DIR/deadline-tests"
"$TEST_DIR/deadline-tests"
echo '[gate.scheduler_deadline_host.result] PASS'
