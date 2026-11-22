#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Boot the ISO on device mixes no other QEMU gate uses and require each driver
to come up without stopping anything.

Other gates boot NVMe + e1000e + NEC xHCI + intel-hda with hda-duplex. Real
PCs differ: the x17 R1 and most laptops have Intel's own xHCI behind hubs,
desktops often an AHCI disk and no NVMe, older boards the 82540 e1000, and HDA
codecs with only an output path. Each row is one machine; all boot at once.
A row passes when the desktop is up, its driver line is there, and no panic,
crash record or service restart appears. Host-side fuzzing of the same drivers
lives in fuzz/drivers (driver-fuzz.yml).
"""
import argparse, pathlib, subprocess, sys, time

READY = b'[silkbar.ready]'
STOPPED = (b'KERNEL PANIC', b'panic_handler', b'[crash.record]', b'[service.restarted]')

# name -> (extra QEMU args, lines that must appear)
ROWS = {
    'qemu_xhci_hub': (['-device', 'qemu-xhci,id=xhci', '-device', 'usb-hub,bus=xhci.0,port=1,id=hub0',
                       '-device', 'usb-kbd,bus=xhci.0,port=1.1', '-device', 'usb-tablet,bus=xhci.0,port=1.2'],
                      [b'[sexusb.ready]', b'[sexusb.hub]']),
    'ahci_only': (['-device', 'ahci,id=ahci', '-drive', 'if=none,id=sata,file={disk},format=raw',
                   '-device', 'ide-hd,drive=sata,bus=ahci.0'],
                  [b'[sexdrive.ahci.ready]', b'ahci=1']),
    'e1000_82540': (['-netdev', 'user,id=n0', '-device', 'e1000,netdev=n0'], [b'[sexnet.ether.ready]']),
    'hda_output': (['-audiodev', 'none,id=a0', '-device', 'intel-hda', '-device', 'hda-output,audiodev=a0'],
                   [b'[bell.hda.ready]']),
    # QEMU's edu teaching device on a generic lease, driven inside Saddle by the
    # Harness SDK's template driver (crates/harness-template): bring-up, a polled
    # and an interrupt-driven factorial, and a DMA round trip through the IOMMU pool.
    'edu_saddle': (['-device', 'edu,dma_mask=0xffffffff'],
                   [b'[kernel.spawn.saddle]', b'kind=Generic', b'[edu.ready]', b'[saddle.selftest] driver=edu ok=1',
                    b'[saddle.driver] name=edu', b'state=running']),
    'bare': ([], []),
}
NVME = ['-drive', 'if=none,id=nvm,file={disk},format=raw', '-device', 'nvme,serial=sexos01,drive=nvm']


def boot(iso, name, out, seconds):
    extra, need = ROWS[name]
    disk, serial = out / f'disk-{name}.img', out / f'serial-{name}.log'
    with disk.open('wb') as f: f.truncate(256 << 20)
    serial.write_bytes(b'')
    storage = [] if name == 'ahci_only' else NVME
    args = [x.format(disk=disk) for x in storage + extra]
    nic = [] if any(x.startswith('user,') for x in args) else ['-nic', 'none']
    cmd = ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku,+rdseed', '-m', '2G', '-vga', 'none',
           '-device', 'VGA,xres=1280,yres=800', '-cdrom', str(iso), '-serial', f'file:{serial}',
           '-display', 'none', '-no-reboot'] + nic + args
    (out / f'cmd-{name}.txt').write_text(' '.join(cmd) + '\n')
    proc = subprocess.Popen(cmd, stdout=(out / f'qemu-{name}.txt').open('w'), stderr=subprocess.STDOUT)
    return name, need, serial, proc, time.time() + seconds


def judge(log, need):
    """(ok, reason) for one row's serial log."""
    for s in STOPPED:
        if s in log:
            return False, s.decode().strip('[]')
    if READY not in log:
        return False, 'no_desktop'
    missing = [n.decode() for n in need if n not in log]
    return (False, 'missing ' + ' '.join(missing)) if missing else (True, 'ok')


def main():
    a = argparse.ArgumentParser()
    a.add_argument('--iso', default='sexos-v1.0.0.iso')
    a.add_argument('--out', default='/tmp/sexos-device-matrix')
    a.add_argument('--rows', default=','.join(ROWS))
    a.add_argument('--seconds', type=int, default=240)
    a = a.parse_args()
    out = pathlib.Path(a.out); out.mkdir(parents=True, exist_ok=True)
    runs = [boot(pathlib.Path(a.iso).resolve(), n, out, a.seconds) for n in a.rows.split(',')]
    failed = []
    for name, need, serial, proc, deadline in runs:
        while True:
            log = serial.read_bytes()
            done = READY in log and all(n in log for n in need)
            if done or any(s in log for s in STOPPED) or proc.poll() is not None or time.time() > deadline: break
            time.sleep(1)
        time.sleep(2)  # a late restart right after the desktop still counts
        log = serial.read_bytes()
        proc.kill(); proc.wait()
        ok, why = judge(log, need)
        print(f'[device.matrix] row={name} ok={int(ok)} reason="{why}"')
        if not ok:
            failed.append(name)
            print(log[-3000:].decode(errors='replace'))
    if failed: sys.exit(f'[device.matrix] FAIL rows={",".join(failed)} logs={out}')
    print(f'[device.matrix] PASS rows={a.rows}')


if __name__ == '__main__':
    main()
