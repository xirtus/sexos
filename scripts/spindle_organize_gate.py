#!/usr/bin/env python3
"""The Organizer in Files (plan phase 2): preview, organize now, the person's rules,
automatic placement of new arrivals from post-commit events, undo (which pins), pins,
explicit placement, and the same behaviour after a restart.

Inbox, Downloads and Desktop are the organized folders; documents, pictures, music and
videos go to /Documents, /Pictures, /Music and /Videos; anything else stays. Run
against an ISO built from this tree. The gate copies the source disk and never
changes it.
"""
import argparse
import json
import re
import shutil
import time
from pathlib import Path

from quil_autosave_gate import boot

KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}
SHIFTED = {'*': '8', '_': 'minus'}


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    shutil.rmtree(a.out, ignore_errors=True)
    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk.resolve(strict=True), disk)
    with disk.open('r+b') as f:
        f.truncate(1024**3)
    rows = []
    state = {}

    def start(name):
        g = boot(a.iso.resolve(strict=True), disk, a.out, name, a.resolution)
        state['g'] = g
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        time.sleep(2)
        return g

    def text():
        return state['g']['text']()

    def run(line, pattern, timeout=90):
        g = state['g']
        q = g['q']
        g['focus'](153)
        time.sleep(.3)
        begin = len(text())
        for key in line:
            if key.isupper() or key in SHIFTED:
                q.cmd('send-key', {'keys': [
                    {'type': 'qcode', 'data': 'shift'},
                    {'type': 'qcode', 'data': SHIFTED.get(key, key.lower())},
                ], 'hold-time': 80})
            else:
                q.key(KEYS.get(key, key))
            time.sleep(.04)
        q.key('ret')
        match = g['wait'](pattern, begin, timeout)
        rows.append({'command': line, 'receipt': match[0]})
        return begin

    def lines_since(begin):
        return re.findall(r'\[spindle\.organize\.line\] ([^\n]*)', text()[begin:])

    def wait(pattern, begin=0, timeout=60):
        m = state['g']['wait'](pattern, begin, timeout)
        rows.append({'wait': pattern, 'receipt': m[0]})
        return m

    start('organize')
    try:
        for folder in ('/Inbox', '/Downloads', '/Desktop', '/Documents'):
            run(f'mkdir {folder}', r'\[spindle.fs.result\] kind=3 status=0')
        run('write /Inbox/a.pdf document', r'\[spindle.fs.result\] kind=8 status=0')
        run('write /Inbox/b.png image', r'\[spindle.fs.result\] kind=8 status=0')
        run('write /Inbox/c.zip other', r'\[spindle.fs.result\] kind=8 status=0')
        run('write /Downloads/d.mp3 song', r'\[spindle.fs.result\] kind=8 status=0')
        run('touch /Documents/a.pdf', r'\[spindle.fs.line\] ok')
        # Arrivals are logged once committed, whether or not anything moves them.
        wait(r'\[files.organize.event\] seq=\d+ op=new id=\d+ path=/Downloads/d.mp3')

        # Preview: what would move, and why the rest stays. Nothing moves.
        s = run('organize --dry-run', r'\[spindle.organize.done\] kind=48 lines=4')
        got = lines_since(s)
        for want in ('would move /Inbox/a.pdf -> /Documents/a.pdf (rule 1)',
                     'would move /Inbox/b.png -> /Pictures/b.png (rule 2)',
                     'stays /Inbox/c.zip: no rule',
                     'would move /Downloads/d.mp3 -> /Music/d.mp3 (rule 3)'):
            assert want in got, (want, got)
        run('ls /Inbox', r'\[spindle.fs.line\] b.png  6')
        s = run('organize log', r'\[spindle.organize.done\] kind=51 lines=\d+')
        assert any(l.endswith('new /Inbox/b.png: automatic organization is off') for l in lines_since(s)), lines_since(s)

        # Organize now: an occupied name stays rather than being overwritten.
        s = run('organize', r'\[spindle.organize.done\] kind=48 lines=4')
        got = lines_since(s)
        for want in ('moved /Inbox/b.png -> /Pictures/b.png (rule 2)',
                     'moved /Downloads/d.mp3 -> /Music/d.mp3 (rule 3)',
                     'stays /Inbox/a.pdf: already exists',
                     'stays /Inbox/c.zip: no rule'):
            assert want in got, (want, got)
        run('ls /Pictures', r'\[spindle.fs.line\] b.png  6')
        run('ls /Music', r'\[spindle.fs.line\] d.mp3  5')

        # The person's rules outrank the built-in ones; bad rules are refused.
        run('organize rule *.zip /Archives', r'\[spindle.organize.rule\] ok=1')
        run('organize rule * /Anything', r'\[spindle.organize.failed\] kind=50 status=7')
        run('organize rule *.pdf /data/x', r'\[spindle.organize.failed\] kind=50 status=7')
        s = run('organize rules', r'\[spindle.organize.done\] kind=50 lines=1')
        assert lines_since(s) == ['*.zip /Archives'], lines_since(s)

        # Automatic: switching it on organizes what is there, then each new arrival.
        run('organize auto on', r'\[spindle.organize.auto\] enabled=1')
        wait(r'\[files.organize.moved\] rule=100 from=/Inbox/c.zip to=/Archives/c.zip')
        s = run('write /Desktop/next.png automatic', r'\[spindle.fs.result\] kind=8 status=0')
        wait(r'\[files.organize.event\] seq=\d+ op=new id=\d+ path=/Desktop/next.png', s)
        wait(r'\[files.organize.moved\] rule=2 from=/Desktop/next.png to=/Pictures/next.png', s)
        s = run('write /Inbox/notes.txt typed', r'\[spindle.fs.result\] kind=8 status=0')
        wait(r'\[files.organize.moved\] rule=1 from=/Inbox/notes.txt to=/Documents/notes.txt', s)
        # A hidden, still-arriving name is never placed.
        s = run('write /Downloads/.big.iso.part partial', r'\[spindle.fs.result\] kind=8 status=0')
        time.sleep(4)
        assert '/Downloads/.big.iso.part' not in text()[s:], text()[s:][-800:]

        # Undo puts it back and pins it there, so automatic mode leaves it alone.
        run('organize undo', r'\[spindle.organize.undo\] ok=1')
        wait(r'\[files.organize.undone\] to=/Inbox/notes.txt from=/Documents/notes.txt pinned=1 left=\d+')
        run('ls /Inbox', r'\[spindle.fs.line\] notes.txt  6')
        s = run('write /Inbox/notes.txt typed again', r'\[spindle.fs.result\] kind=8 status=0')
        time.sleep(4)
        assert 'from=/Inbox/notes.txt' not in text()[s:], text()[s:][-800:]
        s = run('organize log', r'\[spindle.organize.done\] kind=51 lines=\d+')
        assert any(l.endswith('/Inbox/notes.txt: stays: pinned') for l in lines_since(s)), lines_since(s)

        # Pins and an explicit placement ("put this there"), which is undoable too.
        run('organize pin /Inbox/a.pdf', r'\[spindle.organize.pin\] on=1')
        s = run('organize --dry-run', r'\[spindle.organize.done\] kind=48 lines=\d+')
        assert 'stays /Inbox/a.pdf: pinned' in lines_since(s), lines_since(s)
        run('organize send /Inbox/a.pdf /Documents/Old', r'\[spindle.organize.sent\] ok=1')
        run('ls /Documents/Old', r'\[spindle.fs.line\] a.pdf  9')
        run('organize send /Documents/Old/a.pdf /Pictures', r'\[spindle.organize.failed\] kind=46 status=11')
        run('organize unpin /Inbox/notes.txt', r'\[spindle.organize.pin\] on=0')
        run('organize status', r'\[spindle.organize.status\] enabled=1 events=\d+ pins=1 rules=1')
        assert 'KERNEL PANIC' not in text() and 'panicked' not in text()
        state['g']['stop']()

        # After a restart: still on, rules and pins kept, arrivals still placed.
        start('organize-restart')
        run('organize status', r'\[spindle.organize.status\] enabled=1 events=0 pins=1 rules=1')
        s = run('write /Inbox/persist.pdf kept', r'\[spindle.fs.result\] kind=8 status=0')
        wait(r'\[files.organize.moved\] rule=1 from=/Inbox/persist.pdf to=/Documents/persist.pdf', s)
        s = run('write /Desktop/held.zip kept', r'\[spindle.fs.result\] kind=8 status=0')
        wait(r'\[files.organize.moved\] rule=100 from=/Desktop/held.zip to=/Archives/held.zip', s)
        run('organize auto off', r'\[spindle.organize.auto\] enabled=0')
        s = run('write /Inbox/off.pdf stays', r'\[spindle.fs.result\] kind=8 status=0')
        time.sleep(4)
        assert 'from=/Inbox/off.pdf' not in text()[s:], text()[s:][-800:]
        assert 'KERNEL PANIC' not in text() and 'panicked' not in text()
        (a.out / 'result.json').write_text(json.dumps({'ok': True, 'cases': rows}, indent=2) + '\n')
        print(json.dumps({'ok': True, 'cases': len(rows)}))
    finally:
        state['g']['stop']()


if __name__ == '__main__':
    main()
