#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-display-groups.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
from pathlib import Path
import re,sys
root,out=map(Path,sys.argv[1:]);s=(root/'servers/sexdisplay/src/main.rs').read_text()
a=s.index('const FONT_ASCII_5X7:');parts=[s[a:s.index('];',s.index('= [',a))+2]]
for name in ['FONT_UNICODE_5X7','FONT_UNSUPPORTED_5X7']:
 a=s.index('const '+name+':');parts.append(s[a:s.index('];',a)+2])
parts+=re.findall(r'^const (?:FRAME_|SURFACE_CHROME_|TAB_STRIP_|BAR_BG_H|BAR_BG_W_CAP|FOCUS_SURFACE_COLOR|TAB_ACTIVE_COLOR|TAB_INACTIVE_COLOR)[^\n]+',s,re.M)
a=s.index('#[derive(Clone, Copy)]\n#[repr(C)]\nstruct RenderTokensV1');b=s.index('\nstatic mut DISPLAY_TOKENS:',a)
parts.append(s[a:b])
a=s.index('static mut PROGRAM_TITLES:');parts.append(s[a:s.index('\n',a)])
for k in ('const PROGRAM_ROWS:','static mut PIXEL_REGIONS:','static mut PIXEL_SIZE:'):
    a=s.index(k);parts.append(s[a:s.index('\n',a)])
for name in ['program_title','program_pixel_at','program_pixel_row','coverage_blend','scene_bar_blur_rows','bg','vertical_gradient_xrgb','lerp_color_xrgb','clamp_surface','composite_pixel','composite_pixel_selected','composite_pixel_source','bitmap_viewport','bitmap_color','bitmap_surface_damage','fill_rect_color','ascii_glyph_index','scalar_glyph','toolbar_title_fg_at','surface_text_scale','recompute_text_lines','apply_pending_text','text_projection_requirement','changed_text_rows','legacy_text_clear','legacy_text_draw','surface_text_fg_at','raster_batch','raster_full','raster_strip','raster_area','raster_row','fast_content_run','text_run','raster_damage','fill_bar_bg_buffer','blur_bar_bg_buffer_radius1','clock_total_seconds','clock_visible_monotonic_ok','text_surface_damage','app_indicator_color','phase5_active_app_at','top_strip_pixel']:
 a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
 while depth: depth+=(s[i]=='{')-(s[i]=='}');i+=1
 part=s[a:i]
 if name in ['composite_pixel','composite_pixel_selected','composite_pixel_source','bitmap_color','phase5_active_app_at','top_strip_pixel']:
  assert 'scene: &scene::SceneSnapshot' in part
  assert not re.search(r'\b(?:SURFACES|FRAME_GROUPS|BITMAP_STORES|FOCUSED_SURFACE_ID|DISPLAY_TOKENS|RENDER_FRAME_COUNTER)\b',part),name
 if name=='bg':part=part.replace('fn bg(', 'fn scene_bg(',1)
 if name=='scene_bar_blur_rows':part=part.replace('bg(', 'scene_bg(')
 # Declared glass boundary: preserve the reference's old four-argument API.
 part=part.replace('glass_over_bg(', 'scene_glass_over_bg(')
 parts.append(part)
def function(name):
 a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
 while depth:depth+=(s[i]=='{')-(s[i]=='}');i+=1
 return s[a:i]
for name in ['raster_full','raster_strip','raster_area','raster_damage']:
 body=function(name)
 assert "target: &frame_output::Target<'_>" in body and 'scene: &scene::SceneSnapshot' in body
 for forbidden in ['SceneLease::capture','restore_cursor_under(','draw_launcher_panel(','draw_cursor_z_top(','APP_PRESENT_BITS = 0','HIGH_HALF_BASE']:assert forbidden not in body,(name,forbidden)
prepared=re.sub(r'\s+','',function('render_prepared'))
assert prepared.count('SceneLease::capture(')==1 and prepared.count('preparation.target()')==1
assert 'store.prepare(plan.is_full())' in prepared and 'raster_batch(&target,scene,plan)' in prepared
assert prepared.index('SceneLease::capture(')<prepared.index('raster_batch(')<prepared.index('preparation.commit(')<prepared.index('report_app_presented();')
batch=re.sub(r'\s+','',function('raster_batch'))
assert 'ifplan.settle_only(){returntrue;}' in batch
assert batch.index('ifplan.settle_only(){returntrue;}')<batch.index('restore_cursor_under(')
assert 'if!settle_only{unsafe{APP_PRESENT_BITS=0;PREPARED_CURSOR=None;}}' in prepared
assert 'if!settle_only{record_cursor_presented();report_app_presented();' in prepared
assert batch.count('restore_cursor_under(')==1 and batch.count('draw_cursor_z_top(')==1 and batch.count('draw_launcher_panel(')==1
assert batch.index('restore_cursor_under(')<batch.index('raster_damage(')<batch.index('raster_strip(')<batch.index('draw_launcher_panel(')<batch.index('draw_cursor_z_top(')
assert 'draw_cursor_z_top(fb,w,h,total,true,scene);'in batch
for name in ['BAR_BG_BUF','BAR_BLUR_BUF','ROW_BUFFER_PROOF_LOGGED','BLUR_PROOF_LOGGED','ANIM_PROOF_LOGGED','R6_GLASS_PROOF_LOGGED','CLOCK_REDRAW_SOURCE','CLOCK_CANON_SS']:
 match=re.search(r'^static(?: mut)? '+name+r':[^=]+=\s*(?:\[[^\]]+\]|[^;\n]+);',s,re.M);assert match,name;parts.append(match[0])
