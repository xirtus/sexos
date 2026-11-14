#!/usr/bin/env python3
"""Intel RST RAID 0 on two NVMe drives: SexOS reads the volume, Files opens its
partitions, and neither drive is written.

Two member images are made with scripts/make_imsm_raid.py (4 KiB strips, so
the first 16 KiB of the volume alternate between the drives). The volume holds
a GPT made with sgdisk: partition 1 "EFI system partition" and partition 41
"SexRaidCheck", a FAT16 volume labelled RAIDDATA holding HELLO.TXT, whose GPT
entry lies in a strip on the second drive. The drives are attached in the
opposite order to the RAID's member order, so the members are found by serial
number, not by position. The volume is big enough that Files looks for its own
filesystem on it, finds none, and is refused when it tries to make one.

Passes when sexdrive serves the volume, lists both partitions, finds the backup
GPT header in the volume's last sector; Files mounts RAIDDATA read only under
/media, Spindle lists it and prints HELLO.TXT, a write there is refused; and
both drive images are byte for byte unchanged after the run.
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
    size = ['--mib', '128', '--strip-kib', '4']
    info = subprocess.run(gen + [str(d0), str(d1)] + size, check=True, capture_output=True, text=True).stdout
    sectors = int(re.search(r'volume_sectors=(\d+)', info).group(1))
    with open(vol, 'wb') as f:
        f.truncate(sectors * SECTOR)
    subprocess.run(['sgdisk', '-o', '-n', '1:2048:+8M', '-t', '1:ef00', '-c', '1:EFI system partition',
                    '-n', '41:0:+16M', '-t', '41:0700', '-c', '41:SexRaidCheck', str(vol)], check=True, capture_output=True)
    first = int(re.search(r'First sector: (\d+)', subprocess.run(['sgdisk', '-i', '41', str(vol)], check=True, capture_output=True, text=True).stdout).group(1))
    fat, hello = a.out / 'raiddata.img', a.out / 'hello.txt'
    fat.unlink(missing_ok=True)
    with open(fat, 'wb') as f:
        f.truncate(16 << 20)
    hello.write_text('hello from the raid volume\n')
    subprocess.run(['mformat', '-i', str(fat), '-T', '32768', '-h', '64', '-s', '32', '-v', 'RAIDDATA', '::'], check=True, capture_output=True)
    subprocess.run(['mcopy', '-i', str(fat), str(hello), '::HELLO.TXT'], check=True, capture_output=True)
    with open(vol, 'r+b') as f:
        f.seek(first * SECTOR)
        f.write(fat.read_bytes())
    print(subprocess.run(gen + [str(d0), str(d1)] + size + ['--volume', str(vol)],
                         check=True, capture_output=True, text=True).stdout.strip())
    before = (sha(d0), sha(d1))

    # Second member first: sexdrive must match drives to members by serial.
    os.environ['SEXOS_GATE_DISK_SERIAL'] = 'SEXRAID-1'
    os.environ['SEXOS_GATE_DEVICES'] = f'-drive if=none,id=m0,file={d0},format=raw -device nvme,serial=SEXRAID-0,drive=m0'
    os.environ.setdefault('SEXOS_GATE_BOUND', '900')
    sys.path.insert(0, str(ROOT / 'scripts'))
    from quil_autosave_gate import boot
    want = {
        'serve': r'\[sexdrive\.raid\.serve\] ok=1 name=Volume0 level=0 sectors=%d present=0b11 writes=0' % sectors,
        'esp': r'\[sexdrive\.raid\.partition\] index=0 first_lba=2048 sectors=16384 name=EFI system partition',
        'check': r'\[sexdrive\.raid\.partition\] index=40 first_lba=%d sectors=32768 name=SexRaidCheck' % first,
        'read': r'\[sexdrive\.raid\.volume\.read\] ok=1 mbr_signature=1 gpt=1 partitions=2 backup_gpt_at_end=1',
        'second': r'\[sexdrive\.raid\.second\] same_set=1 disk=0 serial=SEXRAID-0',
        'skip': r'\[sexdrive\.storage100\.skip\] reason=raid_member writes=0',
    }
    found, text, error, g = {}, '', None, None
    try:
        g = boot(a.iso, d1, a.out, 'raid', '1280x800')
        t, q = g['text'], g['q']
        for k, pat in want.items():
            g['wait'](pat, 0, a.timeout); found[k] = True
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        keys = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}

        def line(s):
            g['focus'](153); time.sleep(.5)
            for k in s:
                code = keys.get(k, k.lower())
                if k.isupper():
                    q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': code}], 'hold-time': 80})
                else:
                    q.key(code)
                time.sleep(.06)
            q.key('ret')

        def expect(name, s, pattern, timeout=90):
            at = len(t()); line(s); m = g['wait'](pattern, at, timeout); found[name] = True; return m

        expect('media', 'ls /media', r'\[spindle\.fs\.line\] RAIDDATA')
        mounted = re.search(r'\[files\.media\.mounted\] name=RAIDDATA format=FAT16 sectors=32768 start=%d read_only=1' % first, t())
        if mounted: found['mounted'] = True
        if re.search(r'\[files\.fs\.(format\.failed|mount\.failed)\]', t()): found['no_sexos_fs'] = True
        expect('cat', 'cat /media/RAIDDATA/HELLO.TXT', r'\[spindle\.fs\.cat\] hello from the raid volume')
        expect('refused', 'write /media/RAIDDATA/nope.txt x', r'\[spindle\.fs\.line\] read-only disk')
        text = t()
    except Exception as e:
        error = e
        text = g['text']() if g else ''
        # A boot that never finished: show where it stopped.
        serial = a.out / 'raid-serial.log'
        if not g and serial.exists():
            print('\n'.join(serial.read_text(errors='replace').splitlines()[-80:]))
    finally:
        if g: g['stop']()
    after = (sha(d0), sha(d1))
    for line_ in re.findall(r'\[(?:kernel\.pci\.nvme\.found|kernel\.cap\.nvme_bar\.grant|sexdrive\.raid[.\w]*|sexdrive\.nvme2[.\w]*|files\.(?:fs|media)[.\w]*)\][^\n]*', text):
        print(line_)
    need = list(want) + ['media', 'mounted', 'no_sexos_fs', 'cat', 'refused']
    missing = [k for k in need if k not in found]
    unchanged = before == after
    print(f'[raid.gate] markers_missing={missing} error={error!r} members_unchanged={unchanged}')
    if missing or error or not unchanged:
        print('[raid.gate] FAIL'); return 1
    print('[raid.gate] PASS'); return 0


if __name__ == '__main__':
    sys.exit(main())
