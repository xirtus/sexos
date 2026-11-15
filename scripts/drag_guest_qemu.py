#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Drag D2 test guest against QEMU's own devices (host half of the D2 gate).

The in-tree test guest (crates/drag-vmm/guest/drag_test_guest.bin) is a bzImage. QEMU's
`-M microvm -kernel` loads it the way a boot loader does and runs its real-mode setup
code, which enters the same 64-bit code Drag enters directly. The guest then talks to
QEMU's 16550 and QEMU's virtio-mmio virtio-blk (modern), found from the
`virtio_mmio.device=` word QEMU puts on its command line, exactly as it talks to
drag-vmm's devices under Drag. This proves the guest itself (boot protocol, polled UART,
virtio-blk driver) without VMX; drag-vmm's side of the same conversation is proven by its
host tests, and the two together by `drag_gate.py --accel kvm` on a host with VMX.

TCG only; no /dev/kvm needed. Prints `[drag.guest.qemu] PASS` or a SKIP line when there
is no qemu-system-x86_64, and exits non-zero on a mismatch.
"""
import os, re, shutil, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GUEST = ROOT / 'crates/drag-vmm/guest/drag_test_guest.bin'
LIB = (ROOT / 'crates/drag-vmm/src/lib.rs').read_text()
LINE = re.search(r'TEST_DISK_LINE: &\[u8\] = b"([^"]*)"', LIB)[1]
MARK = re.search(r'TEST_GUEST_MARK: &\[u8\] = b"([^"]*)"', LIB)[1]


def main():
    qemu = shutil.which('qemu-system-x86_64')
    if not qemu:
        print('[drag.guest.qemu] SKIP no qemu-system-x86_64 on this host')
        return 0
    with tempfile.TemporaryDirectory(prefix='sexos-drag-guest.') as tmp:
        disk, out = Path(tmp) / 'disk.img', Path(tmp) / 'serial.txt'
        image = bytearray(64 * 1024)
        image[:len(LINE) + 1] = (LINE + '\n').encode()
        disk.write_bytes(bytes(image))
        cmd = [qemu, '-M', 'microvm,acpi=off', '-m', '64', '-no-reboot', '-display', 'none', '-monitor', 'none',
               '-kernel', str(GUEST), '-append', 'console=ttyS0',
               '-global', 'virtio-mmio.force-legacy=false',
               '-drive', f'id=d0,file={disk},format=raw,if=none', '-device', 'virtio-blk-device,drive=d0',
               '-serial', f'file:{out}']
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            deadline = time.time() + 60
            while time.time() < deadline:
                text = out.read_text(errors='replace') if out.exists() else ''
                if '[guest] done' in text or '[guest] FAILED' in text or '[guest] no virtio-blk' in text:
                    break
                if proc.poll() is not None:
                    break
                time.sleep(0.2)
        finally:
            proc.kill()
            log = proc.communicate()[0].decode(errors='replace')
        text = out.read_text(errors='replace') if out.exists() else ''
        lines = [l for l in text.splitlines() if l.startswith('[guest]')]
        want = [
            '[guest] Drag D2 test guest: 64-bit entry through the Linux boot protocol',
            None,  # e820 count: QEMU's BIOS map, not Drag's
            None,  # cmdline: QEMU's
            None,  # virtio-blk address: QEMU's
            f'[guest] disk: {LINE}',
            '[guest] disk write and read back ok',
            '[guest] done',
        ]
        ok = len(lines) == len(want) and all(w is None or w == l for w, l in zip(want, lines))
        ok = ok and re.match(r'\[guest\] cmdline: console=ttyS0 virtio_mmio\.device=\d+@0x[0-9a-f]+:\d+', lines[2]) is not None
        ok = ok and re.match(r'\[guest\] virtio-blk at 0x[0-9a-f]+ sectors 0x80$', lines[3]) is not None
        sector1 = disk.read_bytes()[512:512 + len(MARK)].decode(errors='replace')
        ok = ok and sector1 == MARK
        for l in lines:
            print(f'  {l}')
        if not ok:
            print(f'[drag.guest.qemu] FAIL sector1={sector1!r} qemu={log.strip()!r}')
            return 1
        print('[drag.guest.qemu] PASS guest=drag_test_guest.bin devices=qemu-microvm-16550+virtio-mmio-blk')
        return 0


if __name__ == '__main__':
    sys.exit(main())
