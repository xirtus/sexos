#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-launcher.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/launcher.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
from pathlib import Path
import sys,re
s=(Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
parts=re.findall(r'^fn native_launcher_status\b.*?^}',s,re.M|re.S)
assert len(parts)==1
Path(sys.argv[2]).write_text(parts[0])
assert 'launcher::resolve(' in s
assert 'sex_pdx::app_lifecycle::catalog(e.slot)' in s
assert 'sex_pdx::app_lifecycle::info(e.slot)' in s
assert 'Command::FocusQuil => quil_launcher_status()' in s
assert 'quil_activation_info(true)!=Ok(active)' in s
assert 'Command::FocusSpindle => spindle_launcher_status()' in s
assert s.count('if palette_execute_selected() { toggle_command_palette(); }')==2
assert 'PALETTE_TEXT.poll(4,display_admit)' in s
assert 'palette_text_poll(); linen_text_poll();' in s
print('[launcher.host] PASS scope=exact_incarnation_policy_and_static_successor_wiring_guest_activation_separate')
names=['native_launcher_status','launcher_live_action','launcher_row_title','launcher_subject_live','launcher_action_record','launcher_queue_focus','launcher_palette_closed','launcher_focus_discard','launcher_focus_current','launcher_focus_retry_ready','launcher_focus_cancel','launcher_focus_poll','launcher_focus_existing','desktop_content_ready','desktop_content_blocker','desktop_state_poll','reparent_frame_to_scene','sync_scene_visibility']
parts=[]
for name in names:
 found=re.findall(r'^(?:unsafe )?fn '+name+r'\b.*?^}',s,re.M|re.S)
 assert len(found)==1,name
 parts.extend(found)
pending=re.findall(r'^#\[derive\(Clone,Copy\)\]struct LauncherFocus \{.*?^}',s,re.M|re.S)
assert len(pending)==1
parts.extend(pending)
delivery=(Path(sys.argv[1])/'servers/silk-shell/src/linen_text_delivery.rs').read_text()
parts.append(delivery[delivery.index('pub struct FocusDelivery'):])
parts.append(delivery[delivery.index('pub const DESKTOP_SURFACES'):delivery.index('/// A full display queue')].replace('DesktopStates','AdmissionStates'))
Path(sys.argv[2]+'.workspace').write_text('\n'.join(parts))
assert 'LAST_DISPLAY_FOCUS_RESULT=Some((arg0,result.0))'in s
assert 'if focus&&DESKTOP_STATES.ready(surface_id){try_set_focus(surface_id);}'in s
assert 'ok=restore_minimized_frame_inner(fid,false)'in s
assert 'desktop_state_poll(); launcher_focus_poll();'in s
assert 'APP_INPUT.pending() || LAUNCHER_FOCUS.is_some()'in s
assert 'launcher_focus_discard(); // A newer explicit focus'in s
assert 'LAST_DISPLAY_FOCUS_RESULT=Some((op.surface,0))'in s
assert 'launcher_palette_closed(launcher_pending)'in s
assert 'try_set_focus_inner(sid,true)'in s
assert 'if refresh_children {if let Some((x,y,w,h))=get_surface_bounds(sid)'in s
assert 'ok=try_set_focus_inner(frame,false)'in s
assert 'if !try_set_focus_inner(p.frame,false)'in s
for name,target,surface in [('open_quil_in_active_scene','instance.successor','instance.surface'),('open_spindle_in_active_scene','17','306')]:
 body=re.findall(r'^unsafe fn '+name+r'\b.*?^}',s,re.M|re.S)[0]
 live=body.split('if instance.state==2',1)[1].split('if instance.state==3',1)[0]
 assert 'launcher_focus_existing('+target+','+surface+',None)'in live
 assert '(true)'not in live
PY
LAUNCHER_PRODUCTION="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/launcher_authority.rs" -o "$TEST_DIR/authority"
"$TEST_DIR/authority"
LAUNCHER_WORKSPACE_PRODUCTION="$TEST_DIR/production.rs.workspace" rustc --edition=2021 --test "$ROOT_DIR/tests/launcher_workspace.rs" -o "$TEST_DIR/workspace"
"$TEST_DIR/workspace"
echo '[launcher.workspace.host] PASS scope=actual_shell_action_reparent_scene_and_admission_callbacks_only_not_native_pixels'
