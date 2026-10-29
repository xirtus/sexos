#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-service-custody.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
python3 - "$TEST_DIR" <<'PY'
from pathlib import Path
import sys
s=Path('servers/sexfiles/src/backends/mod.rs').read_text()
Path(sys.argv[1],'backend_trait.rs').write_text(s[:s.index('pub mod ramfs;')])
lib=Path('crates/sex-pdx/src/lib.rs').read_text()
line=next(line for line in lib.splitlines() if line.startswith('pub const DISKFS_V2_MAX_READ:'))
Path(sys.argv[1],'diskfs_constant.rs').write_text(line+'\n')
PY
rustc --edition=2021 --test tests/service_custody.rs -o "$TEST_DIR/custody"
"$TEST_DIR/custody"
APP_RESOURCE_TEST_DIR="$TEST_DIR" rustc --edition=2021 --test tests/ramfs_retirement.rs -o "$TEST_DIR/ramfs"
"$TEST_DIR/ramfs"
python3 - "$TEST_DIR/history.rs" <<'PY'
from pathlib import Path
import sys
s=Path('apps/spindle/src/main.rs').read_text()
Path(sys.argv[1]).write_text(s[s.index('const MAX_HISTORY:'):s.index('fn history_nav(')])
assert 'msg.type_id==1 && msg.caller_pd==11' in s
assert 'spindle_storage_sync' not in s and 'spindle_storage_settle' not in s
assert 'STORAGE_PROBE.step(' in s and 'STORAGE_PROBE.close()' in s
assert 'HISTORY_IO.step_at(hist,sex_pdx::get_ticks(),' in s and 'HISTORY_IO.receive(msg.arg0,hist)' in s
assert 'if !control{unsafe{HISTORY_IO.mark_dirty();}}' in s
assert 'b"save"|b"load"|b"history-status"|b"history-cancel"|b"history-retry"' in s
tr=Path('servers/sexfiles/src/trampoline.rs').read_text()
assert tr.index('history.retire(notice.target_pd)')<tr.index('vfs::retire_app_subject(notice.target_pd)')
assert 'history.poll(pdx_reply)' in tr and 'crate::spindle_history::live(caller)' in tr
PY
APP_RESOURCE_TEST_DIR="$TEST_DIR" rustc --edition=2021 --test tests/spindle_history.rs -o "$TEST_DIR/history"
"$TEST_DIR/history"
rustc --edition=2021 --test tests/history_journal.rs -o "$TEST_DIR/journal"
"$TEST_DIR/journal"
rustc --edition=2021 --test tests/history_recovery.rs -o "$TEST_DIR/recovery"
"$TEST_DIR/recovery"
rustc --edition=2021 --test tests/spindle_viewport.rs -o "$TEST_DIR/viewport"
"$TEST_DIR/viewport"
python3 - "$TEST_DIR/editor.rs" <<'PYEDITOR'
from pathlib import Path
import re,sys
s=Path('apps/spindle/src/main.rs').read_text()
parts=[s[s.index('#[derive(Clone, Copy, PartialEq)]\nenum ViMode'):s.index('// ── Prompt redraw')]]
for name in ['handle_key','handle_key_insert','handle_key_normal','scancode_to_ascii']:
    parts.append(re.findall(r'^fn '+name+r'\b.*?^}',s,re.M|re.S)[0])
