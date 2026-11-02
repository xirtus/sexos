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
# The fixture includes actual Main metadata helpers, freshly extracted here.
# Runtime controls must compile and fail their exact named test; neither a
# missing mutation site nor a compiler error qualifies as rejection evidence.
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY_BROWSER_TLS'
import json,os,re,subprocess,sys
from pathlib import Path
root,out=map(Path,sys.argv[1:]);main=(root/'apps/kaleidoscope/src/main.rs').read_text();panel=(root/'apps/kaleidoscope/src/browser_view.rs').read_text();fixture=root/'tests/browser_view.rs'
def viewport_block(source):
 start=source.index('            if req.type_id==sex_pdx::document_view::OP{')
 end=source.index('            if req.type_id==1&&req.caller_pd==11',start)
 return source[start:end]
def viewport_custody(source):
 body=re.sub(r'\s+','',re.sub(r'//[^\n]*','',viewport_block(source)))
 if 'self.ui.cancel()'in body:raise ValueError('live viewport may not abandon an admitted text batch')
 for required in ['self.ui_last.fill(0)','self.ui_metadata=None','self.ui_dirty=true']:
  if required not in body:raise ValueError('viewport must queue fresh layout after draining its historical completion')
viewport_custody(main)
for name,old,new in [('cancel-admitted','self.ui_last.fill(0)','self.ui.cancel();self.ui_last.fill(0)'),('lost-current-layout','self.ui_dirty=true','self.ui_dirty=false')]:
 block=viewport_block(main)
 if block.count(old)!=1:raise ValueError('ambiguous viewport custody control')
 bad=main.replace(block,block.replace(old,new,1),1)
 try:viewport_custody(bad)
 except ValueError:pass
 else:raise ValueError('viewport source defect survived '+name)
print('[browser.viewport.custody] PASS source_controls=2 scope=actual_live_notice_preserves_active_and_invalidates_layout_cache_not_native_pixels',flush=True)
def once(source,old,new):
 if source.count(old)!=1:raise ValueError('missing/ambiguous exact TLS mutation site: '+old)
 return source.replace(old,new,1)
def helpers(source):
 extracted=[]
 for name in ('tls_view_code','tls_frame_metadata'):
  matches=list(re.finditer(r'^fn '+name+r'\(',source,re.M))
  if len(matches)!=1:raise ValueError('missing/ambiguous actual Main helper: '+name)
  start=matches[0].start();brace=source.index('{',start);depth=1;end=brace+1
  while depth and end<len(source):
   depth+=(source[end]=='{')-(source[end]=='}');end+=1
  if depth:raise ValueError('incomplete actual Main helper: '+name)
  extracted.append(source[start:end])
 return '\n'.join(extracted)+'\n'
