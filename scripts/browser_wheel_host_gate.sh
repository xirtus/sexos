#!/usr/bin/env bash
set -euo pipefail
WHEEL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHEEL_TEMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-browser-wheel.XXXXXX")"
trap 'rm -rf "$WHEEL_TEMP"' EXIT
python3 - "$WHEEL_ROOT" "$WHEEL_TEMP/production.rs" <<'PY'
from pathlib import Path
import re,sys
source=(Path(sys.argv[1])/'apps/kaleidoscope/src/main.rs').read_text();parts=[]
for name in ('visible_reader','scroll_input','receive_wheel'):
 found=re.findall(r'^    fn '+name+r'\b.*?^    }',source,re.M|re.S)
 assert len(found)==1,name
 parts+=found
assert source.count('self.receive_wheel(&req);')==1
Path(sys.argv[2]).write_text('impl App {\n'+'\n'.join(parts)+'\n}\n')
PY
rustc --edition=2021 --crate-name sex_net_config --crate-type rlib "$WHEEL_ROOT/crates/sex-net-config/src/lib.rs" -o "$WHEEL_TEMP/libsex_net_config.rlib"
BROWSER_WHEEL_PRODUCTION="$WHEEL_TEMP/production.rs" rustc --edition=2021 --extern sex_net_config="$WHEEL_TEMP/libsex_net_config.rlib" --test "$WHEEL_ROOT/tests/browser_wheel.rs" -o "$WHEEL_TEMP/tests"
"$WHEEL_TEMP/tests"
echo '[browser.wheel.host] PASS scope=actual_reader_adapter_sender_validation_with_host_sources_not_native_authority_or_pixels'
