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
for name in ['scene_bar_blur_rows','bg','vertical_gradient_xrgb','lerp_color_xrgb','clamp_surface','composite_pixel','composite_pixel_selected','composite_pixel_source','bitmap_viewport','bitmap_color','bitmap_surface_damage','fill_rect_color','ascii_glyph_index','scalar_glyph','toolbar_title_fg_at','surface_text_scale','recompute_text_lines','legacy_text_clear','legacy_text_draw','surface_text_fg_at','raster_damage','text_surface_damage','app_indicator_color','phase5_active_app_at','top_strip_pixel']:
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
for name in ['raster_full','raster_strip']:
 body=function(name)
 assert "target: &frame_output::Target<'_>" in body
 assert 'HIGH_HALF_BASE' not in body
 assert 'SceneLease::capture(w,h,bar)' in body
 assert 'top_strip_pixel(x, y, bar, &clock_lines, scene)' in body
for name,paint in [('render','Full'),('redraw_top_strip','Strip'),('redraw_surface_area','Area'),('redraw_cursor','Cursor')]:
 assert 'render_prepared(fb,w,h,bar,Paint::'+paint+')' in re.sub(r'\s+','',function(name))
assert 'render_prepared(fb,w,h,bar,Paint::Damage(rect))' in re.sub(r'\s+','',function('redraw_surface_damage'))
prepared=re.sub(r'\s+','',function('render_prepared'))
assert 'store.prepare(matches!(paint,Paint::Full))' in prepared
for call in ['raster_full(&target,&captured_bar)','raster_strip(&target,&captured_bar)','raster_area(&target,&captured_bar)','raster_damage(&target,rect,&captured_bar)','raster_cursor(&target,&captured_bar)']:assert call in prepared
assert prepared.index('preparation.commit(|pixels|publish_frame(')<prepared.index('record_cursor_presented();')<prepared.index('report_app_presented();')
assert prepared.index('preparation.commit(|pixels|publish_frame(')<prepared.index('civil_clock_drawn(')
raster=function('raster_damage')
assert 'let fb=target.pointer();' in raster and 'SceneLease::capture(w,h,bar)' in raster
assert 'HIGH_HALF_BASE' not in raster
assert 'report_app_presented(' not in raster and 'publish_frame(' not in raster
focus=s[s.index('                0xED => {'):s.index('                0xEE => {')]
assert 'needs_top_strip_redraw |= changed;' in focus
assert 'stage.retire(info.target_pd);'  in s and 'if !SURFACES[index].active { stage.reset(); }' in s
assert 'if let Some(index)=vacant {' in s and 'TEXT_STAGES[index].reset();' in s and 'BITMAP_STORES.reset_surface(' in s
destroy=s[s.index('                0xEE => {'):s.index('                0xF5 => {')]
assert destroy.index('TEXT_STAGES[index].reset();')<destroy.index('*slot = SURFACE_EMPTY;')
assert 'text_damage.rect().filter(|_| fb_live && !transaction_full_render)' in s
assert '0xFD | 0x276 | 0x277 | 0x278 | 0x27A | 0x27B | 0x27C | 0x27D | 0x28B)' in s
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
DISPLAY_COMPOSITION_SOURCE="$TEST_DIR/production.rs" rustc --extern silkbar_model="$TEST_DIR/libsilkbar_model.rlib" --edition=2021 --test "$ROOT_DIR/tests/display_surface_groups.rs" -o "$TEST_DIR/tests"
RUST_MIN_STACK=16777216 "$TEST_DIR/tests" --test-threads=1
rustc --edition=2021 --test "$ROOT_DIR/tests/display_bitmap_transaction.rs" -o "$TEST_DIR/bitmap-tests"
"$TEST_DIR/bitmap-tests" --test-threads=1
python3 "$ROOT_DIR/scripts/check_display_bitmap_binding.py" --root "$ROOT_DIR"
PYTHONDONTWRITEBYTECODE=1 python3 "$ROOT_DIR/scripts/check_bitmap_stores.py" --root "$ROOT_DIR"
echo '[gate.display_surface_groups.result] PASS scope=actual_compositor_and_target_pixels_host_OutputBoundary_not_Main_receipts_or_guest_screenshot'
