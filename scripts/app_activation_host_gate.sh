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
parts=[]
for name in ['spindle_activation_info','spindle_launcher_status','spindle_route_slot','spindle_has_focus']:
    found=re.findall(r'^fn '+name+r'\b.*?(?=\n(?:fn |unsafe fn |///))',s,re.M|re.S)
    assert len(found)==1,name
    parts.append(found[0])
parts.append(re.findall(r'^const fn is_spindle_text_key\b.*?^}',s,re.M|re.S)[0])
app=(r/'apps/spindle/src/main.rs').read_text()
parts.append(re.findall(r'^fn scancode_to_ascii\b.*?^}',app,re.M|re.S)[0])
Path(sys.argv[2]+'.spindle').write_text('\n'.join(parts))
k=(r/'kernel/src/app_resources.rs').read_text();a=k[k.index('pub fn activation('):k.index('pub fn ack(')]
assert a.index('g.authorize(')<a.index('if new.activated')<a.index('restore_reserved_pages(')<a.index('new.activated=true')<a.index('activate_task(main)')
for path in ['kernel/src/ipc.rs','kernel/src/ipc/router.rs']:
    assert 'app_resources::accepts_ipc(' in (r/path).read_text()
assert 'app_resources::activation_permitted(' in (r/'kernel/src/scheduler.rs').read_text()
assert 'load_elf(elf_path, pd_id, layout, domain_id)' in (r/'kernel/src/pd/create.rs').read_text()
assert 'create_boot_domain(quil_module.path(),Some(16),16,9)' in (r/'kernel/src/init.rs').read_text()
assert 'create_boot_domain(quil_module.path(),Some(18),18,9)' in (r/'kernel/src/init.rs').read_text()
assert 'create_boot_domain(spindle_module.path(),Some(17),17,12)' in (r/'kernel/src/init.rs').read_text()
# Verify the explicit aggregate budget and chain order in the actual genesis.
init=(r/'kernel/src/init.rs').read_text()
assert 'plan_boot_apps(&[9, 12, 15, 16, 17, 18])' in init
assert '(spindle_successor,302), (quil_third,307)' in init
assert init.index('reserve_successor(quil_id,successor,') < init.index('reserve_successor(successor,quil_third,') < init.index('crate::boot_memory::seal_all()')
assert 'MAX_APPS:usize=6;' in (r/'kernel/src/app_resource_state.rs').read_text()
assert 'SLOTS: [usize; 6] = [38, 39, 40, 45, 50, 54]' in (r/'crates/sex-pdx/src/app_lifecycle.rs').read_text()
assert 'CATALOG_CAPACITY: usize = 6;' in (r/'servers/sexdisplay/src/surface_lifecycle.rs').read_text()
for name,value in [('QUIL_THIRD_LIFECYCLE_SLOT',54),('QUIL_THIRD_ACTIVATION_SLOT',55),('QUIL_THIRD_CLOSE_SLOT',56),('QUIL_THIRD_ROUTE_SLOT',57)]:
    assert re.search(r'pub const '+name+r':(?:usize|u64)='+str(value)+r';',(r/'crates/sex-pdx/src/app_lifecycle.rs').read_text())
print('[app.activation.wiring] PASS scope=static_reservation_and_commit_order')
PY
QUIL_ACTIVATION_PRODUCTION="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/quil_activation.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"

SPINDLE_ACTIVATION_PRODUCTION="$TEST_DIR/production.rs.spindle" rustc --edition=2021 --test "$ROOT_DIR/tests/spindle_activation.rs" -o "$TEST_DIR/spindle-tests"
"$TEST_DIR/spindle-tests"

python3 - "$ROOT_DIR" <<'PYCODE'
from pathlib import Path
import sys
s=(Path(sys.argv[1])/'apps/spindle/src/main.rs').read_text()
assert 'DISPLAY_IO.poll(4,' in s
assert 'registration_pending || presentation_pending || storage_owner!=0 || unsafe{DISPLAY_IO.pending() || HISTORY_STATUS.pending()}' in s
assert 'if presentation_pending && !unsafe{DISPLAY_IO.pending()}' in s
assert s.index('DISPLAY_IO.cancel()')<s.index('"[app.close.accepted]')
assert 'spindle_display_call' not in s
assert s.index('DISPLAY_IO.text(i,*text,')<s.index('GRID_PREV[i]=Some(*text);')
PYCODE
rustc --edition=2021 --test "$ROOT_DIR/tests/spindle_display_delivery.rs" -o "$TEST_DIR/display-tests"
"$TEST_DIR/display-tests"
