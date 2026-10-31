#!/usr/bin/env bash
set -euo pipefail
CIVIL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CIVIL_TEMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-civil-clock.XXXXXX")"
trap 'rm -rf "$CIVIL_TEMP"' EXIT
python3 - "$CIVIL_ROOT" "$CIVIL_TEMP" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1]);out=Path(sys.argv[2]);source=(root/'servers/sexdisplay/src/main.rs').read_text();parts=[]
for name in ['module_rect','ascii_glyph_index','civil_clock_lines','civil_clock_fg_at','civil_clock_drawn','clock_fg_at','handle_civil_clock_update','clock_canon_apply_to_bar']:
    values=re.findall(r'^fn '+name+r'\b.*?^\}',source,re.M|re.S);assert len(values)==1,name;parts+=values
for name in ['FONT','FONT_ASCII_5X7']:
    values=re.findall(r'^const '+name+r':.*?^\];',source,re.M|re.S);assert len(values)==1,name;parts+=values
for name in ['CLOCK_CANON_HH','CLOCK_CANON_MM','CLOCK_CANON_SS']:
    values=re.findall(r'^static mut '+name+r':.*?;',source,re.M);assert len(values)==1,name;parts+=values
# The actual event-loop dispatch selects the strict boundary before narrowing,
# and every canonical/demo path is gated off for ordinary civil presentation.
arm=source.split('silkbar_model::OP_SILKBAR_UPDATE => {',1)[1].split('                0x11 =>',1)[0]
assert arm.index('handle_civil_clock_update(')<arm.index('handle_silkbar_update(')
assert 'msg.caller_pd,msg.caller_task,[msg.arg0,msg.arg1,msg.arg2]'in arm
assert 'needs_top_strip_redraw|=changed;'in arm
assert 'if bar.civil_enabled && msg.arg0 as u32==UpdateKind::SetClock as u32{continue;}'in arm
assert 'if !bar.civil_enabled && !clock_from_silkbar {'in source
assert 'bar.civil_enabled=option_env!("SEXOS_LEGACY_DEMO_CLOCK").is_none();'in source
assert 'bar.civil_clock.health(raw_ticks)'in source
composer=re.findall(r'^fn top_strip_pixel\b.*?^\}',source,re.M|re.S);assert len(composer)==1 and 'clock_fg_at(x, y, bar)'in composer[0]
assert source.count('top_strip_pixel(x, y, bar)')==2
for name,path in [('render','full'),('redraw_top_strip','strip')]:
    actual=re.findall(r'^fn '+name+r'\b.*?^\}',source,re.M|re.S);assert len(actual)==1
    assert actual[0].index('core::ptr::write_volatile(')<actual[0].index('civil_clock_drawn(bar,w,h,"'+path+'")')
    assert actual[0].index('draw_launcher_panel(')<actual[0].index('civil_clock_drawn(bar,w,h,"'+path+'")')
display='\n'.join(parts)+'\n';(out/'display.rs').write_text(display)
decision='let changed=!was_enabled || old_lines!=civil_clock_lines(&bar.civil_clock);'
assert display.count(decision)==1
(out/'display-redraw-mutant.rs').write_text(display.replace(decision,'let changed=true;',1))
# Mutate the real receiver, not a substitute host implementation. The focused
# authority regression must reject a model with its nonzero wrong-task guard removed.
(out/'lib.rs').write_text((root/'crates/silkbar-model/src/lib.rs').read_text())
civil=(root/'crates/silkbar-model/src/civil_clock.rs').read_text();assert '||caller!=expected'in civil
(out/'civil_clock.rs').write_text(civil.replace('||caller!=expected','',1))
fixture=(root/'tests/silkbar_civil_clock.rs').read_text().replace('../crates/sex-pdx/src/clock_source.rs',str(root/'crates/sex-pdx/src/clock_source.rs')).replace('../crates/sex-pdx/src/desktop_style.rs',str(root/'crates/sex-pdx/src/desktop_style.rs')).replace('../crates/sex-pdx/src/app_catalog.rs',str(root/'crates/sex-pdx/src/app_catalog.rs'))
(out/'mutant.rs').write_text(fixture.replace('../crates/silkbar-model/src/lib.rs',str(out/'lib.rs')))
PY
SILKBAR_CIVIL_DISPLAY="$CIVIL_TEMP/display.rs" rustc --edition=2021 --test "$CIVIL_ROOT/tests/silkbar_civil_clock.rs" -o "$CIVIL_TEMP/tests"
"$CIVIL_TEMP/tests"
rustc --edition=2021 --crate-name silkbar_model --crate-type rlib "$CIVIL_ROOT/crates/silkbar-model/src/lib.rs" -o "$CIVIL_TEMP/libsilkbar_model.rlib"
rustc --edition=2021 --extern silkbar_model="$CIVIL_TEMP/libsilkbar_model.rlib" --test "$CIVIL_ROOT/tests/silkbar_clock_probe.rs" -o "$CIVIL_TEMP/probe-tests"
"$CIVIL_TEMP/probe-tests"
SILKBAR_CIVIL_DISPLAY="$CIVIL_TEMP/display-redraw-mutant.rs" rustc --edition=2021 --test "$CIVIL_ROOT/tests/silkbar_civil_clock.rs" -o "$CIVIL_TEMP/redraw-mutant"
if "$CIVIL_TEMP/redraw-mutant" repeated_visible_seconds_refresh_health_without_requesting_repaint; then
    echo '[silkbar.civil.host] FAIL redundant-redraw mutant survived' >&2
    exit 1
fi
SILKBAR_CIVIL_DISPLAY="$CIVIL_TEMP/display.rs" rustc --edition=2021 --test "$CIVIL_TEMP/mutant.rs" -o "$CIVIL_TEMP/mutant"
if "$CIVIL_TEMP/mutant" actual_display_boundary_authenticates_both_identity_words_before_effect; then
    echo '[silkbar.civil.host] FAIL authority mutant survived' >&2
    exit 1
fi
echo '[silkbar.civil.host] PASS scope=actual_model_Display_boundary_glyphs_not_native_authority_or_pixels authority_guard_removed_rejected=1 redundant_redraw_mutant_rejected=1'