Path(sys.argv[1]).write_text('\n'.join(parts))
PYEDITOR
SPINDLE_EDITOR_SOURCE="$TEST_DIR/editor.rs" rustc --edition=2021 --test tests/spindle_editor.rs -o "$TEST_DIR/editor"
"$TEST_DIR/editor"
python3 - "$TEST_DIR/scroll.rs" <<'PYSCROLL'
from pathlib import Path
import re,sys
s=Path('apps/spindle/src/main.rs').read_text()
parts=[re.findall(r'^const '+name+r':[^\n]+;',s,re.M)[0]for name in ['MAX_SCROLLBACK','MAX_LINE_BYTES']]
parts.append(s[s.index('struct Scrollback {'):s.index('// ── BSS storage')])
Path(sys.argv[1]).write_text('\n'.join(parts))
PYSCROLL
SPINDLE_SCROLL_SOURCE="$TEST_DIR/scroll.rs" rustc --edition=2021 --test tests/spindle_scrollback.rs -o "$TEST_DIR/scroll"
"$TEST_DIR/scroll"
rustc --edition=2021 --test tests/spindle_status.rs -o "$TEST_DIR/status"
"$TEST_DIR/status"
python3 - "$TEST_DIR/probe_parse.rs" <<'PYPARSE'
import re,sys
from pathlib import Path
s=Path('apps/spindle/src/main.rs').read_text()
f=re.findall(r'^fn parse_probe_job\b.*?^}',s,re.M|re.S);assert len(f)==1
Path(sys.argv[1]).write_text(f[0])
PYPARSE
SPINDLE_PROBE_PARSE_SOURCE="$TEST_DIR/probe_parse.rs" rustc --edition=2021 --test tests/spindle_probe.rs -o "$TEST_DIR/probes"
"$TEST_DIR/probes"
python3 - "$TEST_DIR/document_caller.rs" <<'PYCALLER'
from pathlib import Path
import re,sys
s=Path('servers/sexfiles/src/trampoline.rs').read_text()
f=re.findall(r'^fn document_caller_live\b.*?^}',s,re.M|re.S);assert len(f)==1
assert 'let live=document_caller_live(caller);' in s
Path(sys.argv[1]).write_text(f[0])
PYCALLER
DOCUMENT_CALLER_SOURCE="$TEST_DIR/document_caller.rs" rustc --edition=2021 --test tests/document_caller.rs -o "$TEST_DIR/document-caller"
"$TEST_DIR/document-caller"
rustc --edition=2021 --test tests/quil_document.rs -o "$TEST_DIR/documents"
"$TEST_DIR/documents"
rustc --edition=2021 --test tests/document_journal.rs -o "$TEST_DIR/document-journal"
"$TEST_DIR/document-journal"
rustc --edition=2021 --test tests/display_text_transaction.rs -o "$TEST_DIR/text-transaction"
"$TEST_DIR/text-transaction"
rustc --edition=2021 --test tests/quil_view.rs -o "$TEST_DIR/quil-view"
"$TEST_DIR/quil-view"
python3 - "$TEST_DIR/workspace-refresh.rs" <<'PYWORKSPACE'
from pathlib import Path
import re,sys
s=Path('servers/silk-shell/src/main.rs').read_text()
parts=[]
for name in ['workspace_refresh_receive','workspace_refresh_poll']:
 f=re.findall(r'^unsafe fn '+name+r'\b.*?^}',s,re.M|re.S);assert len(f)==1
 parts.append(f[0])
Path(sys.argv[1]).write_text('\n'.join(parts))
assert 'unsafe { workspace_refresh_poll(); shell_lifecycle_poll();' in s
assert 'WORKSPACE_REFRESH_PENDING.iter().any(|p|*p)' in s
assert 'if workspace_refresh_receive(&msg) { continue; }' in s
q=Path('servers/quil/src/main.rs').read_text()
assert q.index('DOCUMENT_WORKSPACE.changed();')>q.index('if event.outcome == document_exchange::Outcome::Saved {')
assert 'DOCUMENT_WORKSPACE.close();DOCUMENT_IO.close();' in q
assert 'DOCUMENT_WORKSPACE.pending()' in q and 'DOCUMENT_WORKSPACE.poll(' in q
PYWORKSPACE
WORKSPACE_REFRESH_SOURCE="$TEST_DIR/workspace-refresh.rs" rustc --edition=2021 --test tests/workspace_refresh.rs -o "$TEST_DIR/workspace-refresh"
"$TEST_DIR/workspace-refresh"
rustc --edition=2021 --test tests/boot_display.rs -o "$TEST_DIR/boot-display"
"$TEST_DIR/boot-display"
python3 - "$TEST_DIR/absolute.rs" <<'PYABS'
import re,sys
from pathlib import Path
s=Path('servers/sexinput/src/main.rs').read_text()
f=re.findall(r'^fn normalize_pointer_report_v1\b.*?^}',s,re.M|re.S);assert len(f)==1
Path(sys.argv[1]).write_text(f[0])
PYABS
ABS_POINTER_PRODUCTION="$TEST_DIR/absolute.rs" rustc --edition=2021 --test tests/absolute_pointer.rs -o "$TEST_DIR/absolute"
"$TEST_DIR/absolute"
python3 - <<'PYPOLICY'
from pathlib import Path
s=Path('servers/silk-shell/src/main.rs').read_text()
p=s[s.index('unsafe fn collar_check_operation('):s.index('// ── C2: Collar Grant Table')]
assert p.index('if explicit_denial')<p.index('document_policy::allows(')
assert 'grant.state != CollarGrantState::Active' in p
f=s[s.index('unsafe fn open_linen_object_in_quil('):s.index('// ── C2: Collar-Gated')]
assert f.index('obj.disk_path!=0&&obj.disk_path<3')<f.index('collar_check_operation(')
PYPOLICY
rustc --edition=2021 --test tests/document_policy.rs -o "$TEST_DIR/document-policy"
"$TEST_DIR/document-policy"
python3 - "$TEST_DIR/sector_write.rs" <<'PYSECTOR'
import re,sys
from pathlib import Path
s=Path('servers/sexfiles/src/backends/diskfs.rs').read_text()
f=re.findall(r'^    pub fn diskfs_write_object_entry\b.*?^    }',s,re.M|re.S);assert len(f)==1
Path(sys.argv[1]).write_text('impl DiskFs {\n'+f[0]+'\n}')
PYSECTOR
DISKFS_SECTOR_WRITE_SOURCE="$TEST_DIR/sector_write.rs" rustc --edition=2021 --test tests/diskfs_sector_write.rs -o "$TEST_DIR/sector-write"
"$TEST_DIR/sector-write"
rustc --edition=2021 --test tests/sexfiles_deferred.rs -o "$TEST_DIR/deferred"
"$TEST_DIR/deferred"
python3 - <<'PYKEYS'
from pathlib import Path
s=Path('servers/silk-shell/src/main.rs').read_text()
for number,name in [(100,'APP'),(101,'STATIC'),(102,'TEST3'),(103,'TEST4')]:
    assert f'static mut SURFACE_{number}_ALIVE: bool = false;' in s
    assert f'lifecycle_register(SURFACE_ID_{name}, LifecycleState::Allocated);' in s
