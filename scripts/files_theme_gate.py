#!/usr/bin/env python3
"""Loom grid repaints when Tatami changes the shared appearance setting."""
import argparse
import shutil
import time
from pathlib import Path

from install_program import install
from quil_autosave_gate import boot
from settings_persist_gate import first_start

ROOT = Path(__file__).resolve().parents[1]
LIGHT = bytes((0xF6, 0xF5, 0xF7))
DARK = bytes((0x1C, 0x1A, 0x20))


def pixels(path, colour):
    data = path.read_bytes()
    assert data.startswith(b'P6\n'), path
    return data.count(colour)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--iso', required=True, type=Path)
    parser.add_argument('--source-disk', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--resolution', default='1280x800')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    disk = args.out / 'nvme.img'
    shutil.copyfile(args.source_disk, disk)
    install(disk, [('loom', (ROOT / 'iso_root/programs/loom').read_bytes()),
                   ('tatami', (ROOT / 'iso_root/programs/tatami').read_bytes())])
    guest = boot(args.iso, disk, args.out, 'files-theme', args.resolution)
    q, log = guest['q'], guest['text']

    def command(line):
        guest['focus'](153)
        time.sleep(.5)
        start = len(log())
        for key in line:
            q.key('spc' if key == ' ' else key)
            time.sleep(.05)
        q.key('ret')
        return start

    def shot(name):
        time.sleep(1)
        path = args.out / f'{name}.ppm'
        q.cmd('screendump', {'filename': str(path)})
        return path

    try:
        guest['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        start = command('files')
        launched = guest['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)', start, 120)
        guest['wait'](r'\[files.app.ready\] target=\d+ task=\d+ root=/ portal=1', start, 120)
        guest['wait'](r'\[files.app.theme\] mode=Light', start, 60)
        surface = int(launched[3])
        guest['focus'](surface)
        time.sleep(.5)
        q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'ctrl'}, {'type': 'qcode', 'data': 'g'}], 'hold-time': 80})
        guest['wait'](r'\[files.app.grid\] on=1 dir=/', start, 60)
        light = shot('loom-light')
        assert pixels(light, LIGHT) > 5000, 'light grid background missing'

        start = command('run tatami')
        first_start(guest, q, log, start)
        guest['wait'](r'\[settings.open\] generation=\d+', start, 120)
        for key in 'theme':
            q.key(key)
            time.sleep(.1)
        start = len(log())
        q.key('right')
        guest['wait'](r'\[settings.set\] key=24 value=1 status=0', start, 60)
        guest['wait'](r'\[files.app.theme\] mode=Dark', start, 60)
        guest['focus'](surface)
        dark = shot('loom-dark')
        assert pixels(dark, DARK) > 5000, 'dark grid background missing'
        text = log()
        assert 'KERNEL PANIC' not in text and '[files.app.fatal]' not in text
        print({'ok': True, 'light_pixels': pixels(light, LIGHT), 'dark_pixels': pixels(dark, DARK)})
    finally:
        guest['stop']()


if __name__ == '__main__':
    main()
