#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLIPBOARD_TEMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-clipboard-host.XXXXXX")"
trap 'rm -rf "$CLIPBOARD_TEMP"' EXIT
python3 - "$ROOT" "$CLIPBOARD_TEMP/app_input_production.rs" "$CLIPBOARD_TEMP/browser_wheel.rs" "$CLIPBOARD_TEMP/input_liveness.rs" <<'PY'
from pathlib import Path
import re,sys
source=(Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
def function(name):
    matches=list(re.finditer(r'^(?:unsafe )?fn '+name+r'\b',source,re.M))
    if len(matches)!=1:raise SystemExit('actual Shell function extraction differs: '+name)
    start=matches[0].start();opening=source.index('{',start);depth=1;end=opening+1
    while depth:
        if end>=len(source):raise SystemExit('unterminated Shell function: '+name)
        depth+=(source[end]=='{')-(source[end]=='}');end+=1
    return source[start:end]
association=('app_input_association','browser_input_participant','app_input_key_code')
parts=[function(name) for name in association+('display_scale_poll','app_input_poll','clipboard_intake_poll','clipboard_source_ready','clipboard_poll','key_repeat_poll','desktop_shortcut')]
Path(sys.argv[2]).write_text('\n'.join(parts)+'\n')
wheel=[function(name) for name in association+('chrome_metrics','app_input_participant','document_view_sender','browser_wheel_destination','browser_wheel_input','app_input_poll','frame_accepts_input')]
# Compiled in its own namespace: same production lookup and LIVE/RETRY logic,
# with only finite held catalog/info callbacks supplied by the host fixture.
Path(sys.argv[4]).write_text(function('clipboard_live')+'\n')
# Execute the actual main dispatch prefix, including IPC metadata checks,
# inside a one-pass host loop so its original continue statements retain meaning.
a=source.index('                OP_HID_EVENT => {',source.index('pub extern "C" fn _start()'))
a=source.index('\n',a)+1;b=source.index('                    // Raw HID receive proof:',a)
prefix=source[a:b]
wheel.append('unsafe fn actual_main_wheel_prefix(msg:&PdxMessage){for _ in 0..1 {\n'+prefix+'\n}}')
Path(sys.argv[3]).write_text('\n'.join(wheel)+'\n')
PY
# Shell's keyboard layouts and input method, as the crates Shell links.
rustc --edition=2021 --crate-type=rlib --crate-name sex_keymap "$ROOT/crates/sex-keymap/src/lib.rs" -o "$CLIPBOARD_TEMP/libsex_keymap.rlib"
rustc --edition=2021 --crate-type=rlib --crate-name sex_ime "$ROOT/crates/sex-ime/src/lib.rs" -o "$CLIPBOARD_TEMP/libsex_ime.rlib"
for name in text_encoding text_selection clipboard_wire clipboard_client clipboard_broker app_input; do
    APP_INPUT_PRODUCTION="$CLIPBOARD_TEMP/app_input_production.rs" rustc --edition=2021 -A dead_code --extern sex_keymap="$CLIPBOARD_TEMP/libsex_keymap.rlib" --extern sex_ime="$CLIPBOARD_TEMP/libsex_ime.rlib" --test "$ROOT/tests/$name.rs" -o "$CLIPBOARD_TEMP/$name"
    "$CLIPBOARD_TEMP/$name" --test-threads=1
done
BROWSER_INPUT_LIVENESS_PRODUCTION="$CLIPBOARD_TEMP/input_liveness.rs" BROWSER_WHEEL_PRODUCTION="$CLIPBOARD_TEMP/browser_wheel.rs" rustc --edition=2021 -A dead_code --test "$ROOT/tests/browser_wheel_route.rs" -o "$CLIPBOARD_TEMP/browser-wheel"
"$CLIPBOARD_TEMP/browser-wheel" --test-threads=1
echo '[gate.clipboard_host.result] PASS scope=actual_selection_wire_client_broker_input_and_shell_dispatch_with_host_transport_liveness_callbacks_only_no_native_authority_delivery_or_pixels_claim'
