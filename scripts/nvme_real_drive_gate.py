#!/usr/bin/env python3
"""NVMe drives that are not QEMU's defaults, as found in real laptops.

1. A drive that takes at most 64 KiB per command (MDTS = 4; QEMU's default is
   512 KiB, a Samsung 980 PRO's 2 MiB, some drives' 64 KiB). SexDrive reads the
   limit from Identify Controller and cuts every larger request into commands
   the drive accepts. QEMU fails any longer command, so Files formatting the
   disk, writing, and finding everything again after a reboot proves it.
2. A namespace formatted to 4 KiB sectors. Everything here works in 512-byte
   sectors and a 4 KiB-sector read would DMA past its buffer, so SexDrive must
   refuse the drive before any read or write, say why and how to fix it, and
   the system must still come up. The disk image must be untouched.

3. Another system's disk (a GPT with a FAT partition, standing in for the
   laptop's Windows SSD in AHCI/NVMe mode): sexdrive serves it for reading only,
   Files never mounts or formats it as its own, opens its partition read only
   under /media, a write there is refused, and the image is untouched.

Passes when all three hold, the boot log names the drive (model, MDTS), and Spindle's
`hw` (the hardware report for a PC with no serial port) shows the drive and the
device census, and `hw save` writes the same report to a file.
"""
import argparse, hashlib, os, re, shutil, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from filesystem_gate import session  # noqa: E402

