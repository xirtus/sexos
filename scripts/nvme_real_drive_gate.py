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

Passes when both hold and the boot log names the drive (model, MDTS).
"""
import argparse, hashlib, os, re, shutil, sys, time
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
    print('[nvme_real_drive.gate] PASS', report)
    return 0


if __name__ == '__main__':
    sys.exit(main())
