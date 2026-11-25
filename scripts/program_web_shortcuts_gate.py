#!/usr/bin/env python3
"""Open Kaleidoscope's offline keyboard reference through its own shortcut."""
import argparse
import shutil
import time
from pathlib import Path

from PIL import Image, ImageChops

from quil_autosave_gate import boot


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
    guest = boot(args.iso, disk, args.out, 'web-shortcuts', args.resolution)
    q, log = guest['q'], guest['text']
    try:
        guest['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        guest['focus'](153)
        time.sleep(.5)
        start = len(log())
        for key in 'run kaleidoscope':
            q.key('spc' if key == ' ' else key)
            time.sleep(.05)
        q.key('ret')
        ready = guest['wait'](r'\[web.ready\] target=(\d+) task=(\d+)', start, 240)
        target, task = ready[1], ready[2]
        guest['wait'](r'\[web.loaded\] bytes_ok=1 ', start, 120)
        launched = guest['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', start, 60)
        surface = int(launched[1])
        guest['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w=\d+ h=\d+', start, 60)
        guest['focus'](surface)
        time.sleep(1)
        before = args.out / 'web-before-shortcuts.ppm'
        q.cmd('screendump', {'filename': str(before)})
        start = len(log())
        q.cmd('input-send-event', {'events': [
            {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': key}}}
            for down, keys in ((True, ('ctrl', 'shift', 'slash')),
                               (False, ('slash', 'shift', 'ctrl')))
            for key in keys
        ]})
        guest['wait'](r'\[web.shortcuts.opened\] url=about:shortcuts', start, 120)
        time.sleep(1)
        screenshot = args.out / 'web-shortcuts.ppm'
        q.cmd('screendump', {'filename': str(screenshot)})
        previous = Image.open(before).convert('RGB')
        current = Image.open(screenshot).convert('RGB')
        assert previous.size == current.size
        assert ImageChops.difference(previous, current).getbbox() is not None, 'shortcut did not change the displayed page'
        output = log()
        assert '[web.fatal]' not in output and 'KERNEL PANIC' not in output
        print({'ok': True, 'page': 'about:shortcuts', 'screenshot': str(screenshot)})
    finally:
        guest['stop']()


if __name__ == '__main__':
    main()