assert s.count('desktop_shortcut(scancode,value)')==2
for start,end in [('unsafe fn handle_hid_event(', '// ── Spindle text key passthrough'),('// ── Event-class dispatch','// ── Spindle focused-surface')]:
    part=s[s.index(start):s.index(end,s.index(start))]
    assert 'desktop_shortcut(scancode,value)' in part
PYKEYS
rustc --edition=2021 --test tests/desktop_keys.rs -o "$TEST_DIR/desktop-keys"
"$TEST_DIR/desktop-keys"
python3 - "$TEST_DIR/cursor.rs" "$TEST_DIR/cursor_status.rs" <<'PYCURSOR'
from pathlib import Path
import re,sys
s=Path('servers/silk-shell/src/main.rs').read_text()
parts=[]
for name in ['flush_pending_cursor_send','send_cursor_checked']:
    f=re.findall(r'^unsafe fn '+name+r'\b.*?^}',s,re.M|re.S);assert len(f)==1;parts.append(f[0])
Path(sys.argv[1]).write_text('\n'.join(parts))
pdx=Path('crates/sex-pdx/src/lib.rs').read_text()
Path(sys.argv[2]).write_text(next(line for line in pdx.splitlines()if line.startswith('pub const ERR_SERVICE_NOT_READY:'))+'\n')
quil=Path('servers/quil/src/main.rs').read_text()
viewport=quil[quil.index('if msg.type_id==sex_pdx::document_view::OP{'):quil.index('if msg.type_id==sex_pdx::document_view::ACTION{')]
assert viewport.index('Notice::decode(')<viewport.index('DOCUMENT_DISPLAY.rebind();')<viewport.index('draw_text_lines(')
restore=s[s.index('unsafe fn restore_minimized_frame('):s.index('unsafe fn native_child_geometry(')]
assert restore.index('DOCUMENT_VIEWS[index].refresh();')<restore.index('set_frame_minimized(frame_id, false);')
assert 'CURSOR_SEND_PENDING || WORKSPACE_REFRESH_PENDING.iter().any(|p|*p) || linen_text_can_progress()' in s
PYCURSOR
CURSOR_DELIVERY_SOURCE="$TEST_DIR/cursor.rs" CURSOR_STATUS_SOURCE="$TEST_DIR/cursor_status.rs" rustc --edition=2021 --test tests/desktop_cursor_delivery.rs -o "$TEST_DIR/cursor"
"$TEST_DIR/cursor"
echo '[service.custody.host.gate] PASS scope=production_custody_ramfs_and_history_with_mock_transport_authority_and_host_locks'
