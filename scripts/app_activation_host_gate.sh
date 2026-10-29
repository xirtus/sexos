#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-activation.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
from pathlib import Path
import re,sys
r=Path(sys.argv[1]);s=(r/'servers/silk-shell/src/main.rs').read_text()
parts=[]
for name in ['quil_activation_info','quil_launcher_status','quil_route_slot','quil_has_focus']:
    found=re.findall(r'^fn '+name+r'\b.*?(?=\n(?:fn |unsafe fn |///))',s,re.M|re.S)
    assert len(found)==1,name
    parts.append(found[0])
Path(sys.argv[2]).write_text('\n'.join(parts))
k=(r/'kernel/src/app_resources.rs').read_text();a=k[k.index('pub fn activation('):k.index('pub fn ack(')]
assert a.index('g.authorize(')<a.index('if new.activated')<a.index('restore_reserved_pages(')<a.index('new.activated=true')<a.index('activate_task(main)')
for path in ['kernel/src/ipc.rs','kernel/src/ipc/router.rs']:
    assert 'app_resources::accepts_ipc(' in (r/path).read_text()
assert 'app_resources::activation_permitted(' in (r/'kernel/src/scheduler.rs').read_text()
assert 'load_elf(elf_path, pd_id, layout, domain_id)' in (r/'kernel/src/pd/create.rs').read_text()
assert 'create_boot_domain(quil_module.path(),Some(16),16,9)' in (r/'kernel/src/init.rs').read_text()
print('[app.activation.wiring] PASS scope=static_reservation_and_commit_order')
PY
QUIL_ACTIVATION_PRODUCTION="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/quil_activation.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
