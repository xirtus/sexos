#!/usr/bin/env python3
"""Boot the ISO with several RAM sizes and require each to reach the desktop.

A 2 GiB machine once stopped at BOOT_REGION_NO_FRAMES: the program record pool
asked for one 1 GiB block, which no machine with under ~3 GiB of RAM has whole.
The Alienware x17 has 32 GiB, so the large sizes matter as much as the small.
Guest RAM is mapped without reserving it (reserve=off), so 32G runs on a
runner with less memory: the kernel touches only what it uses.
"""
import argparse, pathlib, subprocess, sys, time

READY, PANIC = b'[silkbar.ready]', b'KERNEL PANIC'

def boot(iso, size, out, seconds):
    disk, serial = out / f'disk-{size}.img', out / f'serial-{size}.log'
    with disk.open('wb') as f: f.truncate(256 << 20)
    serial.write_bytes(b'')
    cmd = ['qemu-system-x86_64', '-M', 'q35,memory-backend=ram0',
           '-object', f'memory-backend-ram,id=ram0,size={size},reserve=off',
           '-cpu', 'max,+pku,+rdseed', '-m', size, '-vga', 'none', '-device', 'VGA,xres=1280,yres=800',
           '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={disk},format=raw',
           '-device', 'nvme,serial=sexos01,drive=nvm', '-serial', f'file:{serial}',
           '-display', 'none', '-no-reboot', '-nic', 'none']
    return size, serial, subprocess.Popen(cmd, stdout=(out / f'qemu-{size}.txt').open('w'), stderr=subprocess.STDOUT), time.time() + seconds

def main():
    a = argparse.ArgumentParser()
    a.add_argument('--iso', default='sexos-v1.0.0.iso')
    a.add_argument('--out', default='/tmp/sexos-ram-sizes')
    a.add_argument('--sizes', default='1G,2G,4G,32G')
    a.add_argument('--seconds', type=int, default=180)
    a = a.parse_args()
    out = pathlib.Path(a.out); out.mkdir(parents=True, exist_ok=True)
    runs = [boot(pathlib.Path(a.iso).resolve(), s, out, a.seconds) for s in a.sizes.split(',')]
    failed = []
    for size, serial, proc, deadline in runs:
        while True:
            log = serial.read_bytes()
            if READY in log or PANIC in log or proc.poll() is not None or time.time() > deadline: break
            time.sleep(1)
        proc.kill(); proc.wait()
        records = next((l for l in log.splitlines() if l.startswith(b'[boot.records]')), b'(no records line)')
        ok = READY in log and PANIC not in log
        print(f'[ram.sizes] size={size} ok={int(ok)} {records.decode(errors="replace")}')
        if not ok:
            failed.append(size)
            tail = log[log.find(PANIC):] if PANIC in log else log[-2000:]
            print(tail.decode(errors='replace')[:2000])
    if failed: sys.exit(f'[ram.sizes] FAIL sizes={",".join(failed)} logs={out}')
    print(f'[ram.sizes] PASS sizes={a.sizes}')

if __name__ == '__main__':
    main()
