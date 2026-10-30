#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexnet-jobs.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cat > "$TEST_DIR/test.rs" <<RS
extern crate self as sex_pdx;
#[path="$ROOT_DIR/crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="$ROOT_DIR/servers/sexnet/src/jobs.rs"]mod jobs;
RS
rustc --edition=2021 --test "$TEST_DIR/test.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[sexnet.jobs.host] PASS scope=actual_bounded_job_and_ABI_not_live_network_authority'
rustc --edition=2021 --test "$ROOT_DIR/tests/network_probe.rs" -o "$TEST_DIR/client-tests"
"$TEST_DIR/client-tests"
rustc --edition=2021 --test "$ROOT_DIR/servers/silk-shell/src/browser_input.rs" -o "$TEST_DIR/input-tests"
"$TEST_DIR/input-tests"
