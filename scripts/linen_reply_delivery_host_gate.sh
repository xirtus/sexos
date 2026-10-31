#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/linen-reply.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
python3 - "$ROOT" <<'PY'
import pathlib,sys,re
s=(pathlib.Path(sys.argv[1])/'servers/linen/src/main.rs').read_text()
assert not re.search(r'\bpdx_reply\s*\(',s), 'unretained reply call remains'
assert 'if !linen_reply_poll() { sched_yield(); continue; }' in s
assert 'if !linen_request_allowed(msg.caller_pd,msg.type_id) { continue; }' in s
assert 'app_lifecycle::info(slot)' in s
assert 'app_catalog::by_target(caller).filter(|i|i.app==sex_pdx::app_catalog::App::Spindle)' in s
assert 'c.target_pd==caller && c.task_id==task && c.surface_id==surface && c.coordinator_pd==3' in s
assert 'reply_delivery::legacy_reply(caller,value,pdx_reply)' in s
assert 'SHELL_REPLY.poll(pdx_reply)' in s
assert s.count('linen_reply(caller_pd,')>=10
PY
rustc --edition=2021 --test "$ROOT/tests/linen_reply_delivery.rs" -o "$TMP/test"
"$TMP/test"
echo '[gate.linen_reply_delivery_host.result] PASS scope=production_reply_custody_and_source_wiring'

python3 - "$ROOT" "$TMP/observer.rs" <<'PYCODE'
from pathlib import Path
import re,sys
s=(Path(sys.argv[1])/'servers/linen/src/main.rs').read_text()
f=re.findall(r'^fn linen_request_allowed\b.*?^}',s,re.M|re.S)
assert len(f)==1
Path(sys.argv[2]).write_text(f[0])
PYCODE
LINEN_OBSERVER_PRODUCTION="$TMP/observer.rs" rustc --edition=2021 --test "$ROOT/tests/linen_observer_authority.rs" -o "$TMP/observer"
"$TMP/observer"
