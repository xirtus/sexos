#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-linen-text.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/linen_text_delivery.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PYCODE'
from pathlib import Path
import re,sys
s=(Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
def function(name):
    found=re.findall(r'^(?:unsafe )?fn '+name+r'\b.*?^\}',s,re.M|re.S)
    assert len(found)==1,name
    return found[0]
parts=[function(name) for name in ['pdx_call','desktop_state_validate','desktop_state_poll','linen_queue_text','display_admit','linen_text_can_progress','linen_text_poll']]
assert 'LINEN_TEXT.poll(4,display_admit)' in function('linen_text_poll')
assert s.count('[linen.text.present]')==1 and '[linen.text.present]' in function('linen_text_poll')
for name in ['linen_render_object_list','linen_render_static_ui']:
    body=function(name);assert 'linen_queue_text(' in body and '0xFA' not in body and '0xFB' not in body
assert 'bell_frame_id' not in function('toggle_command_palette')
assert 'minimize_frame(COMMAND_PALETTE_FRAME_ID);' in function('toggle_command_palette')
assert 'PALETTE_VISIBILITY.set(' in function('toggle_command_palette')
assert 'set_frame_minimized(COMMAND_PALETTE_FRAME_ID,false)' in function('palette_show')
assert 'shell_registration_reply_poll(); desktop_state_poll(); palette_visibility_poll(); linen_text_poll(); linen_async_poll();' in s
assert 'linen_text_can_progress() || PALETTE_VISIBILITY.pending() || DESKTOP_STATES.pending() || LINEN_ASYNC.can_send()' in s
Path(sys.argv[2]).write_text('\n'.join(parts))
PYCODE
DESKTOP_DELIVERY_PRODUCTION="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/desktop_state_delivery.rs" -o "$TEST_DIR/production-tests"
"$TEST_DIR/production-tests"
echo '[gate.linen_text_delivery.result] PASS scope=production_outbox_wrapper_validator_poll_not_guest_pixels'
