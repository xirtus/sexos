#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Climax on Weft: microbenchmarks suite, thermal torture runner, JSON reporting,
frames, controls, and clean window retirement."""
import argparse
import json
import re
import shutil
import time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]


def colours(data, tile):
    """The distinct colours inside a window tile of a binary PPM (P6) screendump."""
    fields = []
    i = 0
    while len(fields) < 4:
        while data[i : i + 1].isspace():
            i += 1
        if data[i : i + 1] == b'#':
            i = data.index(b'\n', i)
            continue
        j = i
        while not data[j : j + 1].isspace():
            j += 1
        fields.append(data[i:j])
        i = j
    assert fields[0] == b'P6' and int(fields[3]) < 256, fields
    width, height = int(fields[1]), int(fields[2])
    i += 1
    x, y, w, h = tile
    seen = set()
    for row in range(max(0, y + 36), min(height, y + h - 8)):
        base = i + (row * width) * 3
        line = data[base + max(0, x + 8) * 3 : base + min(width, x + w - 8) * 3]
        seen.update(line[k : k + 3] for k in range(0, len(line), 3))
    return seen


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out = a.out.resolve()
    a.out.mkdir(parents=True, exist_ok=True)

    if not a.source_disk.exists():
        a.source_disk.parent.mkdir(parents=True, exist_ok=True)
        with a.source_disk.open('wb') as f:
            f.truncate(64 * 1024 * 1024)

    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)

    elf_candidates = [
        ROOT / 'target/x86_64-sex-app/release/climax',
        ROOT / 'iso_root/programs/climax',
    ]
    elf_path = next((path for path in elf_candidates if path.exists()), None)
    assert elf_path is not None, f'climax ELF not found in {elf_candidates}'
    elf = elf_path.read_bytes()
    assert elf[:4] == b'\x7fELF', 'climax binary is not valid ELF'
    install(disk, [('climax', elf)])

    g = boot(a.iso, disk, a.out, 'climax', a.resolution)
    text, q = g['text'], g['q']
    rw, rh = map(int, a.resolution.split('x'))

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        g['focus'](153)
        time.sleep(0.5)

        start = len(text())
        for k in 'run climax':
            q.key('spc' if k == ' ' else k)
            time.sleep(0.08)
        q.key('ret')

        m = g['wait'](r'\[climax.ready\] target=(\d+) task=(\d+)', start, 300)
        target, task = m[1], m[2]
        launch = g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', start, 60)
        surface = int(launch[1])
        g['wait'](r'\[climax.frame\] n=1\n', start, 300)
        time.sleep(2)

        tile = list(map(int, re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)', text())[-1]))
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
            matches = re.findall(rf'\[climax.control\] name={name} x=(-?\d+) y=(-?\d+)', text())
            assert matches, f'no control {name} in serial text'
            click_at(*map(int, matches[-1]))

        def screenshot(name):
            time.sleep(0.5)
            path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(path)})
            time.sleep(0.2)
            n = len(colours(path.read_bytes(), tile))
            assert n > 8, (name, 'blank window', n)
            return n

        # Initial view: benchmarks mode, ready state
        g['wait'](r'\[climax.state\] mode=Benchmarks suite_ran=false', start, 60)
        bench_colours = screenshot('benchmarks-ready')

        # Run Suite
        s_suite = len(text())
        control('run_suite')
        g['wait'](r'\[climax.bench_json\] (\{.*\})', s_suite, 120)
        g['wait'](r'\[climax.state\] mode=Benchmarks suite_ran=true', s_suite, 60)
        bench_ran_colours = screenshot('benchmarks-completed')

        # Switch to Thermal Torture mode
        s_torture = len(text())
        control('torture')
        g['wait'](r'\[climax.state\] mode=Thermal .* step=0/20', s_torture, 60)
        torture_ready_colours = screenshot('torture-ready')

        # Start Step (1 step)
        s_step = len(text())
        control('start_step')
        g['wait'](r'\[climax.state\] mode=Thermal .* step=1/20', s_step, 60)

        # Run Burst (5 steps -> step 6/20)
        s_burst = len(text())
        control('run_burst')
        g['wait'](r'\[climax.state\] mode=Thermal .* step=6/20', s_burst, 120)
        torture_burst_colours = screenshot('torture-burst')

        # Reset (back to step 0/20)
        s_reset = len(text())
        control('reset')
        g['wait'](r'\[climax.state\] mode=Thermal .* step=0/20', s_reset, 60)

        # Close window cleanly via Shell F11
        s_close = len(text())
        g['focus'](surface)
        q.key('f11')
        g['wait'](rf'\[app.private.retired\] target={target} task={task} ', s_close, 120)

        assert '[climax.fatal]' not in text(), 'fatal error found in serial trace'

        result = {
            'ok': True,
            'bench_colours': bench_colours,
            'bench_ran_colours': bench_ran_colours,
            'torture_ready_colours': torture_ready_colours,
            'torture_burst_colours': torture_burst_colours,
        }
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(f'[climax_gate] PASS: {result}')
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
