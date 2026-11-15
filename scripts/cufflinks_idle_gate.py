#!/usr/bin/env python3
"""Cufflinks (Bluetooth) boots and idles. Reads the serial log of the main boot gate
(master_runtime_gate.sh --keep-log); QEMU has no Bluetooth controller (usb-bt-dongle is
gone upstream), so this checks the no-controller path only:

- BOOT spawns Cufflinks (domain 52, Senses key 4) and pairs the HCI region with SexUSB,
  the audio region with Bell and the Bluetooth panel region (W2b) with the Shell;
- Cufflinks finds the region, logs `[cufflinks.idle] controller=none` once and sleeps;
- it never panics, never repeats its start lines, and stays off the CPU once idle: the
  scheduler's per-second active-domain masks (`[sched.active] ... domains=0x...`) after
  the idle line must not name domain 52 more than once.

The controller path itself (firmware, pairing, HID) is covered by bt_host_gate.sh and
x17 receipts."""
import argparse, re, sys
from pathlib import Path

PD = 52


def check(text):
    fails = []
    need = [
        (r'\[kernel\.spawn\.cufflinks\] id=52 key=4 organ=senses', 'BOOT did not spawn Cufflinks'),
        (r'\[boot\.bt\.hci\] usb=5 cufflinks=52 files=11 slot=1003 bytes=131072 runtime_mint=0', 'HCI region not paired'),
        (r'\[cufflinks\.start\]', 'Cufflinks never ran'),
        (r'\[cufflinks\.transport\] ok=1 slot=1003', 'Cufflinks did not find its region'),
        (r'\[sexusb\.bt\.route\] region=1', 'SexUSB did not find the region'),
        (r'\[boot\.bt\.audio\] bell=[1-9]\d* cufflinks=52 slot=1006 bytes=262144 runtime_mint=0', 'Bluetooth audio region not paired with Bell'),
        (r'\[cufflinks\.audio\.route\] ok=1 slot=1006', 'Cufflinks did not find the Bell audio region'),
        (r'\[bell\.bt\.route\] region=1', 'Bell did not find the Bluetooth audio region'),
        (r'\[boot\.bt\.ui\] shell=3 cufflinks=52 slot=1007 notify=1008 runtime_mint=0', 'Bluetooth panel region not paired with the Shell'),
        (r'\[cufflinks\.ui\.route\] ok=1 slot=1007', 'Cufflinks did not find the Bluetooth panel region'),
        (r'\[shell\.bt\.route\] ok=1 slot=1007', 'the Shell did not find the Bluetooth panel region'),
    ]
    for pattern, why in need:
        if not re.search(pattern, text): fails.append(why)
    controller = re.search(r'\[sexusb\.bt\] vendor=', text)
    idle = text.find('[cufflinks.idle] controller=none')
    if not controller and idle < 0: fails.append('no [cufflinks.idle] controller=none line')
    if '[cufflinks.panic]' in text: fails.append('Cufflinks panicked')
    if len(re.findall(r'\[cufflinks\.(start|idle|transport)\]', text)) > 3: fails.append('Cufflinks restarted or loops on its start lines')
    woke = total = 0
    if idle >= 0:
        for m in re.finditer(r'\[sched\.active\] tick=\d+ second_distinct=\d+ window_peak=\d+ switches=\d+ domains=(0x[0-9a-f]+)', text[idle:]):
            total += 1
            if int(m[1], 16) >> PD & 1: woke += 1
        # The first mask after the idle line may still hold the second it went idle in.
        if woke > 1: fails.append(f'Cufflinks ran in {woke} of {total} idle seconds (it should sleep)')
    return fails, {'controller': bool(controller), 'idle_seconds': total, 'cufflinks_active_seconds': woke}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--log', type=Path, required=True)
    a = p.parse_args()
    text = a.log.read_text(errors='replace')
    fails, info = check(text)
    for f in fails: print(f'[gate.cufflinks.idle] FAIL {f}')
    print(f'[gate.cufflinks.idle] {"FAIL" if fails else "PASS"} {info}')
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
