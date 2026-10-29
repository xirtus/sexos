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
assert s.count('if unsafe{HISTORY_IO.blocked()}{')>=2
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
echo '[service.custody.host.gate] PASS scope=production_custody_ramfs_and_history_with_mock_transport_authority_and_host_locks'
