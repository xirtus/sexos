#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEXT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-browser-text.XXXXXX")"
trap 'rm -rf "$TEXT_DIR"' EXIT
python3 - "$ROOT_DIR/apps/kaleidoscope/src/main.rs" "$TEXT_DIR/boundary.rs" <<'PY'
from pathlib import Path
import re,sys
s=Path(sys.argv[1]).read_text();parts=[]
for name in ('https_body','begin_import','submit_import','selection_horizontal'):
 found=re.findall(r'^    fn '+name+r'\b.*?^    }',s,re.M|re.S)
 assert len(found)==1,name
 parts.extend(found)
Path(sys.argv[2]).write_text('impl App {\n'+'\n'.join(parts)+'\n}')
key=re.findall(r'^    fn key_input\b.*?^    }',s,re.M|re.S)[0]
assert 'self.selection_horizontal(key.code==0x6a,key.shift)'in key
assert 'self.selection.left('not in key and 'self.selection.right('not in key
assert 'document_reader::replace_footer(&mut text.bytes,text.len,cols,label)'in s
assert 'let at=(rows-1)*(cols+1)'not in s
PY
BROWSER_TEXT_BOUNDARY="$TEXT_DIR/boundary.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/browser_text_boundary.rs" -o "$TEXT_DIR/boundary-tests"
"$TEXT_DIR/boundary-tests"
echo '[browser.text.host] PASS scope=actual_UTF8_selection_and_import_adapters_with_host_custody_callbacks_not_native_authority_storage_or_pixels'