for name in ['SILK_RENDER_PROOF_PROFILE_ENABLED','SILKBAR_PHASE3_RECEIVE_PROOF_ENABLED','SILKBAR_PHASE5_PIXEL_PROOF_ENABLED']:
 match=re.search(r'^const '+name+r':[^;]+;',s,re.M);assert match,name;parts.append(match[0])
focus=s[s.index('                0xED => {'):s.index('                0xEE => {')]
assert 'needs_top_strip_redraw |= changed;' in focus
assert 'stage.retire(info.target_pd,info.task_id);' in s and 'if !SURFACES[index].active { stage.reset(); }' in s
assert 'if let Some(index)=vacant {' in s and 'TEXT_STAGES[index].reset();' in s and 'BITMAP_STORES.reset_surface(' in s
destroy=s[s.index('                0xEE => {'):s.index('                0xF5 => {')]
assert destroy.index('TEXT_STAGES[index].reset();')<destroy.index('*slot = SURFACE_EMPTY;')
assert 'if let Some(rect)=text_damage.rect(){pending_paint.request_damage(rect,w,h);}' in s
assert '0xFD | 0x276 | 0x277 | 0x278 | 0x27A | 0x27B | 0x27C | 0x27D | 0x28B | 0x28C)' in s
fill=s[s.index('                0xEF => {'):s.index('                sex_pdx::bitmap_batch::BEGIN')]
assert 'fill_damage::update(slot,rect_index,sx,sy,sw,sh,color)' in fill
assert fill.index('slot.owner_pd != msg.caller_pd')<fill.index('fill_damage::update(')
assert 'needs_surface_redraw = true' not in fill
assert 'if sw == 0 || sh == 0 { continue; }' not in fill
for opcode,next_opcode,helper in [('0xFA','0xFB','legacy_text_clear'),('0xFB','0xFC','legacy_text_draw')]:
 part=s[s.index('                '+opcode+' => {'):s.index('                '+next_opcode+' => {')]
 assert part.index('slot.owner_pd != msg.caller_pd')<part.index(helper+'(')
 assert 'text_damage.add('in part and 'needs_surface_redraw'not in part
assert 'FRAME_GROUPS.observe(&SURFACES)' in s and 'FRAME_GROUPS.fit_children(&mut SURFACES)' in s
assert 'FRAME_GROUPS.register(surface_lifecycle::Catalog' in s
assert 'FRAME_GROUPS.awaiting_parent(surface)' in s
assert '[display.child.parent_gate] target={} surface={} parent={} observed=0 standalone=0 retained=1' in s
admit=s[s.index('                0xEC => {'):s.index('                0xDE => {')]
assert admit.index('surface_lifecycle::upsert(')<admit.index('FRAME_GROUPS.observe(')<admit.index('FRAME_GROUPS.awaiting_parent(surface)')
assert 'fill_rect_color(surf, x, y, DISPLAY_TOKENS.focus_surface_color)' not in s
out.write_text('\n'.join(parts))
PY
rustc --edition=2021 --crate-type=rlib --crate-name silkbar_model "$ROOT_DIR/crates/silkbar-model/src/lib.rs" -o "$TEST_DIR/libsilkbar_model.rlib"
# Display draws text with sex-font (and ttf-parser); build it for the host once.
cargo build -q --manifest-path "$ROOT_DIR/Cargo.toml" -p sex-font
FONT_LIBS=(--extern sex_font="$ROOT_DIR/target/debug/libsex_font.rlib" -L dependency="$ROOT_DIR/target/debug/deps")
DISPLAY_COMPOSITION_SOURCE="$TEST_DIR/production.rs" rustc --extern silkbar_model="$TEST_DIR/libsilkbar_model.rlib" "${FONT_LIBS[@]}" --edition=2021 --test "$ROOT_DIR/tests/display_surface_groups.rs" -o "$TEST_DIR/tests"
RUST_MIN_STACK=16777216 "$TEST_DIR/tests" --test-threads=1
rustc --edition=2021 --test "$ROOT_DIR/tests/display_bitmap_transaction.rs" -o "$TEST_DIR/bitmap-tests"
"$TEST_DIR/bitmap-tests" --test-threads=1
python3 "$ROOT_DIR/scripts/check_display_bitmap_binding.py" --root "$ROOT_DIR"
PYTHONDONTWRITEBYTECODE=1 python3 "$ROOT_DIR/scripts/check_bitmap_stores.py" --root "$ROOT_DIR"
echo '[gate.display_surface_groups.result] PASS scope=actual_compositor_and_target_pixels_host_OutputBoundary_not_Main_receipts_or_guest_screenshot'
