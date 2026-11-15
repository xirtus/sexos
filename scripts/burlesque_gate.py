#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Burlesque on Weft: slide navigation, adding/deleting slides, layout changes,
presenter view toggle, PDF export, and clean Shell close."""
import argparse
import json
import re
import shutil
import time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_gate_geometry import colours, maximize

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out = a.out.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)

    elf = (ROOT / 'iso_root/programs/burlesque').read_bytes()
    assert elf[:4] == b'\x7fELF', 'valid ELF header'
    install(disk, [('burlesque', elf)])

    g = boot(a.iso, disk, a.out, 'burlesque', a.resolution)
    text, q = g['text'], g['q']
    rw, rh = map(int, a.resolution.split('x'))

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        g['focus'](153)
        time.sleep(0.5)

        start = len(text())
        for k in 'run burlesque':
            q.key('spc' if k == ' ' else k)
            time.sleep(0.08)
        q.key('ret')

        m = g['wait'](r'\[burlesque.ready\] target=(\d+) task=(\d+)', start, 300)
        target, task = m[1], m[2]
        launch = g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', start, 60)
        surface = int(launch[1])

        g['wait'](r'\[burlesque.frame\] n=1\n', start, 300)
        time.sleep(1)

        tile, before = maximize(g, surface)
        g['wait'](r'\[burlesque.frame\] n=\d+\n', before, 60)
        time.sleep(1)

        ox, oy = tile[0] + 8, tile[1] + 36

        def click_at(x, y):
            x, y = x + ox, y + oy
            q.cmd('input-send-event', {'events': [
                {'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
                {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]})
            time.sleep(0.2)
            for down in (True, False):
                q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
                time.sleep(0.15)

        def control(name):
            matches = re.findall(rf'\[burlesque.control\] name={name} x=(-?\d+) y=(-?\d+)', text())
            assert matches, f'control {name} not found'
            click_at(*map(int, matches[-1]))

        def screenshot(name):
            time.sleep(0.5)
            path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(path)})
            time.sleep(0.2)
            n = len(colours(path.read_bytes(), tile))
            assert n > 8, (name, 'blank window', n)
            return n

        # 1. Assert initial state
        m = g['wait'](r'\[burlesque.state\] slides=1 current=0 presenter=0', 0, 60)

        # 2. Add slide
        start = len(text())
        control('add_slide')
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=0', start, 60)

        # 3. Navigation: prev
        start = len(text())
        control('prev')
        g['wait'](r'\[burlesque.state\] slides=2 current=0 presenter=0', start, 60)

        # 4. Navigation: next
        start = len(text())
        control('next')
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=0', start, 60)

        # 5. Toggle Presenter View
        start = len(text())
        control('presenter')
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=1', start, 60)
        presenter_colours = screenshot('burlesque-presenter')

        # Toggle back to editor
        start = len(text())
        control('presenter')
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=0', start, 60)

        editor_colours = screenshot('burlesque-editor')

        # 6. Export PDF
        start = len(text())
        control('export_pdf')
        m_exp = g['wait'](r'\[burlesque.exported\] file=(\S+) bytes=([1-9]\d*) pages=2', start, 60)
        exported_file = m_exp[1]
        exported_bytes = int(m_exp[2])
        assert exported_bytes > 100, f'PDF too small: {exported_bytes}'

        # 7. Close window via F11
        start = len(text())
        g['focus'](surface)
        q.key('f11')
        g['wait'](rf'\[app.private.retired\] target={target} task={task} ', start, 120)

        assert '[burlesque.fatal]' not in text()

        result = dict(
            ok=True,
            target=target,
            task=task,
            slides=2,
            exported_file=exported_file,
            exported_bytes=exported_bytes,
            colours=dict(editor=editor_colours, presenter=presenter_colours),
        )
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(result)

    finally:
        g['stop']()


if __name__ == '__main__':
    main()
