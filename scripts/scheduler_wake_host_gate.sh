#!/usr/bin/env bash
# Host-only lifecycle helper regressions and explicit ownership-model evidence.
# This is not a kernel/image build entrypoint or an SMP execution proof.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-scheduler-wake.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/kernel/src/scheduler_wake.rs" -o "$TEST_DIR/wake-tests"
"$TEST_DIR/wake-tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/scheduler_ownership_model.rs" -o "$TEST_DIR/model-tests"
"$TEST_DIR/model-tests"
echo '[gate.scheduler_wake_host.result] PASS'
