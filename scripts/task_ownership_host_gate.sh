#!/usr/bin/env bash
# Production atomic ownership/queue tests, not an alternate kernel build path.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-task-ownership.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/task_ownership.rs" -o "$TEST_DIR/ownership-tests"
"$TEST_DIR/ownership-tests" --nocapture
echo '[gate.task_ownership_host.result] PASS scope=production_atomics_queue_not_x86_stack_proof'
