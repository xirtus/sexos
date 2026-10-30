#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUIL_TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-quil-clipboard-host.XXXXXX")"
trap 'rm -rf "$QUIL_TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$QUIL_TEST_DIR/actual_quil_clipboard.rs" <<'PY'
from pathlib import Path
import hashlib,re,sys
source_path=Path(sys.argv[1])/'servers/quil/src/main.rs';source=source_path.read_text()
names=('quil_source','quil_selection_bind','quil_selection_record','quil_selection_fence','quil_revision_changed','quil_fresh_source','quil_replace','quil_clipboard_status','quil_clipboard_events','quil_dispatch_key','text_buffer_undo_push','text_buffer_undo','text_buffer_redo','mark_dirty','scancode_to_char')
def extract(name):
    matches=list(re.finditer(r'^(?:unsafe )?fn '+re.escape(name)+r'\s*\(',source,re.M))
    if len(matches)!=1:raise SystemExit('actual Quil function differs: '+name)
    start=matches[0].start();i=source.index('{',matches[0].end());depth=0;state='normal';blocks=0
    while i<len(source):
        ch=source[i];pair=source[i:i+2]
        if state=='line':
            if ch=='\n':state='normal'
        elif state=='block':
            if pair=='/*':blocks+=1;i+=1
            elif pair=='*/':blocks-=1;i+=1;state='normal'if blocks==0 else'block'
        elif state in ('string','char'):
            if ch=='\\':i+=1
            elif ch==('"'if state=='string'else"'"):state='normal'
        elif pair=='//':state='line';i+=1
        elif pair=='/*':state='block';blocks=1;i+=1
        elif ch=='"':state='string'
        elif ch=="'" and (source[i+2:i+3]=="'" or source[i+1:i+2]=='\\'):state='char'
        elif ch=='{':depth+=1
        elif ch=='}':
            depth-=1
            if depth==0:return source[start:i+1]
        i+=1
    raise SystemExit('unterminated actual Quil function: '+name)
Path(sys.argv[2]).write_text('\n\n'.join(extract(name)for name in names)+'\n')
# Source wiring is separately scoped; the host functions do not execute IPC.
live=source.split('            OP_HID_EVENT => {',1)[1].split('            sex_pdx::OP_QUIL_OPEN_DISK_DOC =>',1)[0]
assert live.index('msg.caller_pd!=3||msg.caller_task!=3')<live.index('sex_pdx::app_key::Key::decode')<live.index('quil_dispatch_key(key,')
assert 'Subject{pd:msg.caller_pd,task:msg.caller_task}'in source
assert 'QUIL_CLIPBOARD.close();QUIL_SELECTION.clear();QUIL_COPY_SNAPSHOT=None;QUIL_PASTE_SNAPSHOT=None;'in source
assert 'quil_fresh_source();'in source.split('event.outcome == document_exchange::Outcome::Loaded',1)[1].split('} else {',1)[0]
assert 'quil_fresh_source();'in source.split('CMD_NEW_BUFFER_STUB => {',1)[1].split('CMD_SETTINGS_STUB =>',1)[0]
print('[gate.quil_clipboard.wiring] PASS source_sha256='+hashlib.sha256(source_path.read_bytes()).hexdigest()+' scope=source_wiring_not_executed_ipc')
PY
QUIL_CLIPBOARD_UI_SOURCE="$QUIL_TEST_DIR/actual_quil_clipboard.rs" rustc --edition=2021 -A dead_code --test "$ROOT_DIR/tests/quil_clipboard_ui.rs" -o "$QUIL_TEST_DIR/tests"
"$QUIL_TEST_DIR/tests" --test-threads=1
echo '[gate.quil_clipboard.result] PASS scope=actual_quil_editor_clipboard_events_and_undo_with_host_render_action_timer_callbacks_no_native_ipc_authority_durable_save_or_pixels_claim'
