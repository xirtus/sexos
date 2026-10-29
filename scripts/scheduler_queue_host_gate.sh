#!/usr/bin/env bash
# Host-only tests of the actual no_std-compatible queue implementation.
# Does not build a kernel/image or create an alternate build entrypoint.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-scheduler-queue.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/kernel/src/scheduler_queue.rs" -o "$TEST_DIR/queue-tests"
"$TEST_DIR/queue-tests"
echo '[gate.scheduler_queue_host.result] PASS'
