#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-desktop-content.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
python3 - "$ROOT" "$TMP/production.rs" <<'PY'
import pathlib,re,sys
s=(pathlib.Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
def function(name):
 m=re.findall(r'^unsafe fn '+name+r'\b.*?^\}',s,re.M|re.S)
 assert len(m)==1, name
 return m[0]
p=function('linen_render_object_list')
assert 'LINEN_OBJECTS.iter().flatten()' in p and 'LINEN_ASYNC' not in p
assert 'desktop_content::list(' in p and 'linen_queue_text(&view.text[..view.len]' in p
assert '0xFA,SURFACE_ID_LINEN' not in p
n=function('native_child_geometry')
assert '!e.dead && !e.close_pending && e.shell_surface()==Some(frame)' in n
assert 'desktop_content::children(' in n and '0xEC,child.sid' in n
assert 'native_child_geometry(surface_id,x,y,w,h)' in function('update_local_geometry')
focus=function('desktop_state_poll')
assert focus.index('DESKTOP_STATES.poll(')<focus.index('DISPLAY_FOCUS.poll(')
assert 'surface_is_alive(sid) && surface_is_lifecycle_focusable(sid) && !is_tombstoned(sid)' in focus
assert '!DESKTOP_STATES.pending_surface(sid)' in focus
assert 'DISPLAY_FOCUS.observe(arg0,admission)' in s and 'slot==SLOT_DISPLAY && opcode==0xED' in s
draw=re.findall(r'^fn shell_draw_text\b.*?^\}',s,re.M|re.S)
assert len(draw)==1
pathlib.Path(sys.argv[2]).write_text('\n'.join([p,n,draw[0],function('linen_queue_text')]))
PY
DESKTOP_CONTENT_PRODUCTION="$TMP/production.rs" rustc --edition=2021 --test "$ROOT/tests/desktop_content.rs" -o "$TMP/test"
"$TMP/test"
echo '[gate.desktop_content_host.result] PASS scope=production_builder_geometry_and_wiring'