GIB = 1 << 30


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def blank(path, source):
    shutil.copyfile(source, path)
    with path.open('r+b') as f:
        f.truncate(GIB)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    os.environ.setdefault('SEXOS_GATE_BOUND', '900')
    report = {}

    # 1. 64 KiB per command.
    disk = a.out / 'mdts.img'
    blank(disk, a.source_disk)
    os.environ['SEXOS_GATE_NVME_OPTS'] = ',mdts=4'
    g, t, q, run, cat, ls = session(a, disk, 'mdts1')
    try:
        g['wait'](r'\[sexdrive\.nvme\.drive\] vendor=1b36 model="[^"]+" firmware="[^"]*" mdts=4 max_transfer_kib=64 namespaces=\d+', 0, 60)
        g['wait'](r'\[sexdrive\.nvme\.namespace\] sectors=2097152 lba_shift=9 nsid=1', 0, 60)
        run('df', r'\[spindle\.fs\.line\] \d+ MiB free of \d+ MiB')
        assert '[files.fs.formatted]' in t(), 'no format on a new disk'
        run('mkdir big', r'\[spindle\.fs\.line\] ok')
        run('write big/a.txt sixty four kilobytes a command', r'\[spindle\.fs\.result\] kind=8 status=0')
        assert cat('big/a.txt') == ['sixty four kilobytes a command'], t()[-400:]
        # The hardware report a PC with no serial port shows: Spindle's `hw`.
        run('hw', r'\[spindle\.hw\] lines=\d+ shown=\d+ head_bytes=\d+')
        hw = t()
        assert re.search(r'\[spindle\.hw\.line\] \[sexdrive\.nvme\.drive\] vendor=1b36 ', hw), 'hw did not show the drive'
        assert re.search(r'\[spindle\.hw\.line\] \[hw\.device\] ', hw), 'hw did not show the device census'
        # `hw save`: the same report as a text file, as it would go to a USB stick.
        run('hw save hw.txt', r'\[spindle\.fs\.result\] kind=8 status=0')
        assert re.search(r'\[spindle\.hw\.save\] lines=[1-9]\d* bytes=[1-9]\d* head_bytes=[1-9]', t()), t()[-400:]
        start = len(t())
        saved = cat('hw.txt')
        assert saved and saved[0] == 'SexOS hardware report (Spindle hw save)', saved[:3]
        # cat reads 4 KiB at a time; give the later reads a moment.
        g['wait'](r'\[spindle\.fs\.cat\] \[sexdrive\.nvme\.drive\] vendor=1b36 ', start, 30)
        text = t()
        errors = re.findall(r'\[sexdrive\.(?:nvme\.(?:write|queued)\.err|block\.read\.(?:api|handoff)\.err|dma\.quarantine)\][^\n]*', text)
        assert not errors, errors
        report['mdts_split'] = re.findall(r'\[sexdrive\.nvme\.split\][^\n]*', text)
        assert 'KERNEL PANIC' not in text
    finally:
        g['stop']()
    g, t, q, run, cat, ls = session(a, disk, 'mdts2')
    try:
        assert cat('big/a.txt') == ['sixty four kilobytes a command'], t()[-400:]
        assert '[files.fs.mounted]' in t() and '[files.fs.formatted]' not in t(), 'mount after reboot failed'
        report['mdts_split'] += re.findall(r'\[sexdrive\.nvme\.split\][^\n]*', t())
    finally:
        g['stop']()
    print('[nvme_real_drive.mdts]', report['mdts_split'] or 'no request needed more than 64 KiB')

    # 2. 4 KiB sectors: refused before any I/O.
    disk = a.out / 'lba4k.img'
    blank(disk, a.source_disk)
    before = sha(disk)
    os.environ['SEXOS_GATE_NVME_OPTS'] = ',logical_block_size=4096,physical_block_size=4096'
    g, t, q, run, cat, ls = session(a, disk, 'lba4k')
    try:
        g['wait'](r'\[sexdrive\.nvme\.namespace\.refused\] nsid=1 reason=sector_size lba_bytes=4096 metadata_bytes=0 fix=nvme_format_lbaf_512 writes=0 reads=0', 0, 60)
        time.sleep(3)
        text = t()
        assert not re.search(r'\[sexdrive\.(?:storage100\.rw|nvme\.write\.submit|disk\.layout)\]', text), 'I/O on a refused drive'
        assert '[files.fs.formatted]' not in text, 'Files formatted a refused drive'
        assert 'KERNEL PANIC' not in text
        report['lba4k'] = 'refused'
    finally:
        g['stop']()
        os.environ.pop('SEXOS_GATE_NVME_OPTS', None)
    assert sha(disk) == before, 'the 4 KiB-sector disk was written'

    # 3. Another system's disk: read only, its partition under /media.
    disk = a.out / 'foreign.img'
    disk.unlink(missing_ok=True)
    with disk.open('wb') as f:
        f.truncate(GIB)
    subprocess.run(['sgdisk', '-o', '-n', '1:2048:+100M', '-t', '1:ef00', '-c', '1:EFI system partition',
                    '-n', '2:0:+16M', '-t', '2:0700', '-c', '2:Basic data partition', str(disk)], check=True, capture_output=True)
    first = int(re.search(r'First sector: (\d+)', subprocess.run(['sgdisk', '-i', '2', str(disk)], check=True, capture_output=True, text=True).stdout).group(1))
    fat, hello = a.out / 'windata.img', a.out / 'hello.txt'
    fat.unlink(missing_ok=True)
    with fat.open('wb') as f:
        f.truncate(16 << 20)
    hello.write_text('hello from the other system\n')
    subprocess.run(['mformat', '-i', str(fat), '-T', '32768', '-h', '64', '-s', '32', '-v', 'WINDATA', '::'], check=True, capture_output=True)
    subprocess.run(['mcopy', '-i', str(fat), str(hello), '::hello.txt'], check=True, capture_output=True)
    with disk.open('r+b') as f:
        f.seek(first * 512)
        f.write(fat.read_bytes())
    before = sha(disk)
    g, t, q, run, cat, ls = session(a, disk, 'foreign')
    try:
        g['wait'](r'\[sexdrive\.disk\.layout\] kind=foreign-gpt ', 0, 60)
        g['wait'](r'\[sexdrive\.disk\.foreign\] writes=refused reads=whole_disk', 0, 60)
        g['wait'](r'\[files\.fs\.foreign\] sectors=2097152 read_only=1', 0, 60)
        g['wait'](r'\[files\.media\.mounted\] name=WINDATA format=FAT16 sectors=32768 start=%d read_only=1' % first, 0, 60)
        assert cat('/media/WINDATA/hello.txt') == ['hello from the other system'], t()[-400:]
        run('write /media/WINDATA/nope.txt x', r'\[spindle\.fs\.line\] read-only disk')
        text = t()
        assert not re.search(r'\[files\.fs\.(formatted|mounted)\]', text), 'Files took the other system\'s disk'
        assert 'KERNEL PANIC' not in text
        report['foreign'] = 'read_only'
    finally:
        g['stop']()
    assert sha(disk) == before, 'the other system\'s disk was written'
    print('[nvme_real_drive.gate] PASS', report)
    return 0


if __name__ == '__main__':
    sys.exit(main())
