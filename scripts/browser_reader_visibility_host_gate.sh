#!/usr/bin/env bash
set -euo pipefail
VISIBILITY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VISIBILITY_TEMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-browser-reader-visibility.XXXXXX")"
trap 'rm -rf "$VISIBILITY_TEMP"' EXIT
python3 - "$VISIBILITY_ROOT" "$VISIBILITY_TEMP" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1]);out=Path(sys.argv[2]);source=(root/'apps/kaleidoscope/src/main.rs').read_text();parts=[]
for name in ('reader_body','https_body','has_https_body','visible_reader','show_import_panel','dismiss_import','dismiss_import_key','clear_selection','sync_selection','key_input','selection_horizontal','scroll_input','navigate_reader','clipboard_events','begin_import'):
 found=list(re.finditer(r'^    fn '+name+r'\b',source,re.M));assert len(found)==1,name
 start=found[0].start();opening=source.index('{',found[0].end());depth=0
 for at in range(opening,len(source)):
  depth+=(source[at]=='{')-(source[at]=='}')
  if depth==0:parts.append(source[start:at+1]);break
 else:raise AssertionError(name)
production='impl App {\n'+'\n'.join(parts)+'\n}\n';(out/'production.rs').write_text(production)
old='self.reading&&!self.asset_panel&&!self.import_panel';assert production.count(old)==1
(out/'mutant.rs').write_text(production.replace(old,'self.reading&&!self.asset_panel'))
assert 'if self.dismiss_import_key(code){return;}' in source
assert 'code=>self.navigate_reader(code),' in source
assert '}else if self.visible_reader()&&self.reading_https{' in source
assert '}else if self.visible_reader()&&!self.reading_https{' in source
assert 'else if self.reading&&self.http.view()' not in source
PY
rustc --edition=2021 --crate-name sex_net_config --crate-type rlib "$VISIBILITY_ROOT/crates/sex-net-config/src/lib.rs" -o "$VISIBILITY_TEMP/libsex_net_config.rlib"
BROWSER_READER_VISIBILITY="$VISIBILITY_TEMP/production.rs" rustc --edition=2021 --extern sex_net_config="$VISIBILITY_TEMP/libsex_net_config.rlib" --test "$VISIBILITY_ROOT/tests/browser_reader_visibility.rs" -o "$VISIBILITY_TEMP/tests"
"$VISIBILITY_TEMP/tests" --test-threads=1
BROWSER_READER_VISIBILITY="$VISIBILITY_TEMP/mutant.rs" rustc --edition=2021 --extern sex_net_config="$VISIBILITY_TEMP/libsex_net_config.rlib" --test "$VISIBILITY_ROOT/tests/browser_reader_visibility.rs" -o "$VISIBILITY_TEMP/mutant-tests"
for VISIBILITY_TEST in hidden_import_overlay_refuses_new_export_and_all_selection_navigation hidden_overlay_exports_no_new_plaintext_request; do
  set +e
  "$VISIBILITY_TEMP/mutant-tests" --exact "$VISIBILITY_TEST" --test-threads=1 > "$VISIBILITY_TEMP/$VISIBILITY_TEST.log" 2>&1
  VISIBILITY_STATUS=$?
  set -e
  if [[ "$VISIBILITY_STATUS" != 101 ]]; then cat "$VISIBILITY_TEMP/$VISIBILITY_TEST.log"; echo '[browser.reader.visibility.negative] FAIL'; exit 1; fi
  cat "$VISIBILITY_TEMP/$VISIBILITY_TEST.log"
done
echo '[browser.reader.visibility.host] PASS scope=actual_App_methods_Reader_selection_clipboard_with_host_sources_not_native_authority_or_pixels negative_guard_removed_rejected=2'
