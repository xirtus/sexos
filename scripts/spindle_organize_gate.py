#!/usr/bin/env python3
"""Native Inbox organization: preview, checked move, opt-in auto, undo, reboot.

Run against an ISO freshly built from the tree containing `organize` and
`MOVE_IF_ID`. This gate copies the source disk and never changes it.
"""
import argparse
import json
import shutil
import time
from pathlib import Path

from quil_autosave_gate import boot

KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    if any(a.out.iterdir()):
        raise RuntimeError('fresh evidence directory required')
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk.resolve(strict=True), disk)
    with disk.open('r+b') as f:
        f.truncate(1024**3)

    g = boot(a.iso.resolve(strict=True), disk, a.out, 'organize', a.resolution)
    text = g['text']
    q = g['q']
    rows = []

    def run(line, pattern, timeout=90):
        g['focus'](153)
        time.sleep(.3)
        start = len(text())
        for key in line:
            if key.isupper():
                q.cmd('send-key', {'keys': [
                    {'type': 'qcode', 'data': 'shift'},
                    {'type': 'qcode', 'data': key.lower()},
                ], 'hold-time': 80})
            else:
                q.key(KEYS.get(key, key))
            time.sleep(.03)
        q.key('ret')
        match = g['wait'](pattern, start, timeout)
        rows.append({'command': line, 'receipt': match[0]})
        return match

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        run('organize', r'\[spindle.organize.done\] moved=0 skipped=0 scanned=0 dry=0')
        run('write /Inbox/a.pdf document', r'\[spindle.fs.result\] kind=8 status=0')
        run('write /Inbox/b.png image', r'\[spindle.fs.result\] kind=8 status=0')
        run('write /Inbox/c.txt other', r'\[spindle.fs.result\] kind=8 status=0')
        run('touch /Documents/a.pdf', r'\[spindle.fs.line\] ok')

        run('organize --dry-run', r'\[spindle.organize.done\] moved=0 skipped=1 scanned=3 dry=1')
        assert '[spindle.organize.preview] rule=1 file=a.pdf' in text()
        assert '[spindle.organize.preview] rule=2 file=b.png' in text()
        run('ls /Inbox', r'\[spindle.fs.line\] b.png  6')

        run('organize', r'\[spindle.organize.done\] moved=1 skipped=2 scanned=3 dry=0')
        assert '[spindle.organize.moved] id_checked=1 source=/Inbox/b.png destination=/Pictures/b.png' in text()
        run('ls /Pictures', r'\[spindle.fs.line\] b.png  6')
        run('ls /Inbox', r'\[spindle.fs.line\] a.pdf  9')
        run('organize', r'\[spindle.organize.done\] moved=0 skipped=2 scanned=2 dry=0')
        run('write /Inbox/off.pdf disabled', r'\[spindle.fs.result\] kind=8 status=0')
        time.sleep(3)
        assert '[spindle.organize.moved] id_checked=1 source=/Inbox/off.pdf' not in text()
        run('ls /Inbox', r'\[spindle.fs.line\] off.pdf  9')

        run('organize auto on', r'\[spindle.organize.auto\] enabled=1')
        g['wait'](r'\[spindle.organize.moved\] id_checked=1 source=/Inbox/off.pdf destination=/Documents/off.pdf', 0, 90)
        run('write /Inbox/next.png automatic', r'\[spindle.fs.result\] kind=8 status=0')
        g['wait'](r'\[spindle.organize.moved\] id_checked=1 source=/Inbox/next.png destination=/Pictures/next.png', 0, 90)
        run('organize undo', r'\[spindle.organize.undo\] remaining=')
        run('ls /Inbox', r'\[spindle.fs.line\] next.png  10')
        time.sleep(3)
        assert text().count('source=/Inbox/next.png destination=/Pictures/next.png') == 1

        g['stop']()
        g = boot(a.iso.resolve(strict=True), disk, a.out, 'organize-reboot', a.resolution)
        text, q = g['text'], g['q']
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        run('ls /Inbox', r'\[spindle.fs.line\] next.png  10')
        run('write /Inbox/reboot.pdf held', r'\[spindle.fs.result\] kind=8 status=0')
        time.sleep(3)
        assert 'source=/Inbox/reboot.pdf destination=/Documents/reboot.pdf' not in text()
        run('organize auto on', r'\[spindle.organize.auto\] enabled=1')
        g['wait'](r'\[spindle.organize.moved\] id_checked=1 source=/Inbox/reboot.pdf destination=/Documents/reboot.pdf', 0, 90)
        g['stop']()
        g = boot(a.iso.resolve(strict=True), disk, a.out, 'organize-reboot-enabled', a.resolution)
        text, q = g['text'], g['q']
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        run('write /Inbox/persist.pdf yes', r'\[spindle.fs.result\] kind=8 status=0')
        g['wait'](r'\[spindle.organize.moved\] id_checked=1 source=/Inbox/persist.pdf destination=/Documents/persist.pdf', 0, 90)
        assert 'KERNEL PANIC' not in text() and 'panicked' not in text()
        (a.out / 'result.json').write_text(json.dumps({'ok': True, 'cases': rows}, indent=2) + '\n')
        print(json.dumps({'ok': True, 'cases': len(rows)}))
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
