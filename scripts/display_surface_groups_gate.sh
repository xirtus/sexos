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
parts+=re.findall(r'^const (?:FRAME_|SURFACE_CHROME_|TAB_STRIP_|BAR_BG_H)[^\n]+',s,re.M)
for name in ['clamp_surface','composite_pixel','composite_pixel_selected','fill_rect_color','ascii_glyph_index','toolbar_title_fg_at','surface_text_scale','recompute_text_lines','surface_text_fg_at','redraw_surface_damage','text_surface_damage','app_indicator_color','phase5_active_app_at','top_strip_pixel']:
 a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
 while depth: depth+=(s[i]=='{')-(s[i]=='}');i+=1
 parts.append(s[a:i])
for name in ['render','redraw_top_strip']:
 a=s.index('fn '+name+'(');b=s.index('\nfn ',a+1)
 assert 'top_strip_pixel(x, y, bar)' in s[a:b]
focus=s[s.index('                0xED => {'):s.index('                0xEE => {')]
assert 'needs_top_strip_redraw |= changed;' in focus
assert 'stage.retire(info.target_pd);'  in s and 'if !SURFACES[index].active { stage.reset(); }' in s
assert 'if let Some(index)=vacant { TEXT_STAGES[index].reset(); }' in s
destroy=s[s.index('                0xEE => {'):s.index('                0xF5 => {')]
assert destroy.index('TEXT_STAGES[index].reset();')<destroy.index('*slot = SURFACE_EMPTY;')
assert 'text_damage.rect().filter(|_| fb_live && !transaction_full_render)' in s
assert '0xFD | 0x276 | 0x277 | 0x278)' in s
fill=s[s.index('                0xEF => {'):s.index('                sex_pdx::text_batch::BEGIN')]
assert 'fill_damage::update(slot,rect_index,sx,sy,sw,sh,color)' in fill
assert fill.index('slot.owner_pd != msg.caller_pd')<fill.index('fill_damage::update(')
assert 'needs_surface_redraw = true' not in fill
assert 'if sw == 0 || sh == 0 { continue; }' not in fill
assert 'FRAME_GROUPS.observe(&SURFACES)' in s and 'FRAME_GROUPS.fit_children(&mut SURFACES)' in s
assert 'FRAME_GROUPS.register(surface_lifecycle::Catalog' in s
assert 'fill_rect_color(surf, x, y, DISPLAY_TOKENS.focus_surface_color)' not in s
out.write_text('\n'.join(parts))
PY
DISPLAY_COMPOSITION_SOURCE="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/display_surface_groups.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests" --test-threads=1
echo '[gate.display_surface_groups.result] PASS scope=actual_compositor_pixels_and_production_catalog_not_guest_screenshot'
