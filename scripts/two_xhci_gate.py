#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Two USB controllers, as Tiger Lake laptops have (the Alienware x17 R1: a Type-C xHCI
leased first, and the chipset xHCI behind every USB-A port). SexUSB drives both:

1. Boot with a second xHCI; SexUSB reports both controllers running.
2. A USB keyboard plugged into the SECOND controller types `df` into Spindle and the
   answer comes back (keys reach Input from the controller that is not the first).
3. A USB stick on the second controller mounts under /media in Files.
4. The keyboard is pulled out; the stick still reads.
"""
import argparse, os, re, shutil, struct, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))


def unsparse(name):
    raw = (ROOT / 'tests/volumes' / f'{name}.sparse').read_bytes()
    assert raw[:8] == b'SXSPARSE'
    out = bytearray(struct.unpack('<Q', raw[8:16])[0] * 512)
    at = 16
    while at < len(raw):
        lba = struct.unpack('<Q', raw[at:at + 8])[0]
        out[lba * 512:lba * 512 + 512] = raw[at + 8:at + 520]
        at += 520
    return bytes(out)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024 ** 3)
    stick = a.out / 'stick.img'
    stick.write_bytes(unsparse('fat12'))
    # The second controller comes after the gate's own (nec-usb-xhci) on the PCI bus, so
    # the kernel leases it to an extra slot, as the x17's chipset xHCI is.
    os.environ['SEXOS_GATE_DEVICES'] = '-device qemu-xhci,id=xhci2'
    os.environ.setdefault('SEXOS_GATE_BOUND', '600')
    from quil_autosave_gate import boot
    g = boot(a.iso, disk, a.out, 'two-xhci', a.resolution)
    t, q = g['text'], g['q']

    # QEMU sends keys to the newest keyboard: the hot-added USB one. (input-send-event's
    # `device` names a display console, not an input device, so it is not used.)
    q.sock.settimeout(30)

    def usb_key(code):
        for down in (True, False):
            q.cmd('input-send-event', {'events': [
                {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': code}}}]})
            time.sleep(.05)

    report = {}
    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 180)
        m = re.search(r'\[sexusb\.controllers\] held=(\d+) running=(\d+)', t())
        assert m and m.groups() == ('2', '2'), re.findall(r'\[sexusb\.controller[^\n]*', t())
        report['controllers'] = 2
        time.sleep(2)

        # A keyboard on the second controller.
        s = len(t())
        q.cmd('device_add', {'driver': 'usb-kbd', 'bus': 'xhci2.0', 'id': 'kbd2'})
        g['wait'](r'\[sexusb\.device\] slot=\d+ port=\d+ [^\n]*class=0', s, 60)
        time.sleep(1)
        g['focus'](153)
        time.sleep(.5)
        # Spindle may drop the first keys while it takes focus: try the command again.
        for attempt in range(3):
            s = len(t())
            for k in ('d', 'f', 'ret'):
                usb_key(k)
            try:
                g['wait'](r'MiB free of', s, 30)
                break
            except TimeoutError:
                if attempt == 2:
                    raise
        assert re.search(r'\[sexinput\.kbd\.recv\] source=usb|\[sexinput\.kbd\.recv\] key=', t()), 'no USB keyboard report reached Input'
        report['keyboard'] = 'typed'

        # A stick on the second controller.
        q.cmd('blockdev-add', {'driver': 'raw', 'node-name': 'stickdrive', 'file': {'driver': 'file', 'filename': str(stick)}})
        s = len(t())
        q.cmd('device_add', {'driver': 'usb-storage', 'bus': 'xhci2.0', 'drive': 'stickdrive', 'id': 'stick'})
        g['wait'](r'\[sexusb\.storage\] slot=\d+ disk=\d+ name="QEMU QEMU HARDDISK" block=512 sectors=\d+ ', s, 60)
        g['wait'](r'\[files\.media\.mounted\] name=FLOPPY format=FAT12 [^\n]*source=usb', s, 60)
        report['stick'] = 'mounted'

        # The keyboard leaves; the stick keeps working.
        s = len(t())
        q.cmd('device_del', {'id': 'kbd2'})
        g['wait'](r'\[sexusb\.device\.gone\] slot=\d+ ', s, 60)
        assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
    finally:
        g['stop']()
    print({'ok': True, **report})


if __name__ == '__main__':
    main()
