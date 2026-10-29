#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-task-exit.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/task_exit_lifecycle.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.task_exit_lifecycle.result] PASS scope=production_watch_and_custody_not_physical_stack_or_TLB_execution'
