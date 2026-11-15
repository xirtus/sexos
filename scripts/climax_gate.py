#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Climax on Weft: microbenchmarks suite, real-time telemetry charts, thermal torture runner,
"Compare with Linux" view loading docs/CLIMAX_VS_LINUX_2026_11_14.md, JSON reporting,
frames, controls, and clean window retirement."""
import argparse
import json
import re
import shutil
import struct
import time
import zlib
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_gate_geometry import maximize

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


def ppm_to_png(ppm_bytes, tile=None):
    """Convert PPM P6 image bytes to standard PNG format without external PIL dependency."""
    header_end = ppm_bytes.find(b'\n255\n')
    assert header_end != -1, 'valid PPM header'
    header = ppm_bytes[:header_end]
    raw_pixels = ppm_bytes[header_end + 5:]
    tokens = header.split()
    assert tokens[0] == b'P6', 'PPM P6 signature'
    w = int(tokens[1])
    h = int(tokens[2])

    if tile:
        tx, ty, tw, th = tile
        tx = max(0, min(tx, w))
        ty = max(0, min(ty, h))
        tw = max(1, min(tw, w - tx))
        th = max(1, min(th, h - ty))
        rows = []
        for y in range(ty, ty + th):
            start = (y * w + tx) * 3
            end = start + tw * 3
            rows.append(b'\x00' + raw_pixels[start:end])
        raw_data = b''.join(rows)
        out_w, out_h = tw, th
    else:
        rows = []
        row_len = w * 3
        for y in range(h):
            start = y * row_len
            rows.append(b'\x00' + raw_pixels[start:start + row_len])
        raw_data = b''.join(rows)
        out_w, out_h = w, h

    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)

    ihdr = struct.pack('>IIBBBBB', out_w, out_h, 8, 2, 0, 0, 0)
    idat = zlib.compress(raw_data, 6)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', idat) + chunk(b'IEND', b'')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--save-assets', action='store_true', help='Export screenshot PNGs to docs/user/assets/')
    p.add_argument('--screenshots-dir', type=Path, default=None)
    a = p.parse_args()
    a.out = a.out.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    if a.screenshots_dir:
        a.screenshots_dir = a.screenshots_dir.resolve()
        a.screenshots_dir.mkdir(parents=True, exist_ok=True)

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
        time.sleep(1)

        tile, before = maximize(g, surface)
        g['wait'](r'\[climax.frame\] n=\d+\n', before, 60)
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

        # Verify comparison document loaded
        g['wait'](r'\[climax.compare\] source=.* rows=9', start, 60)

        # Initial view: benchmarks mode, ready state
        g['wait'](r'\[climax.state\] mode=Benchmarks suite_ran=false', start, 60)
        bench_colours = screenshot('benchmarks-ready')

        # Run Suite
        s_suite = len(text())
        control('run_suite')
        g['wait'](r'\[climax.bench_json\] (\{.*\})', s_suite, 120)
        g['wait'](r'\[climax.state\] mode=Benchmarks suite_ran=true', s_suite, 60)
        bench_ran_colours = screenshot('benchmarks-completed')
        assert bench_ran_colours > 25, f'insufficient distinct colours in benchmarks-completed: {bench_ran_colours}'

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
        torture_burst_colours = screenshot('realtime-charts')
        assert torture_burst_colours > 25, f'insufficient distinct colours in realtime-charts: {torture_burst_colours}'

        # Reset (back to step 0/20)
        s_reset = len(text())
        control('reset')
        g['wait'](r'\[climax.state\] mode=Thermal .* step=0/20', s_reset, 60)

        # Switch to Compare with Linux mode & reload benchmark doc
        s_compare = len(text())
        control('compare_linux')
        g['wait'](r'\[climax.state\] mode=CompareLinux', s_compare, 60)
        s_reload = len(text())
        control('reload_doc')
        g['wait'](r'\[climax.compare\] source=\S+ rows=9 geomean_pct=134\.8', s_reload, 60)
        compare_linux_colours = screenshot('compare-linux')
        assert compare_linux_colours > 25, f'insufficient distinct colours in compare-linux: {compare_linux_colours}'

        # Save full 1280x800 QEMU desktop asset PNGs if requested
        if a.save_assets or a.screenshots_dir:
            target_dir = a.screenshots_dir if a.screenshots_dir else (ROOT / 'docs/user/assets')
            target_dir.mkdir(parents=True, exist_ok=True)
            rt_ppm = (a.out / 'realtime-charts.ppm').read_bytes()
            (target_dir / 'climax-realtime-charts.png').write_bytes(ppm_to_png(rt_ppm))
            cmp_ppm = (a.out / 'compare-linux.ppm').read_bytes()
            (target_dir / 'climax-compare-linux.png').write_bytes(ppm_to_png(cmp_ppm))

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
            'compare_linux_colours': compare_linux_colours,
        }
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(f'[climax_gate] PASS: {result}')
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
