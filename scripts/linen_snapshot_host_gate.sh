#!/usr/bin/env bash
# Actual production asynchronous snapshot engine and publication function.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-linen-snapshot.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
import pathlib,re,sys
source=(pathlib.Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
def exact(pattern):
    matches=re.findall(pattern,source,re.M|re.S)
    if len(matches)!=1:raise SystemExit(f'production extraction failed: {pattern}: {len(matches)}')
    return matches[0]
parts=[]
for name in ['LINEN_MAX_OBJECTS','SURFACE_ID_LINEN']:
    parts.append(exact(r'^(?:pub )?const '+name+r':[^\n]+;'))
for name in ['LinenObjectKind','LinenObjectState']:
    parts.append('#[derive(Debug,Clone,Copy,PartialEq,Eq)]\n#[repr(u8)]\n'+exact(r'^enum '+name+r' \{.*?^\}'))
parts.append('#[derive(Debug,Clone,Copy)]\n'+exact(r'^struct LinenObject \{.*?^\}'))
parts.append(exact(r'^const LINEN_SEED_OBJECTS:.*?^\];'))
for name in ['linen_async_reply','linen_select_first_valid_object','linen_fetch_remote_snapshot','linen_queue_open','linen_async_poll']:
    parts.append(exact(r'^(?:unsafe )?fn '+name+r'\b.*?^\}'))
# Fail if a blocking receiver returns to the production snapshot route.
assert 'fn linen_sync_reply' not in source
assert source.count('pdx_call(sex_pdx::SLOT_LINEN, OP_LINEN_OPEN_INTENT')==0
assert 'if linen_async_reply(&msg)' in source
pathlib.Path(sys.argv[2]).write_text('\n\n'.join(parts))
PY
LINEN_SNAPSHOT_SOURCE="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/linen_snapshot.rs" -o "$TEST_DIR/linen-tests"
"$TEST_DIR/linen-tests" --test-threads=1
echo '[gate.linen_snapshot_host.result] PASS scope=production_async_state_and_publication_mock_PDX'
