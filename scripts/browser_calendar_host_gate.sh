#!/usr/bin/env bash
# Actual bounded clock panel with independent known-date tests and compiled
# controls. No source authenticity, syscall authority or native pixel claim.
set -euo pipefail
CALENDAR_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${BROWSER_CALENDAR_EVIDENCE_DIR:-}" ]]; then
    CALENDAR_DIR="$BROWSER_CALENDAR_EVIDENCE_DIR"
    [[ ! -e "$CALENDAR_DIR" ]] || { echo 'fresh calendar evidence directory required' >&2; exit 1; }
    mkdir -p "$CALENDAR_DIR"
else
    CALENDAR_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-browser-calendar.XXXXXX")"
    trap 'rm -rf "$CALENDAR_DIR"' EXIT
fi
rustc --edition=2021 --crate-type rlib --crate-name sex_time "$CALENDAR_ROOT/crates/sex-time/src/lib.rs" -o "$CALENDAR_DIR/libsex_time.rlib"
rustc --edition=2021 --extern sex_time="$CALENDAR_DIR/libsex_time.rlib" --test "$CALENDAR_ROOT/tests/browser_calendar.rs" -o "$CALENDAR_DIR/positive"
"$CALENDAR_DIR/positive"
python3 - "$CALENDAR_ROOT" "$CALENDAR_DIR" <<'PY'
from pathlib import Path
import hashlib,json,sys
root=Path(sys.argv[1]);out=Path(sys.argv[2]);source=(root/'apps/kaleidoscope/src/clock_view.rs').read_text();tests=(root/'tests/browser_calendar.rs').read_text()
for name in ('clock_source.rs','desktop_style.rs'):
 p=out/'crates/sex-pdx/src'/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes((root/'crates/sex-pdx/src'/name).read_bytes())
target=out/'tests';target.mkdir()
controls=[('measure','.min(63)','.min(79)','both_tiles_and_fullscreen_keep_all_eight_rows_without_hidden_enlarge_loop'),
          ('epoch','SexTimestamp::from_unix(i64::from(value), 0)','SexTimestamp::from_unix(i64::from(value) + 1, 0)','three_civil_views_match_known_epoch_relative_days_and_boundary_seconds')]
for name,old,new,test in controls:
 assert source.count(old)==1,(name,old)
 mutant=out/(name+'.rs');mutant.write_text(source.replace(old,new))
 path=target/(name+'.rs');path.write_text(tests.replace('#[path="../apps/kaleidoscope/src/clock_view.rs"]mod clock_view;','#[path='+json.dumps(str(mutant))+']mod clock_view;'))
 (out/(name+'.test')).write_text(test+'\n')
(out/'sources.json').write_text(json.dumps({name:hashlib.sha256((root/name).read_bytes()).hexdigest()for name in ('apps/kaleidoscope/src/clock_view.rs','tests/browser_calendar.rs','crates/sex-time/src/lib.rs','crates/sex-pdx/src/clock_source.rs','crates/sex-pdx/src/desktop_style.rs')},indent=2)+'\n')
PY
for CALENDAR_CONTROL in measure epoch; do
    rustc --edition=2021 --extern sex_time="$CALENDAR_DIR/libsex_time.rlib" --test "$CALENDAR_DIR/tests/$CALENDAR_CONTROL.rs" -o "$CALENDAR_DIR/$CALENDAR_CONTROL"
    CALENDAR_TEST="$(cat "$CALENDAR_DIR/$CALENDAR_CONTROL.test")"
    set +e
    "$CALENDAR_DIR/$CALENDAR_CONTROL" --exact "$CALENDAR_TEST" > "$CALENDAR_DIR/$CALENDAR_CONTROL.log" 2>&1
    CALENDAR_RESULT=$?
    set -e
    [[ "$CALENDAR_RESULT" == 101 ]] || { cat "$CALENDAR_DIR/$CALENDAR_CONTROL.log"; exit 1; }
    echo "[browser.calendar.control] name=$CALENDAR_CONTROL compiled=1 rejected=1"
done
echo '[browser.calendar.host] PASS scope=actual_clock_panel_calendar_and_bounded_rows_no_native_time_authority_or_pixels'
