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
assert 'app_lifecycle::info(39)' in s
assert 'c.target_pd == 12 && c.task_id == 12 && c.surface_id == 154' in s
assert 'reply_delivery::legacy_reply(caller,value,pdx_reply)' in s
assert 'SHELL_REPLY.poll(pdx_reply)' in s
assert s.count('linen_reply(caller_pd,')>=10
PY
rustc --edition=2021 --test "$ROOT/tests/linen_reply_delivery.rs" -o "$TMP/test"
"$TMP/test"
echo '[gate.linen_reply_delivery_host.result] PASS scope=production_reply_custody_and_source_wiring'
