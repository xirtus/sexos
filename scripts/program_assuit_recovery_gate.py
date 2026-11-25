#!/usr/bin/env python3
"""Assuit restores an idle recovery copy after abrupt power loss.

Boot 1 edits the welcome document without saving, waits for Assuit's recovery
receipt, and kills QEMU without closing the window. Boot 2 must restore that
edit, save it to the document, and reopen the saved result.
"""
import argparse
import re
import shutil
import time
from pathlib import Path

from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
CATALOG = (ROOT / 'crates/sex-pdx/src/app_catalog.rs').read_text()
SURFACE = {int(target): int(surface) for target, surface in re.findall(
    r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)', CATALOG)}


def session(iso, disk, out, tag, resolution):
    g = boot(iso, disk, out, tag, resolution)
    g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
    return g


def type_text(q, value):
    for char in value:
        q.key('spc' if char == ' ' else char)
        time.sleep(.05)


def open_assuit(g):
    text, q = g['text'], g['q']
    g['focus'](153)
    start = len(text())
    type_text(q, 'run assuit')
    q.key('ret')
    opened = g['wait'](r'\[assuit.open\] target=(\d+) task=(\d+) name=welcome.docx pages=1\n', start, 900)
    g['focus'](SURFACE[int(opened[1])])
    time.sleep(.8)
    return opened


def chord(q, *keys):
    q.cmd('input-send-event', {'events': [
        {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': key}}}
        for down, sequence in ((True, keys), (False, keys[::-1])) for key in sequence
    ]})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--iso', type=Path, required=True)
    parser.add_argument('--source-disk', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--resolution', default='1280x800')
    args = parser.parse_args()
    binary = (ROOT / 'iso_root/programs/assuit').read_bytes()
    assert binary[:4] == b'\x7fELF'
    args.out.mkdir(parents=True, exist_ok=True)
    disk = args.out / 'nvme.img'
    shutil.copyfile(args.source_disk, disk)
    install(disk, [('assuit', binary)])

    g = session(args.iso, disk, args.out, 'assuit-recovery-1', args.resolution)
    try:
        open_assuit(g)
        text, q = g['text'], g['q']
        start = len(text())
        q.key('end')
        type_text(q, ' recovery')
        g['wait'](r'\[assuit.text\] chars=\d+ first=Welcome to Assuit recovery\n', start, 120)
        kept = g['wait'](r'\[assuit.kept\] name=welcome.docx.recovered.docx\n', start, 90)
        assert '[assuit.saved]' not in text()[start:start + kept.end()], 'document was saved before power cut'
        q.cmd('screendump', {'filename': str(args.out / 'assuit-unsaved-before-cut.ppm')})
        assert (args.out / 'assuit-unsaved-before-cut.ppm').stat().st_size > 10000
        time.sleep(2)
    finally:
        g['stop']()  # Power cut: no Close event or final recovery write.

    g = session(args.iso, disk, args.out, 'assuit-recovery-2', args.resolution)
    try:
        text, q = g['text'], g['q']
        start = len(text())
        opened = open_assuit(g)
        g['wait'](r'\[assuit.recovered\]\n', start, 60)
        q.cmd('screendump', {'filename': str(args.out / 'assuit-restored.ppm')})
        assert (args.out / 'assuit-restored.ppm').stat().st_size > 10000
        start = len(text())
        q.key('end')
        type_text(q, '!')
        g['wait'](r'\[assuit.text\] chars=\d+ first=Welcome to Assuit recovery!\n', start, 120)
        start = len(text())
        chord(q, 'ctrl', 's')
        g['wait'](r'\[assuit.saved\] name=welcome.docx bytes=\d+\n', start, 120)
        q.key('f11')
        g['wait'](rf'\[app.private.retired\] target={opened[1]} task={opened[2]} ', start, 180)
        start = len(text())
        open_assuit(g)
        assert '[assuit.recovered]' not in text()[start:], 'saved document still opened from recovery copy'
        start = len(text())
        q.key('end')
        type_text(q, '?')
        g['wait'](r'\[assuit.text\] chars=\d+ first=Welcome to Assuit recovery!?\n', start, 120)
        assert 'KERNEL PANIC' not in text() and '[std.abort]' not in text()
        print({'ok': True, 'power_cut_recovered': True, 'saved_reopened': True})
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
