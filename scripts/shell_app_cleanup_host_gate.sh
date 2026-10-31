#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMP="$(mktemp -d "${TMPDIR:-/tmp}/shell-app-cleanup.XXXXXX")"
trap 'rm -rf "$TEMP"' EXIT
python3 - "$ROOT" "$TEMP/production.rs" <<'PY'
import pathlib,re,sys
root=pathlib.Path(sys.argv[1]);source=(root/'servers/silk-shell/src/main.rs').read_text()
assert 'e.target == msg.caller_pd && !e.allows_operation(msg.type_id, [msg.arg0,msg.arg1,msg.arg2])' in source
assert 'if !app_custody::buffer_sender(msg.caller_pd) { continue; }' in source
parts=[]
for name in ['shell_retire_app_surface','shell_lifecycle_poll','shell_lifecycle_hint','shell_close_poll','shell_request_app_close','shell_registration_reply_poll']:
    matches=re.findall(r'^unsafe fn '+name+r'\b.*?^\}',source,re.M|re.S)
    if len(matches)!=1:raise SystemExit('production extraction mismatch: '+name)
    parts.extend(matches)
pathlib.Path(sys.argv[2]).write_text('\n'.join(parts))
PY
SHELL_CLEANUP_SOURCE="$TEMP/production.rs" rustc --edition=2021 --test "$ROOT/tests/shell_app_cleanup.rs" -o "$TEMP/test"
"$TEMP/test" --test-threads=1
# Compiled control: cleanup that skips terminal input retirement must fail.
python3 - "$TEMP/production.rs" "$TEMP/mutant.rs" <<'PY'
import sys
source=open(sys.argv[1]).read();old='APP_INPUT.retire(subject);'
assert source.count(old)==1
open(sys.argv[2],'w').write(source.replace(old,''))
PY
SHELL_CLEANUP_SOURCE="$TEMP/mutant.rs" rustc --edition=2021 --test "$ROOT/tests/shell_app_cleanup.rs" -o "$TEMP/mutant-test"
set +e
"$TEMP/mutant-test" --exact cleanup_removes_every_owned_tab_preserving_unrelated_frame_and_prototypes > "$TEMP/mutant.log" 2>&1
MUTANT_STATUS=$?
set -e
if [[ "$MUTANT_STATUS" != 101 ]]; then cat "$TEMP/mutant.log"; echo '[gate.shell_app_cleanup_host.negative] FAIL'; exit 1; fi
echo '[gate.shell_app_cleanup_host.negative] PASS skipped_input_retirement_rejected=1'
echo '[gate.shell_app_cleanup_host.result] PASS scope=production_retirement_clipboard_input_and_ack_mock_transport'
