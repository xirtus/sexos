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
for name in ['quil_activation_edge','quil_activation_info','quil_reserve_remaining','quil_reserve_label','quil_launcher_status','quil_route_slot','quil_has_focus']:
    found=re.findall(r'^fn '+name+r'\b.*?(?=\n(?:fn |unsafe fn |///))',s,re.M|re.S)
    assert len(found)==1,name
    parts.append(found[0])
Path(sys.argv[2]).write_text('\n'.join(parts))
parts=[]
for name in ['spindle_activation_info','spindle_reserve_label','spindle_launcher_status','spindle_route_slot','spindle_has_focus']:
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
# Creation, resource registration and predecessor bindings consume the same
# bounded descriptive catalog. The registered boot_app_catalog check validates
# actual extracted grant tuples; source anchors here only connect genesis.
init=(r/'kernel/src/init.rs').read_text()
assert 'create_boot_domain(module.path(),Some(incarnation.target),layout,pkey)' in init
assert 'assert_eq!(created,incarnation.target,"BOOT_SUCCESSOR_IDENTITY")' in init
assert 'plan_boot_apps(&sex_pdx::app_catalog::TARGETS)' in init
assert 'crate::app_resources::init(sext_id, silkshell_id,' in init
assert 'sex_pdx::app_catalog::CONTENT' in init
assert 'sex_pdx::app_catalog::predecessor(incarnation)' in init
reserve=init.index('reserve_successor(predecessor.target,incarnation.target,silkshell_id,incarnation.activation_slot.expect(')
audit=init.index('crate::app_resources::audit_boot_catalog(silkshell_id);')
assert reserve<audit<init.index('crate::boot_memory::seal_all()')
assert 'MAX_APPS:usize=sex_pdx::app_catalog::COUNT;' in (r/'kernel/src/app_resource_state.rs').read_text()
assert 'SLOTS: [usize; crate::app_catalog::COUNT] = crate::app_catalog::LIFECYCLE_SLOTS' in (r/'crates/sex-pdx/src/app_lifecycle.rs').read_text()
assert 'CATALOG_CAPACITY: usize = crate::app_catalog::COUNT;' in (r/'servers/sexdisplay/src/surface_lifecycle.rs').read_text()
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
assert 'registration_pending || presentation_pending || storage_owner!=0 || unsafe{DISPLAY_IO.pending() || HISTORY_STATUS.pending() || NETWORK_PROBE.busy()}' in s
assert s.index('NETWORK_PROBE.advance(sex_pdx::get_ticks())')<s.index('let msg=if registration_pending')
assert 'if msg.type_id==1 && msg.caller_pd==13' in s
assert s.index('NETWORK_PROBE.close()')<s.index('"[app.close.accepted]')
assert 'if presentation_pending && !unsafe{DISPLAY_IO.pending()}' in s
assert s.index('DISPLAY_IO.cancel()')<s.index('"[app.close.accepted]')
assert 'spindle_display_call' not in s
assert s.index('DISPLAY_IO.text(i,*text,')<s.index('GRID_PREV[i]=Some(*text);')
PYCODE
rustc --edition=2021 --test "$ROOT_DIR/tests/spindle_display_delivery.rs" -o "$TEST_DIR/display-tests"
"$TEST_DIR/display-tests"

python3 - "$ROOT_DIR" "$TEST_DIR/access-focus.rs" <<'PYFOCUS'
from pathlib import Path
import re,sys
s=(Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
f=re.findall(r'^unsafe fn access_move_focus\b.*?^}',s,re.M|re.S);assert len(f)==1
Path(sys.argv[2]).write_text(f[0])
assert 'access_move_focus(&nodes[..count], action == SurfaceAction::AccessFocusNext)' in s
PYFOCUS
ACCESS_FOCUS_SOURCE="$TEST_DIR/access-focus.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/access_focus_cycle.rs" -o "$TEST_DIR/access-focus"
"$TEST_DIR/access-focus"
