#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-bell-cleanup.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
from pathlib import Path
import sys
root,out=map(Path,sys.argv[1:]);s=(root/'servers/sexbell/src/main.rs').read_text()
def region(start,end):
    a=s.index(start);return s[a:s.index(end,a)]
parts=[region('const BELL_QUEUE_CAPACITY:', '// ── Read-Cap Allowlist'),
       region('const MUTE_LIST_CAPACITY:', '/// Check if a caller PD is muted.'),
       region('const POLICY_TABLE_CAPACITY:', '/// Look up policy'),
       region('const SPAM_WINDOW_TICKS:', '/// Check and record a notify event')]
a=s.index('unsafe fn retire_app_resources(');brace=s.index('{',a);depth=1;i=brace+1
while depth:
    depth+=(s[i]=='{')-(s[i]=='}');i+=1
parts.append(s[a:i]);out.write_text('\n'.join(parts))
PY
BELL_CLEANUP_SOURCE="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/bell_app_cleanup.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[gate.bell_app_cleanup.result] PASS scope=actual_retirement_body_and_queue_not_kernel_attestation'
