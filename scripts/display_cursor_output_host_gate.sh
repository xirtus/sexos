#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-cursor-output.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import re,sys,subprocess,os
root,out=map(Path,sys.argv[1:]);source=(root/'servers/sexdisplay/src/main.rs').read_text()
def function(name):
 a=source.index('fn '+name+'(');i=source.index('{',a)+1;depth=1
 while depth:depth+=(source[i]=='{')-(source[i]=='}');i+=1
 return source[a:i]
def guards(s):
 def body(name):
  a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
  while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
  return re.sub(r'\s+','',s[a:i])
 for name in ['raster_full','raster_strip','raster_area','raster_damage']:
  b=body(name);assert b.index('draw_launcher_panel(')<b.index('draw_cursor_z_top('),name
  assert re.search(r'draw_cursor_z_top\([^;]*,true,scene\);',b),name
 strip=body('raster_strip');assert strip.index('restore_cursor_under(')<strip.index('foryin0..51')<strip.index('draw_launcher_panel(')
 draw=body('draw_cursor_z_top');assert 'scene.surfaces.iter()' in draw
 for forbidden in ['serial_println!','serial_record!','INPUT_TRACE_PRESENTS','INPUT_TRACE_DRAWS','LAST_COUNTED_DRAW_SEQ','record_cursor_presented(','report_app_presented(']:assert forbidden not in draw,forbidden
 prepare=body('render_prepared');assert prepare.index('preparation.commit(')<prepare.index('record_cursor_presented();')<prepare.index('report_app_presented();')
 cleanup=body('prepare_app_cleanup');assert 'scrub_cursor_under();' in cleanup
 scrub=body('scrub_cursor_under');assert 'forpixelin&mutCURSOR_UNDER_SAVED{core::ptr::write_volatile(pixel,0);}'in scrub
 assert 'CURSOR_UNDER_VALID=false;'in scrub and 'CURSOR_UNDER_X=0;CURSOR_UNDER_Y=0;'in scrub
guards(source)
# Wrong production overlay order must fail the source guard independently.
for name in ['raster_full','raster_strip','raster_area','raster_damage']:
 body=function(name);launcher=re.search(r'\s*draw_launcher_panel\([^;]+;',body)[0];cursor=re.search(r'\s*draw_cursor_z_top\([^;]+;',body)[0]
 altered=body.replace(launcher,'OVERLAY_SENTINEL',1).replace(cursor,launcher,1).replace('OVERLAY_SENTINEL',cursor,1)
 try:guards(source.replace(body,altered,1))
 except AssertionError:pass
 else:raise AssertionError('overlay counterexample accepted '+name)
parts=[]
for prefix in ['MAX_FB_','FRAME_','FOCUS_SURFACE_COLOR','TAB_ACTIVE_COLOR','TAB_INACTIVE_COLOR','BAR_BG_H','CURSOR_SURFACE_ID']:
 parts+=re.findall(r'^const '+prefix+r'[^\n]+',source,re.M)
a=source.index('#[derive(Clone, Copy)]\n#[repr(C)]\nstruct RenderTokensV1');b=source.index('\nstatic mut DISPLAY_TOKENS:',a);parts.append(source[a:b])
a=source.index('const CURSOR_ARROW_W:');b=source.index('\n];',a)+3;parts.append(source[a:b])
a=source.index('static mut CURSOR_UNDER_SAVED:');b=source.index('\n',source.index('static mut CURSOR_UNDER_VALID:',a));parts.append(source[a:b])
a=source.index('#[derive(Clone,Copy)]\nstruct PreparedCursor');b=source.index('\nfn record_cursor_presented',a);parts.append(source[a:b])
for name in ['INPUT_TRACE_LAST_SEQ','INPUT_TRACE_LAST_SHELL_TICK']:parts.append(re.search(r'^static mut '+name+r':[^\n]+',source,re.M)[0])
for name in ['scene_bar_blur_rows','bg','vertical_gradient_xrgb','lerp_color_xrgb','draw_cursor_z_top','restore_cursor_under','scrub_cursor_under']:parts.append(function(name))
actual='\n'.join(parts);module=out/'actual.rs';module.write_text(actual)
rlib=out/'libsilkbar_model.rlib';subprocess.run(['rustc','--edition=2021','--crate-type=rlib','--crate-name','silkbar_model',str(root/'crates/silkbar-model/src/lib.rs'),'-o',str(rlib)],check=True)
def run(name,code,exact=None):
 module=out/(name+'.rs');module.write_text(code);binary=out/name
 env=os.environ.copy();env['DISPLAY_CURSOR_SOURCE']=str(module);env['RUST_MIN_STACK']='16777216'
 subprocess.run(['rustc','--edition=2021','--test',str(root/'tests/display_cursor_output.rs'),'--extern','silkbar_model='+str(rlib),'-o',str(binary)],env=env,check=True)
 result=subprocess.run([str(binary),'--test-threads=1',*(['--exact',exact]if exact else[])],env=env,capture_output=True,text=True)
 print(result.stdout,end='');print(result.stderr,end='');return result
assert run('actual',actual).returncode==0
controls=[('skip-restore','if !CURSOR_UNDER_VALID { return; }','if CURSOR_UNDER_VALID { return; }','full_overlap_is_visible_and_move_restores_exact_menu_without_arrow_trail'),('wrong-save-under','core::ptr::read_volatile(fb.add(idx))','0x00badbad','strip_repaint_recaptures_new_menu_before_cursor_moves_away')]
for name,old,new,test in controls:
 assert actual.count(old)==1,(name,actual.count(old));result=run(name,actual.replace(old,new),test)
 assert result.returncode!=0 and 'FAILED' in result.stdout,name
 print('[display.cursor.output.mutant] rejected='+name+' exact='+test)
print('[display.cursor.output.host] PASS actual_cases=5 compiled_runtime_mutants=2 overlay_source_controls=4 scope=actual_cursor_scene_target_pixels_host_background_menu_CPU_output_not_Main_receipts_or_hardware')
PY
