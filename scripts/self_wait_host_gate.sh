#!/usr/bin/env bash
# Production timer/inventory/ownership helpers, no privileged context emulation.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-self-wait.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/self_wait.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.self_wait_host.result] PASS scope=production_atomics_not_live_x86_timer_or_SMP'
