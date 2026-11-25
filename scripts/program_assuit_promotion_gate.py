#!/usr/bin/env python3
"""Cut power at Assuit's synced-staging / Files-promotion boundary.

The serial receipt precedes the rename request, but host observation cannot
guarantee the cut lands inside Files. Accept only the complete old or complete
new document after reboot; neither a partial ZIP nor a recovery overlay counts.
"""
import argparse
import shutil
import time
from pathlib import Path

from install_program import install
from program_assuit_recovery_gate import ROOT, session, open_assuit, type_text, chord


def type_command(q, command):
    for char in command:
        q.key({' ': 'spc', '/': 'slash', '.': 'dot'}.get(char, char))
        time.sleep(.05)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--iso', type=Path, required=True)
    parser.add_argument('--source-disk', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--resolution', default='1280x800')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    disk = args.out / 'nvme.img'
    shutil.copyfile(args.source_disk, disk)
    binary = (ROOT / 'iso_root/programs/assuit').read_bytes()
    assert binary[:4] == b'\x7fELF'
    install(disk, [('assuit', binary)])

    g = session(args.iso, disk, args.out, 'assuit-promotion-1', args.resolution)
    try:
        open_assuit(g)
        text, q = g['text'], g['q']
        start = len(text())
        q.key('end')
        type_text(q, ' baseline')
        g['wait'](r'\[assuit.text\] chars=\d+ first=Welcome to Assuit baseline\n', start, 120)
        start = len(text())
        chord(q, 'ctrl', 's')
        baseline = g['wait'](r'\[assuit.promote.begin\] from=welcome.docx.assuit-\d+.tmp to=welcome.docx bytes=(\d+) fnv=([0-9a-f]{16})\n', start, 120)
        g['wait'](r'\[assuit.promote.done\] to=welcome.docx\n', start, 120)
        g['wait'](r'\[assuit.saved\] name=welcome.docx bytes=\d+\n', start, 120)
        time.sleep(2)  # Let the baseline write reach the emulated disk.

        start = len(text())
        q.key('end')
        type_text(q, ' replacement')
        g['wait'](r'\[assuit.text\] chars=\d+ first=Welcome to Assuit baseline replacement\n', start, 120)
        start = len(text())
        chord(q, 'ctrl', 's')
        replacement = g['wait'](r'\[assuit.promote.begin\] from=welcome.docx.assuit-\d+.tmp to=welcome.docx bytes=(\d+) fnv=([0-9a-f]{16})\n', start, 120)
        assert baseline[2] != replacement[2], 'the replacement bytes did not change'
        # Do not wait for a successful save or give the app a Close event.
    finally:
        g['stop']()
    saw_done = '[assuit.promote.done] to=welcome.docx' in text()[start:]

    g = session(args.iso, disk, args.out, 'assuit-promotion-2', args.resolution)
    try:
        text, q = g['text'], g['q']
        g['focus'](153)
        start = len(text())
        type_command(q, 'sum /data/assuit/welcome.docx')
        q.key('ret')
        actual = g['wait'](r'\[spindle.fs.line\] (\d+) bytes, fnv ([0-9a-f]{16})\n', start, 120)
        expected = {(baseline[1], baseline[2]), (replacement[1], replacement[2])}
        assert (actual[1], actual[2]) in expected, ('partial saved file', actual[0], expected)
        expected_text = 'Welcome to Assuit baseline' if (actual[1], actual[2]) == (baseline[1], baseline[2]) else 'Welcome to Assuit baseline replacement'
        start = len(text())
        open_assuit(g)
        if '[assuit.recovered]' in text()[start:]:
            start = len(text())
            q.key('esc')
            g['wait'](r'\[assuit.discarded\] name=welcome.docx\n', start, 120)
        start = len(text())
        q.key('end')
        type_text(q, '!')
        found = g['wait'](r'\[assuit.text\] chars=\d+ first=(Welcome to Assuit baseline(?: replacement)?)!\n', start, 120)
        assert found[1] == expected_text, (found[0], expected_text)
        time.sleep(.5)
        shot = args.out / 'assuit-promotion-reopened.ppm'
        q.cmd('screendump', {'filename': str(shot)})
        assert shot.stat().st_size > 10000
        trace = text()
        assert 'KERNEL PANIC' not in trace and '[std.abort]' not in trace
        print({'ok': True, 'reopened': found[1], 'disk_bytes': int(actual[1]), 'disk_fnv': actual[2], 'promote_done_seen_before_cut': saw_done})
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
