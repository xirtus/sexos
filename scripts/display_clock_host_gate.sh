#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-display-clock.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
python3 - "$ROOT" "$TMP/production.rs" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1]);s=(root/'servers/sexdisplay/src/main.rs').read_text();parts=[]
for name in ['module_rect','ascii_glyph_index','clock_fg_at']:
 m=re.findall(r'^fn '+name+r'\b.*?^\}',s,re.M|re.S);assert len(m)==1,name;parts+=m
for name in ['FONT','FONT_ASCII_5X7']:
 m=re.findall(r'^const '+name+r':.*?^\];',s,re.M|re.S);assert len(m)==1,name;parts+=m
assert 'silkbar_model::for_desktop_width(unsafe{FB_W}as usize)'in s
assert 'sex_pdx::serial_record!("[sexdisplay.clock.presentation]'in s
assert 'silkbar_model::for_desktop_width(desktop_width()as usize)'in(root/'servers/silk-shell/src/main.rs').read_text()
shell=(root/'servers/silk-shell/src/main.rs').read_text()
m=re.findall(r'^fn clamp_position\b.*?^\}',shell,re.M|re.S);assert len(m)==1;parts+=m
toggle=re.findall(r'^unsafe fn toggle_os_panel\b.*?^\}',shell,re.M|re.S);assert len(toggle)==1
assert 'let(x,y)=clamp_position(x as i32,y as i32,w,h);'in toggle[0]
assert toggle[0].index('clamp_position(')<toggle[0].index('pdx_call(SLOT_DISPLAY, 0xEC')
Path(sys.argv[2]).write_text('\n'.join(parts)+'\n')
PY
DISPLAY_CLOCK_PRODUCTION="$TMP/production.rs" rustc --edition=2021 --test "$ROOT/tests/display_clock.rs" -o "$TMP/test"
"$TMP/test"
echo '[gate.display_clock_host.result] PASS scope=actual_clock_glyphs_responsive_geometry_and_hit_regions_not_native_pixels_or_wall_time'
