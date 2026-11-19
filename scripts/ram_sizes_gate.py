#!/usr/bin/env python3
"""Boot the ISO with several RAM sizes and require each to reach the desktop.

A 2 GiB machine once stopped at BOOT_REGION_NO_FRAMES: the program record pool
asked for one 1 GiB block, which no machine with under ~3 GiB of RAM has whole.
The Alienware x17 has 32 GiB, so the large sizes matter as much as the small.
Guest RAM is mapped without reserving it (reserve=off), so 32G runs on a
runner with less memory: the kernel touches only what it uses.

The x17's own setup comes next: 32 GiB with its FHD panel (1920x1080). Limine
picks the native mode from the display's EDID, as on the laptop; the desktop must
lay itself out at that width. The UHD panel (`--panels 32G:3840x2160`) is opt-in:
under QEMU's VGA and OVMF it did not come up at 3840 wide (2026-11-19 CI), which
may be the firmware's mode list rather than SexOS; the x17 itself settles it.
"""
import argparse, pathlib, re, subprocess, sys, time

READY, PANIC = b'[silkbar.ready]', b'KERNEL PANIC'

def boot(iso, size, out, seconds, mode='1280x800'):
    tag = size if mode == '1280x800' else f'{size}-{mode}'
    disk, serial = out / f'disk-{tag}.img', out / f'serial-{tag}.log'
    w, h = mode.split('x')
    # A UHD framebuffer is 33 MB: more than the VGA device's default 16 MB.
    vga = f'VGA,xres={w},yres={h}' + (',vgamem_mb=64' if int(w) * int(h) * 4 > 16 << 20 else '')
    with disk.open('wb') as f: f.truncate(256 << 20)
    serial.write_bytes(b'')
    cmd = ['qemu-system-x86_64', '-M', 'q35,memory-backend=ram0',
           '-object', f'memory-backend-ram,id=ram0,size={size},reserve=off',
           '-cpu', 'max,+pku,+rdseed', '-m', size, '-vga', 'none', '-device', vga,
           '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={disk},format=raw',
           '-device', 'nvme,serial=sexos01,drive=nvm', '-serial', f'file:{serial}',
           '-display', 'none', '-no-reboot', '-nic', 'none']
    return tag, serial, subprocess.Popen(cmd, stdout=(out / f'qemu-{tag}.txt').open('w'), stderr=subprocess.STDOUT), time.time() + seconds

def main():
    a = argparse.ArgumentParser()
    a.add_argument('--iso', default='sexos-v1.0.0.iso')
    a.add_argument('--out', default='/tmp/sexos-ram-sizes')
    a.add_argument('--sizes', default='1G,2G,4G,32G')
    a.add_argument('--seconds', type=int, default=180)
    a.add_argument('--panels', default='32G:1920x1080',  help='SIZE:MODE runs for the x17 panels; empty for none')
    a = a.parse_args()
    out = pathlib.Path(a.out); out.mkdir(parents=True, exist_ok=True)
    iso = pathlib.Path(a.iso).resolve()
    runs = [boot(iso, s, out, a.seconds) for s in a.sizes.split(',')]
    runs += [boot(iso, p.split(':')[0], out, a.seconds, p.split(':')[1])
             for p in a.panels.split(',') if p]
    failed = []
    for size, serial, proc, deadline in runs:
        while True:
            log = serial.read_bytes()
            if READY in log or PANIC in log or proc.poll() is not None or time.time() > deadline: break
            time.sleep(1)
        proc.kill(); proc.wait()
        records = next((l for l in log.splitlines() if l.startswith(b'[boot.records]')), b'(no records line)')
        ok = READY in log and PANIC not in log
        if ok and '-' in size:
            # The desktop laid itself out at the panel's width.
            want = size.split('-')[1].split('x')[0].encode()
            ok = re.search(rb'\[display\.chrome\.metrics\] [^\n]*width=' + want + rb'\b', log) is not None
            if not ok:
                got = re.findall(rb'\[display\.chrome\.metrics\] [^\n]*', log)[-1:] or [b'(no metrics line)']
                print(f'[ram.sizes] {size}: no [display.chrome.metrics] at width {want.decode()}; got {got[0].decode(errors="replace")}')
        print(f'[ram.sizes] size={size} ok={int(ok)} {records.decode(errors="replace")}')
        if not ok:
            failed.append(size)
            tail = log[log.find(PANIC):] if PANIC in log else log[-2000:]
            print(tail.decode(errors='replace')[:2000])
    if failed: sys.exit(f'[ram.sizes] FAIL sizes={",".join(failed)} logs={out}')
    print(f'[ram.sizes] PASS sizes={a.sizes} panels={a.panels}')

if __name__ == '__main__':
    main()
