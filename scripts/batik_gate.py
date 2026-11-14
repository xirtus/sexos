#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Batik on Weft: real pointer painting, color swatches, adjustments,
undo/redo, PNG saving, rendered frames, and clean Shell close."""
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

    elf = (ROOT / 'iso_root/programs/batik').read_bytes()
    assert elf[:4] == b'\x7fELF', 'valid ELF header'
    install(disk, [('batik', elf)])

    g = boot(a.iso, disk, a.out, 'batik', a.resolution)
    text, q = g['text'], g['q']
    rw, rh = map(int, a.resolution.split('x'))

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        g['focus'](153)
        time.sleep(0.5)

        start = len(text())
        for k in 'run batik':
            q.key('spc' if k == ' ' else k)
            time.sleep(0.08)
        q.key('ret')

        m = g['wait'](r'\[batik.ready\] target=(\d+) task=(\d+)', start, 300)
        target, task = m[1], m[2]
        launch = g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', start, 60)
        surface = int(launch[1])

        g['wait'](r'\[batik.frame\] n=1\n', start, 300)
        time.sleep(1)

        tile, before = maximize(g, surface)
        g['wait'](r'\[batik.frame\] n=\d+\n', before, 60)
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

        def drag_stroke(x1, y1, x2, y2, steps=6):
            q.cmd('input-send-event', {'events': [
                {'type': 'abs', 'data': {'axis': 'x', 'value': round((x1 + ox) * 32767 / (rw - 1))}},
                {'type': 'abs', 'data': {'axis': 'y', 'value': round((y1 + oy) * 32767 / (rh - 1))}}]})
            time.sleep(0.15)
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': True, 'button': 'left'}}]})
            time.sleep(0.1)

            for step in range(1, steps + 1):
                t = step / steps
                cx = x1 + t * (x2 - x1)
                cy = y1 + t * (y2 - y1)
                q.cmd('input-send-event', {'events': [
                    {'type': 'abs', 'data': {'axis': 'x', 'value': round((cx + ox) * 32767 / (rw - 1))}},
                    {'type': 'abs', 'data': {'axis': 'y', 'value': round((cy + oy) * 32767 / (rh - 1))}}]})
                time.sleep(0.08)

            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': False, 'button': 'left'}}]})
            time.sleep(0.15)

        def control(name):
            matches = re.findall(rf'\[batik.control\] name={name} x=(-?\d+) y=(-?\d+)', text())
            assert matches, f'control {name} not found'
            click_at(*map(int, matches[-1]))

        def canvas_coords():
            matches = re.findall(r'\[batik.canvas\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)', text())
            assert matches, 'no canvas rect reported'
            return list(map(int, matches[-1]))

        def screenshot(name):
            time.sleep(0.5)
            path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(path)})
            time.sleep(0.2)
            n = len(colours(path.read_bytes(), tile))
            assert n > 8, (name, 'blank window', n)
            return n

        # 1. Assert initial state
        start = len(text())
        m = g['wait'](r'\[batik.state\] layers=(\d+) tool=Brush undo=0 pixel0=([0-9a-fA-F]+)', 0, 60)
        initial_layers = int(m[1])
        initial_pixel0 = m[2]

        # 2. Click color red swatch
        start = len(text())
        control('color_red')
        time.sleep(0.3)

        # 3. Drag a brush stroke on the canvas
        c_x, c_y, c_w, c_h = canvas_coords()
        start = len(text())
        drag_stroke(c_x + 20, c_y + 20, c_x + 50, c_y + 50)
        m_stroke = g['wait'](r'\[batik.state\] layers=(\d+) tool=Brush undo=1 ', start, 60)
        assert m_stroke, 'stroke failed to register undo=1'

        paint_colours = screenshot('batik-painted')

        # 4. Trigger Invert adjustment
        start = len(text())
        control('invert')
        m_invert = g['wait'](r'\[batik.state\] layers=(\d+) tool=Brush undo=2 pixel0=([0-9a-fA-F]+)', start, 60)
        inverted_pixel0 = m_invert[2]
        assert inverted_pixel0 != initial_pixel0, 'invert should invert pixel0'

        invert_colours = screenshot('batik-inverted')

        # 5. Trigger Undo (reverts invert)
        start = len(text())
        control('undo')
        m_undo = g['wait'](r'\[batik.state\] layers=(\d+) tool=Brush undo=1 pixel0=([0-9a-fA-F]+)', start, 60)
        restored_pixel0 = m_undo[2]
        assert restored_pixel0 == initial_pixel0, 'undo should restore pixel0'

        # 6. Save PNG
        start = len(text())
        control('save_png')
        m_saved = g['wait'](r'\[batik.saved\] file=(\S+) bytes=([1-9]\d*)', start, 60)
        saved_file = m_saved[1]
        saved_bytes = int(m_saved[2])
        assert saved_bytes > 64, f'saved file too small: {saved_bytes}'

        # 7. Close window via F11
        start = len(text())
        g['focus'](surface)
        q.key('f11')
        g['wait'](rf'\[app.private.retired\] target={target} task={task} ', start, 120)

        assert '[batik.fatal]' not in text()

        result = dict(
            ok=True,
            target=target,
            task=task,
            layers=initial_layers,
            saved_file=saved_file,
            saved_bytes=saved_bytes,
            colours=dict(painted=paint_colours, inverted=invert_colours),
        )
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(result)

    finally:
        g['stop']()


if __name__ == '__main__':
    main()
