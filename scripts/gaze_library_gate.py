#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Gaze Photo Library QEMU Gate (G-30):
Verifies photo library catalog ingestion, user albums, faces-free smart search by date
and offline-geocoded GPS place (explicitly refusing biometric face/person queries),
and non-destructive crop, EV exposure, and shared Batik filter presets/stacks with
full persistence across restarts while leaving original photo files untouched on disk."""
import argparse
import hashlib
import http.server
import io
import math
import os
import re
import shutil
import struct
import threading
import time
import zlib
from pathlib import Path
try:
    from PIL import Image, ImageDraw
except ImportError:
    Image = None
    ImageDraw = None
from program_gate_geometry import colours, maximize

ROOT = Path(__file__).resolve().parents[1]


def ppm_to_png(ppm_bytes: bytes) -> bytes:
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
    png.extend(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)))
    raw_lines = bytearray()
    for row in range(h):
        raw_lines.append(0)
        raw_lines.extend(rgb[row * w * 3 : (row + 1) * w * 3])
    png.extend(chunk(b'IDAT', zlib.compress(bytes(raw_lines), 9)))
    png.extend(chunk(b'IEND', b''))
    return bytes(png)


def build_jpeg_exif_app1(date_exif: str, lat: float, lon: float, model: str) -> bytes:
    """Builds a standards-compliant little-endian TIFF EXIF APP1 block with DateTimeOriginal & WGS84 GPS."""
    tiff = bytearray(b'II\x2a\x00\x08\x00\x00\x00')
    model_b = model.encode('ascii') + b'\x00'
    if len(model_b) % 2:
        model_b += b'\x00'
    date_b = date_exif.encode('ascii') + b'\x00'
    if len(date_b) % 2:
        date_b += b'\x00'

    model_off = 50
    exif_ifd_off = model_off + len(model_b)
    date_off = exif_ifd_off + 18
    gps_ifd_off = date_off + len(date_b)
    gps_lat_off = gps_ifd_off + 54
    gps_lon_off = gps_lat_off + 24

    def entry(tag: int, ty: int, count: int, val_off: int) -> bytes:
        return struct.pack('<HHII', tag, ty, count, val_off)

    # IFD0: Model, ExifIFD, GPSIFD
    tiff.extend(struct.pack('<H', 3))
    tiff.extend(entry(0x0110, 2, len(model) + 1, model_off))
    tiff.extend(entry(0x8769, 4, 1, exif_ifd_off))
    tiff.extend(entry(0x8825, 4, 1, gps_ifd_off))
    tiff.extend(struct.pack('<I', 0))
    tiff.extend(model_b)

    # Exif SubIFD: DateTimeOriginal (0x9003)
    tiff.extend(struct.pack('<H', 1))
    tiff.extend(entry(0x9003, 2, len(date_exif) + 1, date_off))
    tiff.extend(struct.pack('<I', 0))
    tiff.extend(date_b)

    # GPS SubIFD: LatRef, Lat, LonRef, Lon
    lat_ref = ord('N') if lat >= 0 else ord('S')
    lon_ref = ord('E') if lon >= 0 else ord('W')
    tiff.extend(struct.pack('<H', 4))
    tiff.extend(entry(0x0001, 2, 2, lat_ref))
    tiff.extend(entry(0x0002, 5, 3, gps_lat_off))
    tiff.extend(entry(0x0003, 2, 2, lon_ref))
    tiff.extend(entry(0x0004, 5, 3, gps_lon_off))
    tiff.extend(struct.pack('<I', 0))

    def dms_rationals(deg_val: float) -> bytes:
        a = abs(deg_val)
        d = int(math.floor(a))
        rem_m = (a - d) * 60.0
        m = int(math.floor(rem_m))
        sec_x10000 = int(round((rem_m - m) * 60.0 * 10000.0))
        return struct.pack('<IIIIII', d, 1, m, 1, sec_x10000, 10000)

    tiff.extend(dms_rationals(lat))
    tiff.extend(dms_rationals(lon))

    payload = b'Exif\x00\x00' + bytes(tiff)
    return b'\xff\xe1' + struct.pack('>H', 2 + len(payload)) + payload


def make_scene_jpeg(theme: str, date_exif: str, lat: float, lon: float, model: str) -> bytes:
    w, h = 640, 420
    if Image is None:
        raw_lines = bytearray()
        for y in range(h):
            t = y / max(1, h - 1)
            raw_lines.append(0)
            for x in range(w):
                u = x / max(1, w - 1)
                if theme == 'paris':
                    c = (int(235 * (1 - 0.5 * t)), int(155 * (1 - 0.4 * t)), int(100 + 100 * t))
                elif theme == 'tokyo':
                    c = (int(45 + 150 * t), int(35 + 80 * u), int(110 + 120 * (1 - 0.3 * t)))
                elif theme == 'reykjavik':
                    c = (int(25 + 40 * u), int(95 + 120 * (1 - t)), int(130 + 100 * (1 - 0.2 * t)))
                else:
                    c = (int(65 + 150 * (1 - abs(t - 0.5))), int(80 + 100 * (1 - t * 0.5)), int(125 + 85 * t))
                raw_lines.extend(bytes((max(0, min(255, c[0])), max(0, min(255, c[1])), max(0, min(255, c[2])))))

        def chunk(tag: bytes, data: bytes) -> bytes:
            return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data) & 0xFFFFFFFF)

        app1 = build_jpeg_exif_app1(date_exif, lat, lon, model)
        tiff_body = app1[10:]  # strip FF E1 <len> Exif\0\0
        png = bytearray(b'\x89PNG\r\n\x1a\n')
        png.extend(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)))
        png.extend(chunk(b'eXIf', tiff_body))
        png.extend(chunk(b'IDAT', zlib.compress(bytes(raw_lines), 6)))
        png.extend(chunk(b'IEND', b''))
        return bytes(png)

    img = Image.new('RGB', (w, h))
    px = img.load()
    for y in range(h):
        t = y / max(1, h - 1)
        for x in range(w):
            u = x / max(1, w - 1)
            if theme == 'paris':
                r = int(245 * (1 - 0.55 * t) + 35 * math.sin(u * 3.14))
                g = int(150 * (1 - 0.5 * t) + 60 * (1 - t))
                b = int(95 + 110 * t)
            elif theme == 'tokyo':
                r = int(28 + 165 * (t ** 1.6) * (0.5 + 0.5 * math.sin(u * 9.0)))
                g = int(24 + 95 * (1 - abs(u - 0.5) * 1.6))
                b = int(90 + 150 * (1 - 0.4 * t))
            elif theme == 'reykjavik':
                aurora = max(0.0, math.sin(u * 6.0 + t * 3.0))
                r = int(18 + 40 * aurora)
                g = int(65 + 165 * aurora * (1 - 0.4 * t))
                b = int(115 + 115 * (1 - 0.3 * t))
            else:  # newyork
                r = int(50 + 185 * (1 - abs(t - 0.55)))
                g = int(65 + 125 * (1 - t * 0.6))
                b = int(120 + 95 * t)
            px[x, y] = (max(0, min(255, r)), max(0, min(255, g)), max(0, min(255, b)))

    draw = ImageDraw.Draw(img)
    if theme == 'paris':
        # Sun + Eiffel silhouette + Seine reflection
        draw.ellipse((430, 65, 520, 155), fill=(255, 228, 132))
        draw.polygon([(190, 340), (245, 95), (260, 95), (315, 340)], fill=(42, 28, 38))
        draw.rectangle((0, 340, w, h), fill=(36, 48, 76))
        for ry in range(348, h, 10):
            draw.line((420, ry, 530, ry), fill=(245, 192, 102), width=3)
    elif theme == 'tokyo':
        # Skyline towers + neon windows
        towers = [(60, 170, 140), (165, 120, 255), (290, 85, 365), (400, 145, 495), (520, 160, 605)]
        for x0, top, x1 in towers:
            draw.rectangle((x0, top, x1, h), fill=(18, 22, 42), outline=(236, 72, 153), width=2)
            for wy in range(top + 16, h - 16, 22):
                for wx in range(x0 + 10, x1 - 12, 18):
                    col = (56, 189, 248) if (wx + wy) % 3 else (244, 114, 182)
                    draw.rectangle((wx, wy, wx + 9, wy + 10), fill=col)
    elif theme == 'reykjavik':
        # Glacial mountains + fjord
        draw.polygon([(0, 310), (160, 165), (310, 290), (470, 145), (640, 300), (640, h), (0, h)], fill=(215, 236, 248))
        draw.polygon([(80, 310), (235, 205), (390, 310)], fill=(148, 192, 218))
        draw.rectangle((0, 315, w, h), fill=(16, 52, 78))
    else:
        # Manhattan skyline + warm horizon
        heights = [210, 150, 105, 180, 75, 140, 195, 125, 165]
        for i, top in enumerate(heights):
            x0 = 25 + i * 66
            draw.rectangle((x0, top, x0 + 52, 345), fill=(30, 36, 54), outline=(251, 191, 36), width=1)
        draw.rectangle((0, 345, w, h), fill=(24, 34, 58))

    buf = io.BytesIO()
    img.save(buf, 'JPEG', quality=93, subsampling=0)
    raw = buf.getvalue()
    app1 = build_jpeg_exif_app1(date_exif, lat, lon, model)
    return raw[:2] + app1 + raw[2:]


PARIS_BYTES = make_scene_jpeg('paris', '2026:05:14 16:30:00', 48.8566, 2.3522, 'Leica Q3')
TOKYO_BYTES = make_scene_jpeg('tokyo', '2026:11:02 20:15:00', 35.6762, 139.6503, 'Sony A7CR')
REYKJAVIK_BYTES = make_scene_jpeg('reykjavik', '2025:08:21 11:00:00', 64.1466, -21.9426, 'Fujifilm X100VI')
NEWYORK_BYTES = make_scene_jpeg('newyork', '2026:05:28 14:45:00', 40.7128, -74.0060, 'Ricoh GR IIIx')

GAZE_ELF = (ROOT / 'iso_root/programs/gaze').read_bytes()
assert GAZE_ELF[:4] == b'\x7fELF', 'valid Gaze ELF binary required in iso_root/programs/gaze'

FILES = {
    '/gaze': GAZE_ELF,
    '/paris.jpg': PARIS_BYTES,
    '/tokyo.jpg': TOKYO_BYTES,
    '/reykjavik.jpg': REYKJAVIK_BYTES,
    '/newyork.jpg': NEWYORK_BYTES,
}


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = FILES.get(self.path)
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class QuietServer(http.server.ThreadingHTTPServer):
    daemon_threads = True


server = QuietServer(('127.0.0.1', 0), Handler)
PORT = server.server_address[1]
threading.Thread(target=server.serve_forever, daemon=True).start()

os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot

KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'source-disk', 'out'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--save-assets', action='store_true', help='Save 1280x800 PNGs into docs/user/assets/')
    a = p.parse_args()
    a.out = a.out.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    if a.save_assets:
        (ROOT / 'docs/user/assets').mkdir(parents=True, exist_ok=True)

    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024 ** 3)

    g = boot(a.iso, disk, a.out, 'gaze_library', a.resolution)
    t, q = g['text'], g['q']
    rw, rh = map(int, a.resolution.split('x'))

    def send_text(s: str):
        for ch in s:
            if ch == '@':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': '2'}], 'hold-time': 80})
            elif ch == ':':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'semicolon'}], 'hold-time': 80})
            elif ch.isupper():
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': ch.lower()}], 'hold-time': 80})
            else:
                q.key(KEYS.get(ch, ch))
            time.sleep(0.06)

    def spindle(cmd: str, pattern: str, timeout: int = 120):
        g['focus'](153)
        time.sleep(0.4)
        s = len(t())
        send_text(cmd)
        q.key('ret')
        return g['wait'](pattern, s, timeout)

    def fetched(url_path: str, dest: str):
        m = spindle(
            f'fetch http://10.0.2.2:{PORT}{url_path} {dest}',
            r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',
            300,
        )
        assert m[1].startswith('Saved'), m[1]
        return int(m[1].split()[1])

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        time.sleep(1.5)

        fetched('/gaze', '/bin/gaze')
        spindle('mkdir photos', r'\[spindle.fs.result\] kind=\d+ status=0')
        paris_saved = fetched('/paris.jpg', 'photos/paris.jpg')
        tokyo_saved = fetched('/tokyo.jpg', 'photos/tokyo.jpg')
        reyk_saved = fetched('/reykjavik.jpg', 'photos/reykjavik.jpg')
        ny_saved = fetched('/newyork.jpg', 'photos/newyork.jpg')
        assert paris_saved == len(PARIS_BYTES)
        assert tokyo_saved == len(TOKYO_BYTES)
        assert reyk_saved == len(REYKJAVIK_BYTES)
        assert ny_saved == len(NEWYORK_BYTES)

        # 1. Launch Gaze in Photo Library mode on @photos
        s = len(t())
        spindle('run gaze --library @photos', r'\[gaze\.library\.ready\] photos=4 albums=0 mode=library')
        ready = g['wait'](r'\[view\.ready\] target=(\d+) task=(\d+)', s, 60)
        target, task = ready[1], ready[2]
        launch = g['wait'](rf'\[shell\.program\.launch\] target={target} task={task} surface=(\d+)', s, 60)
        surface = int(launch[1])
        time.sleep(0.8)

        tile, before = maximize(g, surface)
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', before, 60)
        time.sleep(0.8)

        ox, oy = tile[0] + 8, tile[1] + 36

        def click_at(x: int, y: int):
            ax, ay = x + ox, y + oy
            q.cmd('input-send-event', {'events': [
                {'type': 'abs', 'data': {'axis': 'x', 'value': round(ax * 32767 / (rw - 1))}},
                {'type': 'abs', 'data': {'axis': 'y', 'value': round(ay * 32767 / (rh - 1))}},
            ]})
            time.sleep(0.18)
            for down in (True, False):
                q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
                time.sleep(0.14)

        def click_control(name: str, search_from: int = 0):
            matches = re.findall(rf'\[gaze\.control\] name={re.escape(name)} x=(-?\d+) y=(-?\d+)', t()[search_from:])
            if not matches:
                matches = re.findall(rf'\[gaze\.control\] name={re.escape(name)} x=(-?\d+) y=(-?\d+)', t())
            assert matches, f'control {name} not found in Gaze controls'
            cx, cy = map(int, matches[-1])
            click_at(cx, cy)

        def screenshot(name: str) -> int:
            time.sleep(0.6)
            ppm_path = a.out / f'{name}.ppm'
            q.cmd('screendump', {'filename': str(ppm_path)})
            time.sleep(0.25)
            ppm_bytes = ppm_path.read_bytes()
            n_col = len(colours(ppm_bytes, tile))
            assert n_col > 40, f'{name} looks blank ({n_col} colours)'
            png_bytes = ppm_to_png(ppm_bytes)
            (a.out / f'{name}.png').write_bytes(png_bytes)
            if a.save_assets:
                (ROOT / 'docs/user/assets' / f'{name}.png').write_bytes(png_bytes)
            return n_col

        g['focus'](surface)
        time.sleep(0.4)

        # 2. Create User Album "Europe" and add Paris + Reykjavik
        s = len(t())
        q.key('a')
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', s, 30)
        s = len(t())
        send_text('Europe')
        q.key('ret')
        g['wait'](r'\[gaze\.album\.created\] idx=0 name=Europe', s, 60)
        g['wait'](r'\[gaze\.persisted\] bytes=\d+ photos=4 albums=1', s, 60)

        # Find photo indices for paris.jpg and reykjavik.jpg by navigating / checking selection
        # Chronological newest-first order in Gaze:
        # 0: tokyo.jpg (2026-11-02), 1: newyork.jpg (2026-05-28), 2: paris.jpg (2026-05-14), 3: reykjavik.jpg (2025-08-21)
        # Let's select via Home + Right steps and verify via [gaze.selected]!
        paris_idx = None
        paris_path = None
        reyk_idx = None
        reyk_path = None
        s = len(t())
        q.key('home')
        m_sel = g['wait'](r'\[gaze\.selected\] idx=(\d+) name=(\S+) path=(\S+)', s, 30)
        seen_order = [(int(m_sel[1]), m_sel[2], m_sel[3])]
        for _ in range(3):
            s = len(t())
            q.key('right')
            m_sel = g['wait'](r'\[gaze\.selected\] idx=(\d+) name=(\S+) path=(\S+)', s, 30)
            seen_order.append((int(m_sel[1]), m_sel[2], m_sel[3]))

        for idx_val, fname, fpath in seen_order:
            if fname == 'paris.jpg':
                paris_idx, paris_path = idx_val, fpath
            elif fname == 'reykjavik.jpg':
                reyk_idx, reyk_path = idx_val, fpath
        assert paris_idx is not None and reyk_idx is not None, f'missing photos in {seen_order}'

        # Add Paris & Reykjavik to "Europe" album
        for p_idx, expected_path in ((paris_idx, paris_path), (reyk_idx, reyk_path)):
            s = len(t())
            click_control(f'photo_idx_{p_idx}')
            g['wait'](rf'\[gaze\.selected\] idx={p_idx} ', s, 30)
            s = len(t())
            q.key('m')
            g['wait'](rf'\[gaze\.album\.toggled\] album=Europe path={re.escape(expected_path)} member=1', s, 30)

        # Select "Europe" album (shortcut '1') -> visible=2
        s = len(t())
        q.key('1')
        g['wait'](r'\[gaze\.album\.selected\] name=Europe visible=2', s, 30)
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', s, 30)
        albums_colours = screenshot('gaze-library-albums')

        # Return to All Photos ('0') -> visible=4
        s = len(t())
        q.key('0')
        g['wait'](r'\[gaze\.album\.selected\] name=AllPhotos visible=4', s, 30)

        # 3. Faces-Free Smart Search by Date & Place
        # 3a. Verify biometric face query is explicitly refused!
        s = len(t())
        q.key('slash')
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', s, 30)
        s = len(t())
        send_text('face:alice')
        q.key('ret')
        g['wait'](r'\[gaze\.search\] query="face:alice" visible=0 total=4 biometric_refused=1', s, 30)

        # Clear search
        s = len(t())
        click_control('btn_clear_search')
        g['wait'](r'\[gaze\.search\] query="" visible=4 total=4 biometric_refused=0', s, 30)

        # 3b. Search by Place ("Japan" -> Tokyo 1 match)
        s = len(t())
        q.key('slash')
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', s, 30)
        s = len(t())
        send_text('Japan')
        q.key('ret')
        g['wait'](r'\[gaze\.search\] query="Japan" visible=1 total=4 biometric_refused=0', s, 30)

        # Clear and search by Date ("2026-05" -> Paris + New York = 2 matches) for rich screenshot!
        s = len(t())
        click_control('btn_clear_search')
        g['wait'](r'\[gaze\.search\] query="" visible=4 total=4 biometric_refused=0', s, 30)

        s = len(t())
        q.key('slash')
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', s, 30)
        s = len(t())
        send_text('2026-05')
        g['wait'](r'\[gaze\.search\] query="2026-05" visible=2 total=4 biometric_refused=0', s, 30)
        search_colours = screenshot('gaze-smart-search')

        # Exit & clear search via Esc
        s = len(t())
        q.key('esc')
        g['wait'](r'\[gaze\.search\] query="" visible=4 total=4 biometric_refused=0', s, 30)

        # 4. Non-Destructive Edits (Crop 16:9 + Exposure +0.50 EV + Shared Batik Warm Vintage + Stack Sharpen)
        s = len(t())
        click_control(f'photo_idx_{paris_idx}')
        g['wait'](rf'\[gaze\.selected\] idx={paris_idx} name=paris\.jpg', s, 30)

        esc_paris = re.escape(paris_path)
        s = len(t())
        q.key('e')
        g['wait'](rf'\[gaze\.edit\.opened\] path={esc_paris}', s, 30)
        edit_frame_pos = len(t())
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=edit', s, 30)

        # 16:9 crop ('w')
        s = len(t())
        q.key('w')
        g['wait'](rf'\[gaze\.edit\.updated\] path={esc_paris} crop=1000x\d+ ev_x100=0 filter=none stack=0', s, 30)

        # +0.50 EV exposure (`ev_inc` twice -> +25, +50)
        for expected_ev in (25, 50):
            s = len(t())
            click_control('ev_inc', edit_frame_pos)
            g['wait'](rf'\[gaze\.edit\.updated\] path={esc_paris} crop=1000x\d+ ev_x100={expected_ev} ', s, 30)

        # Shared Batik Warm Vintage preset (click control `preset_warm_vintage`)
        s = len(t())
        click_control('preset_warm_vintage', edit_frame_pos)
        g['wait'](rf'\[gaze\.edit\.updated\] path={esc_paris} crop=1000x\d+ ev_x100=50 filter=warm_vintage stack=0', s, 30)

        # Stack shared Batik Sharpen filter node (`stack_add_sharpen`)
        s = len(t())
        click_control('stack_add_sharpen', edit_frame_pos)
        g['wait'](rf'\[gaze\.edit\.updated\] path={esc_paris} crop=1000x\d+ ev_x100=50 filter=warm_vintage stack=1', s, 30)
        g['wait'](r'\[gaze\.persisted\] bytes=\d+ photos=4 albums=1', s, 30)

        edits_colours = screenshot('gaze-nondestructive-edits')

        # Export flattened copy to /data/gaze/exports/paris-edited.png
        s = len(t())
        q.key('s')
        m_exp = g['wait'](r'\[gaze\.export\.saved\] path=(/data/gaze/exports/paris-edited\.png) bytes=(\d+)', s, 60)
        exported_bytes = int(m_exp[2])
        assert exported_bytes > 1000, f'exported PNG too small: {exported_bytes}'

        # 5. Close Gaze, verify original photos/paris.jpg is untouched, and relaunch to prove catalog persistence!
        s = len(t())
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ', s, 90)

        # Check via Spindle `ls photos` that paris.jpg still has exact original byte count!
        spindle('ls photos', rf'\[spindle\.fs\.line\] paris\.jpg\s+{len(PARIS_BYTES)}\b', 60)

        # Relaunch Gaze Library on @photos -> albums=1 persisted & Edited collection has 1 photo!
        s = len(t())
        spindle('run gaze --library @photos', r'\[gaze\.library\.ready\] photos=4 albums=1 mode=library')
        ready2 = g['wait'](r'\[view\.ready\] target=(\d+) task=(\d+)', s, 60)
        launch2 = g['wait'](rf'\[shell\.program\.launch\] target={ready2[1]} task={ready2[2]} surface=(\d+)', s, 60)
        surface2 = int(launch2[1])
        tile2, before2 = maximize(g, surface2)
        g['wait'](r'\[gaze\.frame\] n=\d+ mode=library', before2, 60)
        time.sleep(0.5)

        ox, oy = tile2[0] + 8, tile2[1] + 36
        s = len(t())
        click_control('nav_edited', before2)
        g['wait'](r'\[gaze\.album\.selected\] name=Edited visible=1', s, 30)

        s = len(t())
        g['focus'](surface2)
        time.sleep(0.3)
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={ready2[1]} task={ready2[2]} ', s, 90)

        assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[view.fatal]' not in t()
        print({
            'ok': True,
            'photos': 4,
            'albums': 1,
            'exported_png_bytes': exported_bytes,
            'paris_original_bytes': len(PARIS_BYTES),
            'paris_sha256': hashlib.sha256(PARIS_BYTES).hexdigest()[:16],
            'colours': {
                'albums': albums_colours,
                'smart_search': search_colours,
                'nondestructive_edits': edits_colours,
            },
        })
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
