#!/usr/bin/env python3
"""Intel RST RAID 0 on two NVMe drives: SexOS reads the volume and writes neither drive.

Two member images are made with scripts/make_imsm_raid.py (4 KiB strips, so
the first 16 KiB of the volume alternate between the drives). The volume holds
a GPT made with sgdisk: partition 1 "EFI system partition" and partition 41
"SexRaidCheck", whose entry lies in a strip on the second drive. The drives are
attached in the opposite order to the RAID's member order, so the members are
found by serial number, not by position.

Passes when sexdrive serves the volume, lists both partitions, finds the backup
GPT header in the volume's last sector, and both drive images are byte for
byte unchanged after the run.
"""
import argparse, hashlib, os, re, shutil, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SECTOR = 512


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=Path, default=ROOT / 'sexos-v1.0.0.iso')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--timeout', type=int, default=240)
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    gen = [sys.executable, str(ROOT / 'scripts/make_imsm_raid.py')]
    d0, d1, vol = a.out / 'member0.img', a.out / 'member1.img', a.out / 'volume.img'
    info = subprocess.run(gen + [str(d0), str(d1), '--mib', '64', '--strip-kib', '4'], check=True, capture_output=True, text=True).stdout
    sectors = int(re.search(r'volume_sectors=(\d+)', info).group(1))
    with open(vol, 'wb') as f:
        f.truncate(sectors * SECTOR)
    subprocess.run(['sgdisk', '-o', '-n', '1:2048:+8M', '-t', '1:ef00', '-c', '1:EFI system partition',
                    '-n', '41:0:+16M', '-c', '41:SexRaidCheck', str(vol)], check=True, capture_output=True)
    print(subprocess.run(gen + [str(d0), str(d1), '--mib', '64', '--strip-kib', '4', '--volume', str(vol)],
                         check=True, capture_output=True, text=True).stdout.strip())
    before = (sha(d0), sha(d1))

    serial = a.out / 'serial.log'
    cmd = ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku,+rdseed', '-m', os.environ.get('SEXOS_GATE_MEMORY', '512M'),
           '-smp', os.environ.get('SEXOS_GATE_CORES', '1'), '-vga', 'none', '-device', 'VGA,xres=1280,yres=800',
           '-cdrom', str(a.iso),
           # Second member first: sexdrive must match drives to members by serial.
           '-drive', f'if=none,id=m1,file={d1},format=raw', '-device', 'nvme,serial=SEXRAID-1,drive=m1',
           '-drive', f'if=none,id=m0,file={d0},format=raw', '-device', 'nvme,serial=SEXRAID-0,drive=m0',
           '-serial', f'file:{serial}', '-display', 'none', '-no-reboot', '-no-shutdown', '-nic', 'none']
    proc = subprocess.Popen(cmd, stdout=(a.out / 'qemu.log').open('w'), stderr=subprocess.STDOUT)
    want = {
        'serve': r'\[sexdrive\.raid\.serve\] ok=1 name=Volume0 level=0 sectors=%d present=0b11 writes=0' % sectors,
        'esp': r'\[sexdrive\.raid\.partition\] index=0 first_lba=2048 sectors=16384 name=EFI system partition',
        'check': r'\[sexdrive\.raid\.partition\] index=40 first_lba=\d+ sectors=32768 name=SexRaidCheck',
        'read': r'\[sexdrive\.raid\.volume\.read\] ok=1 mbr_signature=1 gpt=1 partitions=2 backup_gpt_at_end=1',
        'second': r'\[sexdrive\.raid\.second\] same_set=1 disk=0 serial=SEXRAID-0',
        'skip': r'\[sexdrive\.storage100\.skip\] reason=raid_member writes=0',
    }
    found, text = {}, ''
    deadline = time.monotonic() + a.timeout
    try:
        while time.monotonic() < deadline and len(found) < len(want):
            text = serial.read_text(errors='replace') if serial.exists() else ''
            if re.search(r'KERNEL PANIC|PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at', text):
                break
            for k, pat in want.items():
                if k not in found and re.search(pat, text):
                    found[k] = True
            if proc.poll() is not None:
                break
            time.sleep(0.2)
        # Let the rest of the system start and try whatever it tries.
        time.sleep(20)
        text = serial.read_text(errors='replace') if serial.exists() else ''
    finally:
        proc.kill(); proc.wait()
    after = (sha(d0), sha(d1))
    for line in re.findall(r'\[(?:kernel\.pci\.nvme\.found|kernel\.cap\.nvme_bar\.grant|sexdrive\.raid[.\w]*|sexdrive\.nvme2[.\w]*)\][^\n]*', text):
        print(line)
    missing = [k for k in want if k not in found]
    failed = re.search(r'KERNEL PANIC|PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at', text)
    unchanged = before == after
    print(f'[raid.gate] markers_missing={missing} panic={bool(failed)} members_unchanged={unchanged}')
    if missing or failed or not unchanged:
        print('[raid.gate] FAIL'); return 1
    print('[raid.gate] PASS'); return 0


if __name__ == '__main__':
    sys.exit(main())
