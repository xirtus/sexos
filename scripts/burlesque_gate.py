#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Burlesque on Weft: Keynote-class themes, transitions, live animated playback,
editable speaker notes, dual-card presenter view, PPTX/ODP roundtrip, PDF export,
and clean Shell close."""
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
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))

    ihdr = struct.pack('>IIBBBBB', out_w, out_h, 8, 2, 0, 0, 0)
    idat = zlib.compress(raw_data, 6)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', idat) + chunk(b'IEND', b'')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--screenshots-dir', type=Path, default=None)
    p.add_argument('--save-assets', action='store_true', help='Copy screenshots to docs/user/assets/')
    p.add_argument('--require-video', action='store_true', help='Require FFMPEG_RECORDING=2 WebM export profile in guest')
    a = p.parse_args()
    a.out = a.out.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    if a.screenshots_dir:
        a.screenshots_dir = a.screenshots_dir.resolve()
        a.screenshots_dir.mkdir(parents=True, exist_ok=True)

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
            ppm_path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(ppm_path)})
            time.sleep(0.2)
            ppm_bytes = ppm_path.read_bytes()
            n = len(colours(ppm_bytes, tile))
            assert n > 8, (name, 'blank window', n)

            png_bytes = ppm_to_png(ppm_bytes)
            png_path = a.out / f'{name}.png'
            png_path.write_bytes(png_bytes)
            if a.screenshots_dir:
                (a.screenshots_dir / f'{name}.png').write_bytes(png_bytes)
            if a.save_assets:
                assets_dir = ROOT / 'docs/user/assets'
                assets_dir.mkdir(parents=True, exist_ok=True)
                (assets_dir / f'{name}.png').write_bytes(png_bytes)

            return n

        # 1. Assert initial state
        g['wait'](r'\[burlesque.state\] slides=1 current=0 presenter=0 theme=burlesque_velvet transition=none', 0, 60)

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

        # 5. Theme Picker & Screenshot (burlesque-themes.png)
        start = len(text())
        control('theme')
        g['wait'](r'\[burlesque.theme\] name=Editorial Light', start, 60)
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=0 theme=editorial_light', start, 60)
        theme_colours = screenshot('burlesque-themes')

        # 6. Transitions & Animated Playback (burlesque-transition.png)
        start = len(text())
        control('transition')
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=0 theme=editorial_light transition=dissolve', start, 60)

        start = len(text())
        control('play_transition')
        g['wait'](r'\[burlesque.transition\] kind=dissolve frames=8', start, 60)
        transition_colours = screenshot('burlesque-transition')

        # 7. Intra-Slide Builds & Magic Move Transition (burlesque-magic-move.png)
        start = len(text())
        control('magic_move')
        g['wait'](r'\[burlesque.state\] .* transition=magic-move', start, 60)

        start = len(text())
        control('add_build')
        g['wait'](r'\[burlesque.build\] slide=1 step=1', start, 60)

        start = len(text())
        control('step_build')
        g['wait'](r'\[burlesque.build\] slide=1 step=0', start, 60)

        start = len(text())
        control('play_transition')
        g['wait'](r'\[burlesque.transition\] kind=magic-move frames=8', start, 60)
        magic_colours = screenshot('burlesque-magic-move')

        # 8. Presenter View & Speaker Notes (burlesque-presenter-notes.png)
        start = len(text())
        control('presenter')
        g['wait'](r'\[burlesque.presenter\] active=1 slide=1', start, 60)
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=1', start, 60)
        presenter_colours = screenshot('burlesque-presenter-notes')

        # Toggle back to editor
        start = len(text())
        control('presenter')
        g['wait'](r'\[burlesque.presenter\] active=0 slide=1', start, 60)
        g['wait'](r'\[burlesque.state\] slides=2 current=1 presenter=0', start, 60)

        # 9. Rehearsal HUD & Splicy WebM Video Export (burlesque-rehearse-video.png)
        start = len(text())
        control('prev')
        g['wait'](r'\[burlesque.state\] slides=2 current=0 presenter=0', start, 60)

        start = len(text())
        control('rehearse')
        g['wait'](r'\[burlesque.rehearse\] active=1', start, 60)
        time.sleep(1.2)
        rehearse_colours = screenshot('burlesque-rehearse-video')

        # Advance slide during rehearsal to record dwell timing, then commit timings
        control('next')
        time.sleep(0.6)
        start = len(text())
        control('rehearse')
        g['wait'](r'\[burlesque.rehearse\] active=0', start, 60)

        start = len(text())
        control('export_webm')
        if a.require_video:
            m_webm = g['wait'](r'\[burlesque.exported\] fmt=webm file=(\S+) bytes=([1-9]\d*) frames=([1-9]\d*)', start, 60)
            webm_status = 'encoded'
            webm_file = m_webm[1]
            webm_bytes = int(m_webm[2])
            webm_frames = int(m_webm[3])
            assert webm_bytes > 100, f'WebM too small: {webm_bytes}'
        else:
            m_webm = g['wait'](
                r'\[burlesque\.(?:exported\] fmt=webm file=(\S+) bytes=([1-9]\d*) frames=([1-9]\d*)|video\.unavailable\] encoder=splicy reason=ffmpeg_export_not_built frames=([1-9]\d*) hash=([0-9a-f]{16}))',
                start,
                60,
            )
            if m_webm[1]:
                webm_status = 'encoded'
                webm_file = m_webm[1]
                webm_bytes = int(m_webm[2])
                webm_frames = int(m_webm[3])
            else:
                webm_status = 'splicy_timeline_verified'
                webm_file = None
                webm_bytes = 0
                webm_frames = int(m_webm[4])
        assert webm_frames > 15, f'WebM frames too small: {webm_frames}'

        # 10. PPTX / ODP Roundtrip & PDF Export (burlesque-pptx-odp-pdf.png)
        start = len(text())
        control('save_pptx')
        m_pptx = g['wait'](r'\[burlesque.exported\] fmt=pptx file=(\S+) bytes=([1-9]\d*) pages=2', start, 60)
        pptx_bytes = int(m_pptx[2])
        assert pptx_bytes > 500, f'PPTX too small: {pptx_bytes}'

        start = len(text())
        control('import_pptx')
        g['wait'](r'\[burlesque.imported\] fmt=pptx slides=2 notes=[12]', start, 60)

        start = len(text())
        control('save_odp')
        m_odp = g['wait'](r'\[burlesque.exported\] fmt=odp file=(\S+) bytes=([1-9]\d*) pages=2', start, 60)
        odp_bytes = int(m_odp[2])
        assert odp_bytes > 500, f'ODP too small: {odp_bytes}'

        start = len(text())
        control('import_odp')
        g['wait'](r'\[burlesque.imported\] fmt=odp slides=2 notes=[12]', start, 60)

        start = len(text())
        control('export_pdf')
        m_exp = g['wait'](r'\[burlesque.exported\] fmt=pdf file=(\S+) bytes=([1-9]\d*) pages=2', start, 60)
        exported_file = m_exp[1]
        exported_bytes = int(m_exp[2])
        assert exported_bytes > 500, f'PDF too small: {exported_bytes}'
        pdf_colours = screenshot('burlesque-pptx-odp-pdf')

        # 11. Close window via F11
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
            webm_status=webm_status,
            webm_file=webm_file,
            webm_bytes=webm_bytes,
            webm_frames=webm_frames,
            colours=dict(
                theme=theme_colours,
                transition=transition_colours,
                magic_move=magic_colours,
                presenter=presenter_colours,
                rehearse=rehearse_colours,
                pdf=pdf_colours,
            ),
        )
        (a.out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(result)

    finally:
        g['stop']()


if __name__ == '__main__':
    main()
