#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Batik Pixelmator-level gate: multi-layer blend modes, 8-bit paintable layer masks,
pressure-tapered brushes, lasso & magic wand selections with marching ants,
spatial & color filters, and full PNG/JPEG/WebP/PSD export & import."""
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
from program_gate_geometry import colours, maximize

ROOT = Path(__file__).resolve().parents[1]


def ppm_to_png(ppm_bytes: bytes) -> bytes:
    """Converts raw PPM (P6) bytes into a standard PNG byte string using zlib & struct."""
    idx = 0

    def next_token():
        nonlocal idx
        while idx < len(ppm_bytes) and ppm_bytes[idx : idx + 1] in b' \t\r\n':
            idx += 1
        if idx < len(ppm_bytes) and ppm_bytes[idx : idx + 1] == b'#':
            while idx < len(ppm_bytes) and ppm_bytes[idx : idx + 1] != b'\n':
                idx += 1
            return next_token()
        start = idx
        while idx < len(ppm_bytes) and ppm_bytes[idx : idx + 1] not in b' \t\r\n#':
            idx += 1
        return ppm_bytes[start:idx]

    magic = next_token()
    assert magic == b'P6', f'expected P6 magic, got {magic}'
    w = int(next_token())
    h = int(next_token())
    maxval = int(next_token())
    assert maxval == 255, f'expected maxval 255, got {maxval}'

    while idx < len(ppm_bytes) and ppm_bytes[idx : idx + 1] in b' \t\r\n':
        idx += 1
    rgb = ppm_bytes[idx : idx + w * h * 3]

    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', crc)

    png = bytearray(b'\x89PNG\r\n\x1a\n')
    ihdr_data = struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)
    png.extend(chunk(b'IHDR', ihdr_data))

    raw_lines = bytearray()
    for row in range(h):
        raw_lines.append(0)  # filter type None
        raw_lines.extend(rgb[row * w * 3 : (row + 1) * w * 3])

    compressed = zlib.compress(bytes(raw_lines), 9)
    png.extend(chunk(b'IDAT', compressed))
    png.extend(chunk(b'IEND', b''))
    return bytes(png)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--screenshots-dir', default=None, type=Path)
    p.add_argument('--save-assets', action='store_true', help='Save screenshots to docs/user/assets/')
    a = p.parse_args()
    a.out = a.out.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    if a.screenshots_dir:
        a.screenshots_dir = a.screenshots_dir.resolve()
        a.screenshots_dir.mkdir(parents=True, exist_ok=True)
    if a.save_assets:
        (ROOT / 'docs/user/assets').mkdir(parents=True, exist_ok=True)

    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)

    elf = (ROOT / 'iso_root/programs/batik').read_bytes()
    assert elf[:4] == b'\x7fELF', 'valid ELF header'
    install(disk, [('batik', elf)])

    g = boot(a.iso, disk, a.out, 'batik', a.resolution)
    text, q = g['text'], g['q']
    rw, rh = map(int, a.resolution.split('x'))

    screenshots_saved = {}

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
            ppm_path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(ppm_path)})
            time.sleep(0.2)
            ppm_bytes = ppm_path.read_bytes()
            n = len(colours(ppm_bytes, tile))
            assert n > 8, (name, 'blank window', n)

            png_bytes = ppm_to_png(ppm_bytes)
            png_out = a.out / f'{name}.png'
            png_out.write_bytes(png_bytes)
            if a.screenshots_dir:
                (a.screenshots_dir / f'{name}.png').write_bytes(png_bytes)
            if a.save_assets:
                (ROOT / 'docs/user/assets' / f'{name}.png').write_bytes(png_bytes)
            screenshots_saved[name] = str(png_out)

            alias_map = {
                'batik-adjustment-layers': 'adjustment-layers',
                'batik-text-layer': 'text-layer',
                'batik-filter-stack': 'filter-stack',
            }
            if name in alias_map:
                alias = alias_map[name]
                (a.out / f'{alias}.ppm').write_bytes(ppm_bytes)
                (a.out / f'{alias}.png').write_bytes(png_bytes)
                if a.screenshots_dir:
                    (a.screenshots_dir / f'{alias}.png').write_bytes(png_bytes)
                if a.save_assets:
                    (ROOT / 'docs/user/assets' / f'{alias}.png').write_bytes(png_bytes)

            return n

        # 1. Assert initial state
        start = len(text())
        m = g['wait'](
            r'\[batik.state\] layers=(\d+) active=(\d+) blend=(\w+) mask=(\w+) edit_mask=(\w+) tool=(\w+) sel=(\w+) undo=(\d+) pixel0=([0-9a-fA-F]+)',
            0,
            60,
        )
        initial_layers = int(m[1])
        initial_pixel0 = m[9]

        c_x, c_y, c_w, c_h = canvas_coords()

        # 2. Capability: Layers & Blend Mode
        start = len(text())
        control('new_layer')
        g['wait'](r'\[batik.state\] layers=2 ', start, 60)

        start = len(text())
        control('color_blue')
        time.sleep(0.2)
        control('tool_bucket')
        time.sleep(0.2)
        click_at(c_x + c_w // 2, c_y + c_h // 2)
        time.sleep(0.3)

        start = len(text())
        control('blend_multiply')
        g['wait'](r'\[batik.state\].*?blend=Multiply', start, 60)
        screenshot('batik-layers-blend')

        # 3. Capability: Layer Mask
        start = len(text())
        control('add_mask')
        g['wait'](r'\[batik.state\].*?mask=yes', start, 60)

        start = len(text())
        control('edit_mask')
        g['wait'](r'\[batik.state\].*?edit_mask=yes', start, 60)

        start = len(text())
        control('tool_brush')
        control('color_black')
        drag_stroke(c_x + 30, c_y + 30, c_x + 90, c_y + 90, steps=8)
        screenshot('batik-layer-mask')

        # 4. Capability: Brushes with Pressure
        start = len(text())
        control('edit_mask')  # exit mask editing
        g['wait'](r'\[batik.state\].*?edit_mask=no', start, 60)

        start = len(text())
        control('new_layer')
        g['wait'](r'\[batik.state\] layers=3 ', start, 60)

        start = len(text())
        control('preset_chisel')
        control('curve_taper')
        control('color_yellow')
        drag_stroke(c_x + 20, c_y + 120, c_x + 180, c_y + 180, steps=15)
        screenshot('batik-brush-pressure')

        # 5. Capability: Selections (Lasso & Magic Wand) + Marching Ants
        def drag_polygon(pts):
            x0, y0 = pts[0]
            q.cmd('input-send-event', {'events': [
                {'type': 'abs', 'data': {'axis': 'x', 'value': round((x0 + ox) * 32767 / (rw - 1))}},
                {'type': 'abs', 'data': {'axis': 'y', 'value': round((y0 + oy) * 32767 / (rh - 1))}}]})
            time.sleep(0.15)
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': True, 'button': 'left'}}]})
            time.sleep(0.1)
            for px, py in pts[1:]:
                q.cmd('input-send-event', {'events': [
                    {'type': 'abs', 'data': {'axis': 'x', 'value': round((px + ox) * 32767 / (rw - 1))}},
                    {'type': 'abs', 'data': {'axis': 'y', 'value': round((py + oy) * 32767 / (rh - 1))}}]})
                time.sleep(0.1)
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': False, 'button': 'left'}}]})
            time.sleep(0.2)

        start = len(text())
        control('tool_lasso')
        time.sleep(0.2)
        drag_polygon([
            (c_x + 35, c_y + 35),
            (c_x + 145, c_y + 45),
            (c_x + 120, c_y + 145),
            (c_x + 40, c_y + 130),
        ])
        g['wait'](r'\[batik.state\].*?sel=yes', start, 60)

        start = len(text())
        control('deselect')
        g['wait'](r'\[batik.state\].*?sel=no', start, 60)

        start = len(text())
        control('tool_wand')
        time.sleep(0.2)
        click_at(c_x + c_w // 2, c_y + c_h // 2)
        g['wait'](r'\[batik.state\].*?sel=yes', start, 60)
        screenshot('batik-selection-lasso-wand')

        # 6. Capability: Filters
        start = len(text())
        control('deselect')
        g['wait'](r'\[batik.state\].*?sel=no', start, 60)

        start = len(text())
        control('sepia')
        time.sleep(0.5)
        screenshot('batik-filters')

        # 7. Capability: Non-Destructive Adjustment Layers (Curves & Hue/Sat)
        start = len(text())
        control('add_curves_adj')
        g['wait'](r'\[batik.state\].*?adj_layers=1', start, 60)

        start = len(text())
        control('add_curves_adj')  # cycle curves preset
        time.sleep(0.3)
        control('add_huesat_adj')
        g['wait'](r'\[batik.state\].*?adj_layers=2', start, 60)
        screenshot('batik-adjustment-layers')

        # 8. Capability: Scalable Editable Text Layer
        start = len(text())
        control('add_text_layer')
        g['wait'](r'\[batik.state\].*?text_layers=1', start, 60)

        # Type additional text via QEMU key injection
        g['focus'](surface)
        time.sleep(0.2)
        for k in ' 2026':
            q.key('spc' if k == ' ' else k)
            time.sleep(0.08)
        time.sleep(0.3)

        start = len(text())
        control('edit_text_style')  # cycle text style
        time.sleep(0.3)
        screenshot('batik-text-layer')

        # 9. Capability: Non-Destructive Layer Filter Stack
        start = len(text())
        control('push_fx_stack')
        g['wait'](r'\[batik.state\].*?fx_count=1.*?fx_enabled=yes', start, 60)

        start = len(text())
        control('push_fx_stack')
        g['wait'](r'\[batik.state\].*?fx_count=2.*?fx_enabled=yes', start, 60)
        screenshot('batik-filter-stack')

        start = len(text())
        control('toggle_fx_stack')  # toggle FX node disabled
        g['wait'](r'\[batik.state\].*?fx_enabled=no', start, 60)

        start = len(text())
        control('toggle_fx_stack')  # toggle FX node back enabled
        g['wait'](r'\[batik.state\].*?fx_enabled=yes', start, 60)

        # 10. Capability: Codecs Export & Import
        start = len(text())
        control('export_png')
        m_png = g['wait'](r'\[batik.exported\] fmt=png bytes=(\d+)', start, 60)
        assert int(m_png[1]) > 64, 'png export'

        start = len(text())
        control('export_jpg')
        m_jpg = g['wait'](r'\[batik.exported\] fmt=jpg bytes=(\d+)', start, 60)
        assert int(m_jpg[1]) > 64, 'jpg export'

        start = len(text())
        control('export_webp')
        m_webp = g['wait'](r'\[batik.exported\] fmt=webp bytes=(\d+)', start, 60)
        assert int(m_webp[1]) > 64, 'webp export'

        start = len(text())
        control('export_psd')
        m_psd = g['wait'](r'\[batik.exported\] fmt=psd bytes=(\d+)', start, 60)
        assert int(m_psd[1]) > 128, 'psd export'

        start = len(text())
        control('import_psd')
        m_imp_psd = g['wait'](r'\[batik.imported\] fmt=psd layers=(\d+) w=(\d+) h=(\d+)', start, 60)
        assert int(m_imp_psd[1]) >= 1, 'psd import layers'

        start = len(text())
        control('import_png')
        m_imp_png = g['wait'](r'\[batik.imported\] fmt=png layers=(\d+) w=(\d+) h=(\d+)', start, 60)
        assert int(m_imp_png[1]) == 1, 'png import'

        start = len(text())
        control('import_jpg')
        m_imp_jpg = g['wait'](r'\[batik.imported\] fmt=jpg layers=(\d+) w=(\d+) h=(\d+)', start, 60)
        assert int(m_imp_jpg[1]) == 1, 'jpg import'

        start = len(text())
        control('import_webp')
        m_imp_webp = g['wait'](r'\[batik.imported\] fmt=webp layers=(\d+) w=(\d+) h=(\d+)', start, 60)
        assert int(m_imp_webp[1]) == 1, 'webp import'

        screenshot('batik-codecs-io')

        # 8. Close window cleanly via F11
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
            screenshots=screenshots_saved,
            exported=dict(
                png=int(m_png[1]),
                jpg=int(m_jpg[1]),
                webp=int(m_webp[1]),
                psd=int(m_psd[1]),
            ),
        )
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(result)

    finally:
        g['stop']()


if __name__ == '__main__':
    main()