def compile_case(name,main_source,panel_path,fixture_override=None):
 helper=out/(name+'-main-helpers.rs');helper.write_text(helpers(main_source))
 env=dict(os.environ,BROWSER_TLS_MAIN=str(helper));source_path=fixture
 if panel_path is not None or fixture_override is not None:
  def path(match):
   actual=(fixture.parent/match[1]).resolve(strict=True)
   if panel_path is not None and actual==(root/'apps/kaleidoscope/src/browser_view.rs').resolve():actual=panel_path
   return '#[path='+json.dumps(str(actual))+']'
  source=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',path,fixture_override if fixture_override is not None else fixture.read_text())
  if panel_path is not None and str(panel_path)not in source:raise ValueError('fixture does not include actual Browser panel')
  source_path=out/(name+'-fixture.rs');source_path.write_text(source)
 binary=out/name
 result=subprocess.run(['rustc','--edition=2021','--extern','sex_net_config='+str(out/'libsex_net_config.rlib'),'--test',str(source_path),'-o',str(binary)],env=env,capture_output=True,text=True,timeout=60)
 (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
 if result.returncode:raise ValueError(name+': fixture must compile; '+result.stderr[-6000:])
 return binary
def run_case(name,binary,test=None,reject=False):
 command=[str(binary),'--test-threads=1']+(['--exact',test]if test else[])
 result=subprocess.run(command,capture_output=True,text=True,timeout=60);text=result.stdout+result.stderr
 (out/(name+'.log')).write_text(text);print(text,end='',flush=True)
 if reject:
  if result.returncode!=101 or 'running 1 test'not in text or 'test '+test+' ... FAILED'not in text:raise ValueError(name+': exact compiled TLS defect was not rejected at runtime')
 elif result.returncode:raise ValueError(name+': actual TLS fixture failed')
 return text
baseline=compile_case('browser-view-tests',main,None);text=run_case('browser-view-tests',baseline)
normal=re.search(r'test result: ok\. (\d+) passed; 0 failed;',text)
if not normal:raise ValueError('missing normal TLS fixture result')
mutant_panel=once(panel,'h<top+GLYPH_HEIGHT*factor+INSET','h<top+GLYPH_HEIGHT*factor')
mutant_panel=once(mutant_panel,'h-top-GLYPH_HEIGHT*factor-INSET','h-top-GLYPH_HEIGHT*factor')
path=out/'browser-panel-missing-bottom-rim.rs';path.write_text(mutant_panel)
run_case('browser-panel-missing-bottom-rim',compile_case('browser-panel-missing-bottom-rim',main,path),'tls_rim_and_height_boundaries_use_explicit_fallback',True)
mutant_main=once(main,'first:viewport as usize','first:0');(out/'browser-main-wrong-queued-viewport.rs').write_text(mutant_main)
run_case('browser-main-wrong-queued-viewport',compile_case('browser-main-wrong-queued-viewport',mutant_main,None),'tls_every_state_replaces_old_status_with_exact_whole_lines',True)
cancelled=once(fixture.read_text(),'    for viewport in 8..=10 {','    for viewport in 8..=10 { out.cancel();')
run_case('browser-live-viewport-cancel',compile_case('browser-live-viewport-cancel',main,None,cancelled),'live_viewport_keeps_delayed_commit_until_published_receipt_then_latest_clock',True)
print('[browser.tls.panel.host] PASS cases='+normal[1]+' compiled_controls=3 scope=actual_panel_main_metadata_outbox_display_stage_not_native_pixels_or_authority',flush=True)
PY_BROWSER_TLS
echo '[sexnet.http.client.host] PASS scope=actual_client_ABI_job_codec_and_pure_view_not_live_authority_or_pixels'

rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/resolved_http.rs" -o "$TEST_DIR/resolved-http-tests"
"$TEST_DIR/resolved-http-tests"
echo '[sexnet.resolved.http.host] PASS scope=actual_DNS_release_provenance_HTTP_client_ABI_jobs_codecs_with_explicit_policy_peer_not_live_authority_or_network'

rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/document_reader.rs" -o "$TEST_DIR/document-reader-tests"
"$TEST_DIR/document-reader-tests"
echo '[sexnet.document.reader.host] PASS scope=actual_viewport_reader_and_shared_metrics_not_live_display_pixels_input_or_network'

rustc --edition=2021 --test "$ROOT_DIR/tests/browser_asset.rs" -o "$TEST_DIR/browser-asset-tests"
"$TEST_DIR/browser-asset-tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/browser_html_text.rs" -o "$TEST_DIR/browser-html-text-tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/browser_script.rs" -o "$TEST_DIR/browser-script-tests"
"$TEST_DIR/browser-script-tests"
"$TEST_DIR/browser-html-text-tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/browser_bitmap_delivery.rs" -o "$TEST_DIR/browser-bitmap-tests"
"$TEST_DIR/browser-bitmap-tests"
python3 "$ROOT_DIR/scripts/check_browser_output_identity.py" --root "$ROOT_DIR" --baseline-binary "$TEST_DIR/browser-bitmap-tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/browser_asset_presentation.rs" -o "$TEST_DIR/browser-asset-presentation-tests"
"$TEST_DIR/browser-asset-presentation-tests"
bash "$ROOT_DIR/scripts/browser_asset_fence_host_gate.sh"
echo '[sexnet.asset.host] PASS scope=actual_bounded_decoder_view_outbox_and_display_stage_not_live_authority_or_pixels'
rustc --edition=2021 --extern sex_net_config="$TEST_DIR/libsex_net_config.rlib" --test "$ROOT_DIR/tests/http_navigation.rs" -o "$TEST_DIR/http-navigation-tests"
"$TEST_DIR/http-navigation-tests"
echo '[sexnet.http.navigation.host] PASS scope=actual_bounded_path_history_and_codec_not_live_network_authority'

rustc --edition=2021 --test "$ROOT_DIR/tests/entropy_service.rs" -o "$TEST_DIR/entropy-service-tests"
"$TEST_DIR/entropy-service-tests"
PYTHONDONTWRITEBYTECODE=1 python3 "$ROOT_DIR/scripts/check_entropy_lease.py" --root "$ROOT_DIR" --baseline-binary "$TEST_DIR/entropy-service-tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/entropy_client.rs" -o "$TEST_DIR/entropy-client-tests"
"$TEST_DIR/entropy-client-tests"
echo '[sexnet.entropy.host] PASS scope=actual_ABI_private_service_client_not_live_CPU_quality_or_authority'

rustc --edition=2021 --crate-name sex_time --crate-type rlib "$ROOT_DIR/crates/sex-time/src/lib.rs" -o "$TEST_DIR/libsex_time.rlib"
rustc --edition=2021 --extern sex_time="$TEST_DIR/libsex_time.rlib" --test "$ROOT_DIR/tests/certificate_time_client.rs" -o "$TEST_DIR/certificate-time-tests"
"$TEST_DIR/certificate-time-tests"
echo '[sexnet.certificate.time.host] PASS scope=actual_UTC_ABI_source_decoder_broker_client_not_live_RTC_authenticity_or_TLS'
bash "$ROOT_DIR/scripts/browser_calendar_host_gate.sh"

rustc --edition=2021 --crate-name sex_trust_config --crate-type rlib "$ROOT_DIR/crates/sex-trust-config/src/lib.rs" -o "$TEST_DIR/libsex_trust_config.rlib"
rustc --edition=2021 --extern sex_trust_config="$TEST_DIR/libsex_trust_config.rlib" --test "$ROOT_DIR/tests/trust_policy_source.rs" -o "$TEST_DIR/trust-policy-source-tests"
"$TEST_DIR/trust-policy-source-tests"
rustc --edition=2021 --extern sex_trust_config="$TEST_DIR/libsex_trust_config.rlib" --test "$ROOT_DIR/tests/trust_policy_client.rs" -o "$TEST_DIR/trust-policy-client-tests"
"$TEST_DIR/trust-policy-client-tests"
echo '[sexnet.trust.policy.host] PASS scope=actual_immutable_config_source_ABI_broker_kernel_entry_model_client_not_live_BOOT_authority_or_peer_authentication'

HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
CARGO_TARGET_DIR="$TEST_DIR/cargo-target" RUSTFLAGS='--cfg aes_force_soft --cfg polyval_force_soft' cargo test --locked \
  --manifest-path "$ROOT_DIR/Cargo.toml" -p sex-tls --target "$HOST_TARGET"
echo '[sexnet.tls.host] PASS scope=actual_private_TLS_backend_clients_brokers_not_live_entropy_time_network_or_peer_authentication'

# Compile the identical actual vendored library with its pinned provider-free
# test manifest. Published host integration/OpenSSL dependencies stay excluded.
CARGO_TARGET_DIR="$TEST_DIR/cargo-target" RUSTFLAGS='--cfg aes_force_soft --cfg polyval_force_soft' cargo test --locked \
  --manifest-path "$ROOT_DIR/vendor/embedded-tls-0.19.0/sexos-record-tests/Cargo.toml" --target "$HOST_TARGET" --lib
echo '[sexnet.tls.records.host] PASS scope=actual_AEAD_empty_padding_truncation_and_unsupported_handshake_not_live_peer'

bash "$ROOT_DIR/scripts/browser_text_host_gate.sh"
