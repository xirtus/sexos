#!/usr/bin/env bash
set -euo pipefail
WHEEL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHEEL_TEMP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-wheel-host.XXXXXX")"
trap 'rm -rf "$WHEEL_TEMP"' EXIT
python3 - "$WHEEL_ROOT" "$WHEEL_TEMP/production.rs" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1]);parts=[]
def extract(source,pattern):
 matches=list(re.finditer(pattern,source,re.M));assert len(matches)==1,pattern
 start=matches[0].start();opening=source.index('{',matches[0].end());depth=0
 for at in range(opening,len(source)):
  depth+=(source[at]=='{')-(source[at]=='}')
  if depth==0:return source[start:at+1]
 raise AssertionError(pattern)
for name,structs,functions in [
 ('servers/sexusb/src/main.rs',('BootMouseReport','TabletReport','UsbPointerOutbox'),('decode_boot_mouse_report','decode_tablet_report','pack_mouse_axes','pack_tablet_axes','record_device_wheel','record_wheel_admission','send_report_to_sexinput','poll_pointer_reports')),
 ('servers/sexinput/src/main.rs',('HidPointerRawReport','PointerBatch','PointerOutbox','KeyEdge','KeyboardOutbox','InputArrival'),('usb_report_sender','decode_pointer_payload','normalize_pointer_report_v1','hid_to_ps2','poll_input_delivery'))]:
 source=(root/name).read_text()
 for prefix,names in [('struct',structs),('fn',functions)]:
  for symbol in names:
   parts.append(extract(source,r'^(?:#\[derive[^\n]+\]\n)?'+prefix+' '+symbol+r'\b'))
 for symbol in ('UsbPointerOutbox',) if 'sexusb' in name else ('PointerBatch','PointerOutbox','KeyEdge','KeyboardOutbox','InputArrival'):
  parts.append(extract(source,r'^impl '+symbol+r'\b'))
 if 'sexusb' in name:
  matches=re.findall(r'^static mut (?:(?:POINTER|KEYBOARD)_OUTBOX|INPUT_ARRIVAL):[^\n]+',source,re.M)
  assert len(matches)==3;parts+=matches
# Pin fixed input identity to actual immutable BOOT construction, not fixtures.
boot=(root/'kernel/src/init.rs').read_text();creation=(root/'kernel/src/pd/create.rs').read_text()
assert '"sexdisplay", "sexdrive", "silk-shell", "sexinput", "sexusb"' in boot
assert 'let domain_id = (i + 1) as u8;' in boot
assert re.search(r'Task::new\(\s*pd_id, entry',creation)
# Verify both actual tablet call sites retain the original received buffer.
usb=(root/'servers/sexusb/src/main.rs').read_text()
assert 'let report_bytes = [rb0, rb1, rb2, rb3, rb4];' not in usb
assert usb.count('pack_tablet_axes(&td)')==2
assert usb.count('record_device_wheel(slot, s2_actual,')==2
assert usb.count('record_device_wheel(en_slot_id, intr_actual,')==2
input_source=(root/'servers/sexinput/src/main.rs').read_text()
assert 'let mut normalized_events: [(u64, u64, u64); 5]' in input_source
assert input_source.index('!usb_report_sender(&req)')<input_source.index('REAL_USB_POINTER_SEEN = true')
loop=input_source[input_source.index('serial_println!("[sexinput.shell.ready.accept]'):]
assert loop.index('input_arrival.ps2(&mut keyboard_outbox,scancode)')<loop.index('poll_input_delivery(')
assert 'pointer_outbox.poll(send_shell_hid_event)' not in loop
assert 'pdx_call(SLOT_SHELL, OP_HID_EVENT, code, value, EV_KEY)' not in loop
assert 'input_arrival.usb(&mut keyboard_outbox,keycode,modifiers,' in loop
assert 'if keyboard_outbox.available()>=10' in loop
assert '[sexinput.ps2.scancode] raw={:#x}' in loop
Path(sys.argv[2]).write_text('\n'.join(parts)+'\n')
PY
USB_WHEEL_PRODUCTION="$WHEEL_TEMP/production.rs" rustc --edition=2021 --test "$WHEEL_ROOT/tests/usb_wheel.rs" -o "$WHEEL_TEMP/tests"
"$WHEEL_TEMP/tests"
echo '[wheel.host] PASS scope=actual_decoders_normalizers_and_bounded_arrival_order_host_callbacks_not_native_authority_or_pixels'
