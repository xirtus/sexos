#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexnet-jobs.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --crate-name sex_net_config --crate-type rlib "$ROOT_DIR/crates/sex-net-config/src/lib.rs" -o "$TEST_DIR/libsex_net_config.rlib"
cat > "$TEST_DIR/test.rs" <<RS
extern crate self as sex_pdx;
#[path="$ROOT_DIR/crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="$ROOT_DIR/servers/sexnet/src/jobs.rs"]mod jobs;
RS
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$TEST_DIR/test.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[sexnet.jobs.host] PASS scope=actual_bounded_job_and_ABI_not_live_network_authority'
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/network_probe.rs" -o "$TEST_DIR/client-tests"
"$TEST_DIR/client-tests"
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/spindle_network.rs" -o "$TEST_DIR/spindle-network-tests"
"$TEST_DIR/spindle-network-tests"
echo '[sexnet.spindle.client.host] PASS scope=actual_explicit_client_ABI_job_with_partial_peer_not_live_network_authority_or_desktop'
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/servers/silk-shell/src/browser_input.rs" -o "$TEST_DIR/input-tests"
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
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$TEST_DIR/dns.rs" -o "$TEST_DIR/dns-tests"
"$TEST_DIR/dns-tests"
echo '[sexnet.dns.jobs.host] PASS scope=actual_owned_job_ABI_codec_not_live_network_authority'

rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/dns_probe.rs" -o "$TEST_DIR/dns-client-tests"
"$TEST_DIR/dns-client-tests"
echo '[sexnet.dns.client.host] PASS scope=actual_client_with_private_job_peer_not_live_network'

rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/http_probe.rs" -o "$TEST_DIR/http-client-tests"
"$TEST_DIR/http-client-tests"
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/browser_view.rs" -o "$TEST_DIR/browser-view-tests"
"$TEST_DIR/browser-view-tests"
echo '[sexnet.http.client.host] PASS scope=actual_client_ABI_job_codec_and_pure_view_not_live_authority_or_pixels'

rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/resolved_http.rs" -o "$TEST_DIR/resolved-http-tests"
"$TEST_DIR/resolved-http-tests"
echo '[sexnet.resolved.http.host] PASS scope=actual_DNS_release_provenance_HTTP_client_ABI_jobs_codecs_with_explicit_policy_peer_not_live_authority_or_network'

rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/document_reader.rs" -o "$TEST_DIR/document-reader-tests"
"$TEST_DIR/document-reader-tests"
echo '[sexnet.document.reader.host] PASS scope=actual_viewport_reader_and_shared_metrics_not_live_display_pixels_input_or_network'
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/http_navigation.rs" -o "$TEST_DIR/http-navigation-tests"
"$TEST_DIR/http-navigation-tests"
echo '[sexnet.http.navigation.host] PASS scope=actual_bounded_path_history_and_codec_not_live_network_authority'
