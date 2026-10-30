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

cat > "$TEST_DIR/dns.rs" <<RS
extern crate self as sex_pdx;
extern crate self as sex_net;
#[path="$ROOT_DIR/crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="$ROOT_DIR/crates/sex-pdx/src/network_dns.rs"]pub mod network_dns;
#[path="$ROOT_DIR/crates/sex-net/src/dns_wire.rs"]pub mod dns_wire;
#[path="$ROOT_DIR/servers/sexnet/src/jobs.rs"]mod jobs;
#[path="$ROOT_DIR/servers/sexnet/src/dns_jobs.rs"]mod dns_jobs;
RS
rustc --edition=2021 --test "$TEST_DIR/dns.rs" -o "$TEST_DIR/dns-tests"
"$TEST_DIR/dns-tests"
echo '[sexnet.dns.jobs.host] PASS scope=actual_owned_job_ABI_codec_not_live_network_authority'

rustc --edition=2021 --test "$ROOT_DIR/tests/dns_probe.rs" -o "$TEST_DIR/dns-client-tests"
"$TEST_DIR/dns-client-tests"
echo '[sexnet.dns.client.host] PASS scope=actual_client_with_private_job_peer_not_live_network'
