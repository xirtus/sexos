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
echo '[gate.shell_app_cleanup_host.result] PASS scope=production_retirement_and_ack_mock_transport'
