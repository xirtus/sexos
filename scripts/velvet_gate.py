#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Velvet on Weft: real pointer/keyboard play across all three games, rendered
frames, restart, flags, merging, matching and Shell Close. No test-only game API."""
import argparse
import json
import re
import shutil
import time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_egui_gate import colours
from program_gate_geometry import maximize, current_tile
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
    elf = (ROOT / 'iso_root/programs/velvet').read_bytes()
    assert elf[:4] == b'\x7fELF'
    install(disk, [('velvet', elf)])
    g = boot(a.iso, disk, a.out, 'velvet', a.resolution)
    text, q = g['text'], g['q']
    rw, rh = map(int, a.resolution.split('x'))
    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        g['focus'](153)
        time.sleep(.5)
        start = len(text())
        for k in 'run velvet':
            q.key('spc' if k == ' ' else k)
            time.sleep(.08)
        q.key('ret')
        m = g['wait'](r'\[velvet.ready\] target=(\d+) task=(\d+)', start, 300)
        target, task = m[1], m[2]
        launch = g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', start, 60)
        surface = int(launch[1])
        g['wait'](r'\[velvet.frame\] n=1\n', start, 300)
        tile, resized = maximize(g, surface)
        layout = g['wait'](rf'\[velvet.layout\] w=({tile[2]-16}) h=({tile[3]-42})', resized, 120)
        width, height = map(int, layout.groups())
        g['focus'](surface)

        def click_at(x, y):
            assert 0 <= x < width and 0 <= y < height, (x, y, width, height)
            actual = current_tile(text(), surface)
            side = (actual[2] - width) // 2
            top = actual[3] - height - side
            x, y = x + actual[0] + side, y + actual[1] + top
            assert 0 <= x < rw and 0 <= y < rh, (x, y, rw, rh)
            q.cmd('input-send-event', {'events': [
                {'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
                {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]})
            time.sleep(.2)
            for down in (True, False):
                q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
                time.sleep(.15)

        def cell(i):
            matches = re.findall(rf'\[velvet.cell\] n={i} x=(-?\d+) y=(-?\d+)', text()[resized:])
            assert matches, f'no cell {i}'
            click_at(*map(int, matches[-1]))

        def control(name):
            matches = re.findall(rf'\[velvet.control\] name={name} x=(-?\d+) y=(-?\d+)', text()[resized:])
            assert matches, name
            click_at(*map(int, matches[-1]))

        def chord(key):
            q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': k}}}
                for down, keys in ((True, ('ctrl', key)), (False, (key, 'ctrl'))) for k in keys]})

        def screenshot(name):
            time.sleep(.5)
            path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(path)})
            time.sleep(.2)
            n = len(colours(path.read_bytes(), current_tile(text(), surface)))
            assert n > 8, (name, 'blank window', n)
            return n

        start = len(text())
        cell(0)
        m = g['wait'](r'\[velvet.state\] game=Snags state=(Playing|Won) opened=([1-9]\d*) flags=0 ', start, 60)
        opened = int(m[2])
        snags_colours = screenshot('snags')
        start = len(text())
        chord('n')
        g['wait'](r'\[velvet.state\] game=Snags state=Playing opened=0 flags=0 ', start, 60)
        start = len(text())
        chord('f')
        g['wait'](r'\[velvet.frame\]', start, 60)
        cell(0)
        g['wait'](r'\[velvet.state\] game=Snags state=Playing opened=0 flags=1 ', start, 60)
        start = len(text())
        control('mingle')
        g['wait'](r'\[velvet.state\] game=Mingle state=Playing .* moves=0 ', start, 60)
        g['focus'](surface)
        for k in ('left', 'up', 'right', 'down'):
            q.key(k)
            time.sleep(.3)
        g['wait'](r'\[velvet.state\] game=Mingle state=Playing .* moves=([1-9]\d*) ', start, 60)
        mingle_colours = screenshot('mingle')
        start = len(text())
        control('twins')
        g['wait'](r'\[velvet.state\] game=TwinFlames state=Playing .* pairs=0 turns=0\n', start, 60)
        start = len(text())
        cell(0)
        g['wait'](r'\[velvet.frame\]', start, 60)
        start = len(text())
        cell(1)
        g['wait'](r'\[velvet.state\] game=TwinFlames state=Playing .* turns=1\n', start, 60)
        twins_colours = screenshot('twin-flames')
        start = len(text())
        control('new')
        g['wait'](r'\[velvet.state\] game=TwinFlames state=Playing .* pairs=0 turns=0\n', start, 60)
        start = len(text())
        g['focus'](surface)
        q.key('f11')
        g['wait'](rf'\[app.private.retired\] target={target} task={task} ', start, 120)
        assert '[velvet.fatal]' not in text()
        result = dict(ok=True, opened=opened, colours=dict(snags=snags_colours, mingle=mingle_colours, twins=twins_colours))
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(result)
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
