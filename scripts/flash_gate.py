#!/usr/bin/env python3
"""Flash Photo Booth QEMU Gate Test.

Validates the SexOS Flash UVC Photo Booth application in guest QEMU:
1. Boots guest with FakeUvcCamera (10.0.2.2:PORT) and disk.
2. Launches Flash with library grant: `run +net flash 10.0.2.2:PORT @/data/flash/library`.
3. Verifies UVC negotiation ([flash.uvc.negotiated] width=640 height=480).
4. Toggles 3x3 Live Effects Grid matrix ('e' key), verifies [flash.filter] mode=grid,
   and captures screenshot `flash-effects-grid.png`.
5. Selects live filter ('6' -> PopArt), verifies [flash.filter] mode=live filter=PopArt.
6. Triggers 3s Timer Shot ('t' key), verifies countdown 3 -> 2 -> 1 and [flash.photo] mode=timer.
7. Triggers 4-Shot Burst ('b' key), verifies [flash.burst] count=4 contact=burst-0001.png,
   and captures screenshot `flash-burst-library.png` showing the bottom filmstrip.
8. Opens photo library in Gaze ('g' key), verifies [flash.gaze.launched], [view.ready],
   and [view.slide] index=1 of=N, steps to slide 2 ('n' key), and captures `flash-gaze-library.png`.
9. Closes Gaze and Flash cleanly, verifying clean retirement and 0 kernel faults.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))

from flash_fakes import FakeUvcCamera
from install_program import install

os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND', '600')

from quil_autosave_gate import boot

KEYS = {' ': 'spc', '.': 'dot', '-': 'minus', '/': 'slash'}
SHIFTED = {'+': 'equal', '@': '2', ':': 'semicolon', '_': 'minus'}


def find_flash_binary():
    candidates = [
        ROOT / 'target/x86_64-sex-app/release/flash',
        ROOT / 'iso_root/programs/flash',
        ROOT / 'target/x86_64-sex-app/debug/flash',
    ]
    for p in candidates:
        if p.is_file():
            elf = p.read_bytes()
            if elf.startswith(b'\x7fELF') and int.from_bytes(elf[16:18], 'little') == 3:
                return elf
    raise FileNotFoundError("PIE Flash binary (ET_DYN) not found. Run: bash scripts/flash_host_gate.sh")


def find_gaze_binary(iso_path: Path, out_dir: Path) -> bytes:
    for p in (ROOT / 'iso_root/programs/gaze', ROOT / 'target/x86_64-sex-app/release/view'):
        if p.is_file():
            return p.read_bytes()
    extracted = out_dir / 'gaze.elf'
    extracted.unlink(missing_ok=True)
    subprocess.run(
        ['xorriso', '-osirrox', 'on', '-indev', str(iso_path), '-extract', '/programs/gaze', str(extracted)],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return extracted.read_bytes()


def ppm_to_png(ppm_path: Path, png_path: Path):
    """Convert Netpbm P6 PPM to standard PNG format."""
    if shutil.which('sips'):
        res = subprocess.run(
            ['sips', '-s', 'format', 'png', str(ppm_path), '--out', str(png_path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if res.returncode == 0 and png_path.is_file():
            return

    # Pure Python P6 -> PNG converter
    with open(ppm_path, 'rb') as f:
        magic = f.readline().strip()
        if magic != b'P6':
            raise ValueError(f"Not a P6 PPM: {magic!r}")
        line = f.readline()
        while line.startswith(b'#'):
            line = f.readline()
        w, h = map(int, line.split())
        f.readline()  # maxval
        raw_rgb = f.read(w * h * 3)

    raw_lines = bytearray()
    for y in range(h):
        raw_lines.append(0)  # Filter byte 0 (None)
        raw_lines.extend(raw_rgb[y * w * 3 : (y + 1) * w * 3])

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack('>I', len(data))
            + tag
            + data
            + struct.pack('>I', zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    png = bytearray(b'\x89PNG\r\n\x1a\n')
    png.extend(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)))  # 8-bit RGB
    png.extend(chunk(b'IDAT', zlib.compress(bytes(raw_lines), 6)))
    png.extend(chunk(b'IEND', b''))

    with open(png_path, 'wb') as f:
        f.write(png)


def capture_screenshot(q, out_dir: Path, filename: str, screenshots_dir: Path = None):
    png_path = out_dir / filename
    ppm_path = out_dir / f"{png_path.stem}.ppm"
    q.cmd('screendump', {'filename': str(ppm_path)})
    time.sleep(0.3)
    if ppm_path.is_file():
        ppm_to_png(ppm_path, png_path)
        ppm_path.unlink(missing_ok=True)
        print(f"[flash_gate] Captured screenshot: {png_path.name} ({png_path.stat().st_size} bytes)")
        if screenshots_dir is not None:
            screenshots_dir.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(png_path, screenshots_dir / png_path.name)
            print(f"[flash_gate] Copied to screenshots-dir: {screenshots_dir / png_path.name}")
        # Copy to docs/user/assets if directory exists
        assets_dir = ROOT / 'docs/user/assets'
        if assets_dir.is_dir():
            shutil.copyfile(png_path, assets_dir / png_path.name)
            print(f"[flash_gate] Copied to docs/user/assets/{png_path.name}")
    else:
        print(f"[flash_gate] Warning: screendump did not produce {ppm_path}")


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--binding', type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--screenshots-dir', type=Path, default=None)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--require-video', action='store_true')
    a = p.parse_args()

    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024 ** 3)

    elf = find_flash_binary()
    gaze_elf = find_gaze_binary(a.iso, a.out)
    install(disk, [('flash', elf), ('gaze', gaze_elf)])

    camera = FakeUvcCamera(host='127.0.0.1', port=0, width=640, height=480, fps=30)
    print(f"[flash_gate] FakeUvcCamera listening on 127.0.0.1:{camera.port}")

    g = boot(a.iso, disk, a.out, 'flash', a.resolution)
    t = g['text']
    q = g['q']

    def keys(text):
        for k in text:
            if k in SHIFTED or k.isupper():
                q.cmd(
                    'send-key',
                    {
                        'keys': [
                            {'type': 'qcode', 'data': 'shift'},
                            {'type': 'qcode', 'data': SHIFTED.get(k, k.lower())},
                        ],
                        'hold-time': 80,
                    },
                )
            else:
                q.key(KEYS.get(k, k))
            time.sleep(0.05)

    try:
        # Wait for desktop / shell ready
        g['wait'](r'\[app\.private\.retired\] target=41 ', 0, 120)
        time.sleep(2)

        # Focus Spindle shell (surface 153)
        g['focus'](153)
        time.sleep(0.5)
        s = len(t())

        # Launch Flash with photo library grant @/data/flash/library
        cmd_line = f'run +net flash 10.0.2.2:{camera.port} @/data/flash/library'
        print(f"[flash_gate] Launching Flash: {cmd_line}")
        keys(cmd_line)
        q.key('ret')

        # 1. Wait for shell program launch
        m_launch = g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', s, 120)
        flash_target = int(m_launch[1])
        flash_task = int(m_launch[2])
        flash_surface = int(m_launch[3])
        print(f"[flash_gate] Flash launched: target={flash_target} task={flash_task} surface={flash_surface}")

        # 2. Assert [flash.ready]
        g['wait'](r'\[flash\.ready\]', s, 60)
        print("[flash_gate] Flash ready received")

        # 3. Assert UVC negotiation over TCP (640x480 YUY2)
        g['wait'](r'\[flash\.uvc\.source\] kind=tcp', s, 60)
        g['wait'](r'\[flash\.uvc\.negotiated\] width=640 height=480 format=Yuy2', s, 60)
        print("[flash_gate] UVC negotiated over TCP (640x480 YUY2)")

        # Focus Flash window
        g['focus'](flash_surface)
        time.sleep(1.0)

        # 4. Toggle 3x3 Live Effects Grid ('e' key)
        print("[flash_gate] Toggling 3x3 Live Effects Grid ('e')...")
        q.key('e')
        g['wait'](r'\[flash\.filter\] mode=grid', s, 30)
        time.sleep(1.5)
        capture_screenshot(q, a.out, 'flash-effects-grid.png', a.screenshots_dir)

        # 5. Select Live Filter ('6' -> PopArt)
        print("[flash_gate] Selecting PopArt filter ('6')...")
        q.key('6')
        g['wait'](r'\[flash\.filter\] mode=live filter=PopArt', s, 30)
        time.sleep(0.5)

        # 6. Trigger 3-Second Timer Shot ('t' key)
        print("[flash_gate] Triggering 3s Countdown Timer Shot ('t')...")
        q.key('t')
        g['wait'](r'\[flash\.timer\] countdown=3', s, 30)
        g['wait'](r'\[flash\.timer\] countdown=2', s, 30)
        g['wait'](r'\[flash\.timer\] countdown=1', s, 30)
        m_timer_photo = g['wait'](
            r'\[flash\.photo\] mode=timer filter=PopArt path=(/data/flash/library/photo-\d+\.png) bytes=(\d+)',
            s,
            60,
        )
        print(f"[flash_gate] Timer photo captured: path={m_timer_photo[1]} bytes={m_timer_photo[2]}")

        # 7. Trigger 4-Shot Burst ('b' key)
        print("[flash_gate] Triggering 4-Shot Burst ('b')...")
        q.key('b')
        m_burst = g['wait'](
            r'\[flash\.burst\] count=4 contact=(/data/flash/library/burst-\d+\.png) bytes=(\d+)',
            s,
            60,
        )
        print(f"[flash_gate] 4-Shot burst + contact sheet captured: contact={m_burst[1]} bytes={m_burst[2]}")
        time.sleep(1.0)
        capture_screenshot(q, a.out, 'flash-burst-library.png', a.screenshots_dir)

        # 8. Open Library in Gaze ('g' key)
        print("[flash_gate] Opening photo library in Gaze ('g')...")
        s_open = len(t())
        q.key('g')
        g['wait'](r'\[flash\.gaze\.launched\] path=/data/flash/library', s_open, 30)
        m_gaze_launch = g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', s_open, 60)
        gaze_surface = int(m_gaze_launch[3])
        m_gaze = g['wait'](r'\[view\.ready\] target=(\d+) task=(\d+)', s_open, 60)
        gaze_target = int(m_gaze[1])
        gaze_task = int(m_gaze[2])
        print(f"[flash_gate] Gaze launched: target={gaze_target} task={gaze_task} surface={gaze_surface}")

        # Wait for first slide
        m_slide1 = g['wait'](r'\[view\.slide\] index=1 of=(\d+) name=(.+)', s_open, 30)
        print(f"[flash_gate] Gaze slide 1 displayed: of={m_slide1[1]} name={m_slide1[2]}")
        g['focus'](gaze_surface)
        time.sleep(1.0)

        # Step to next photo in Gaze ('n' key)
        print("[flash_gate] Advancing to next photo in Gaze ('n')...")
        s_next = len(t())
        q.key('n')
        m_slide2 = g['wait'](r'\[view\.slide\] index=2 of=(\d+) name=(.+)', s_next, 30)
        print(f"[flash_gate] Gaze slide 2 displayed: of={m_slide2[1]} name={m_slide2[2]}")
        time.sleep(1.0)
        capture_screenshot(q, a.out, 'flash-gaze-library.png', a.screenshots_dir)

        # Close Gaze ('f11' key)
        print("[flash_gate] Closing Gaze ('f11')...")
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={gaze_target} task={gaze_task} ', s_open, 60)
        print("[flash_gate] Gaze retired cleanly")

        # Refocus Flash window and close ('Escape')
        g['focus'](flash_surface)
        time.sleep(0.5)
        print("[flash_gate] Closing Flash ('Escape')...")
        q.key('esc')
        g['wait'](r'\[flash\.done\]', s, 60)
        g['wait'](rf'\[app\.private\.retired\] target={flash_target} task={flash_task} ', s, 60)
        print("[flash_gate] Flash retired cleanly")

        # 9. Verify Gaze directly via `run gaze @/data/flash/library` from Spindle shell
        print("[flash_gate] Testing direct Gaze launch: run gaze @/data/flash/library...")
        g['focus'](153)
        time.sleep(0.5)
        s_gaze = len(t())
        keys('run gaze @/data/flash/library')
        q.key('ret')
        m_gaze2_launch = g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', s_gaze, 60)
        gaze2_surface = int(m_gaze2_launch[3])
        m_gaze2 = g['wait'](r'\[view\.ready\] target=(\d+) task=(\d+)', s_gaze, 60)
        gaze2_target = int(m_gaze2[1])
        gaze2_task = int(m_gaze2[2])
        print(f"[flash_gate] Direct Gaze launched: target={gaze2_target} task={gaze2_task} surface={gaze2_surface}")

        m_slide_direct = g['wait'](r'\[view\.slide\] index=1 of=(\d+) name=(.+)', s_gaze, 30)
        print(f"[flash_gate] Direct Gaze slide displayed: of={m_slide_direct[1]} name={m_slide_direct[2]}")
        g['focus'](gaze2_surface)
        time.sleep(0.5)

        print("[flash_gate] Closing direct Gaze ('f11')...")
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={gaze2_target} task={gaze2_task} ', s_gaze, 60)
        print("[flash_gate] Direct Gaze retired cleanly")

        # Assert no kernel panics or faults
        assert 'KERNEL PANIC' not in t() and 'panicked' not in t(), "Kernel panic detected"
        assert camera.frames_sent >= 10, f"Expected >=10 frames sent by camera, got {camera.frames_sent}"
        print(f"[flash_gate] PASS (frames sent by fake camera: {camera.frames_sent})")

    finally:
        try:
            g['stop']()
        except Exception:
            pass
        camera.stop()


if __name__ == '__main__':
    main()
