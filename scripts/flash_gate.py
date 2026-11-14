#!/usr/bin/env python3
"""Flash Camera QEMU gate test.

Validates the SexOS Flash UVC capture application in guest QEMU:
1. Spawns FakeUvcCamera on host loopback TCP (accessible via QEMU user net at 10.0.2.2:PORT).
2. Installs the flash binary into the NVMe disk program store.
3. Boots QEMU with SexOS ISO and NIC enabled.
4. Executes `run +net flash 10.0.2.2:PORT --photo --record-frames 15 --frames 20`.
5. Asserts serial outputs:
   - [flash.ready]
   - [flash.uvc.negotiated] width=640 height=480 format=Yuy2
   - [flash.frame] n=...
   - [flash.photo] path=/data/flash/photo-0001.png bytes=...
   - [flash.video] path=/data/flash/video-0001.webm bytes=... frames=... (or [flash.video.unavailable])
   - [flash.done] total_frames=20
"""
import argparse
import os
import re
import shutil
import sys
import time
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
        ROOT / 'iso_root/programs/flash',
        ROOT / 'target/x86_64-sex-app/release/flash',
        ROOT / 'target/x86_64-sex-app/debug/flash',
    ]
    for p in candidates:
        if p.is_file():
            elf = p.read_bytes()
            if elf.startswith(b'\x7fELF') and int.from_bytes(elf[16:18], 'little') == 3:
                return elf
    raise FileNotFoundError("PIE Flash binary (ET_DYN) not found. Run: bash scripts/flash_host_gate.sh")


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--binding', type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--require-video', action='store_true')
    a = p.parse_args()

    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024 ** 3)

    elf = find_flash_binary()
    install(disk, [('flash', elf)])

    camera = FakeUvcCamera(host='127.0.0.1', port=0, width=640, height=480, fps=30)
    print(f"[flash_gate] FakeUvcCamera listening on 127.0.0.1:{camera.port}")

    g = boot(a.iso, disk, a.out, 'flash', a.resolution)
    t = g['text']
    q = g['q']

    def keys(text):
        for k in text:
            if k in SHIFTED or k.isupper():
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': SHIFTED.get(k, k.lower())}], 'hold-time': 80})
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

        cmd_line = f'run +net flash 10.0.2.2:{camera.port} --photo --record-frames 15 --frames 20'
        print(f"[flash_gate] Sending command: {cmd_line}")
        keys(cmd_line)
        q.key('ret')

        # 1. Wait for shell program launch
        m = g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', s, 120)
        print(f"[flash_gate] Program launched: target={m[1]} task={m[2]} surface={m[3]}")

        # 2. Assert [flash.ready]
        g['wait'](r'\[flash\.ready\]', s, 60)
        print("[flash_gate] Flash ready received")

        # 3. Assert [flash.uvc.source] kind=tcp and [flash.uvc.negotiated]
        g['wait'](r'\[flash\.uvc\.source\] kind=tcp', s, 60)
        g['wait'](r'\[flash\.uvc\.negotiated\] width=640 height=480 format=Yuy2', s, 60)
        print("[flash_gate] UVC negotiated over TCP (640x480 YUY2)")

        # 4. Assert [flash.photo]
        m_photo = g['wait'](r'\[flash\.photo\] path=/data/flash/photo-0001\.png bytes=(\d+)', s, 60)
        assert int(m_photo[1]) > 100, f"expected valid PNG bytes, got {m_photo[1]}"
        print(f"[flash_gate] Photo captured: bytes={m_photo[1]}")

        # 5. Assert video (strictly required when --require-video is passed on recording-profile CI)
        if a.require_video:
            m_video = g['wait'](r'\[flash\.video\] path=/data/flash/video-0001\.webm bytes=(\d+) frames=(\d+)', s, 120)
            assert int(m_video[1]) > 100 and int(m_video[2]) == 15, f"expected non-empty WebM video, got {m_video[0]}"
            print(f"[flash_gate] Splicy WebM video captured: bytes={m_video[1]} frames={m_video[2]}")
        else:
            m_video = g['wait'](r'\[flash\.(?:video\] path=/data/flash/video-0001\.webm bytes=(\d+) frames=(\d+)|video\.unavailable)', s, 120)
            print(f"[flash_gate] Video outcome: {m_video[0]}")

        # 6. Assert [flash.done]
        g['wait'](r'\[flash\.done\] total_frames=20', s, 120)
        print("[flash_gate] Flash finished all 20 frames")

        # 7. Wait for app retirement
        g['wait'](rf'\[app\.private\.retired\] target={m[1]} task={m[2]} ', s, 60)
        print("[flash_gate] Flash process retired cleanly")

        # Check for kernel panics and real TCP camera frame delivery
        assert 'KERNEL PANIC' not in t() and 'panicked' not in t(), "Kernel panic detected"
        assert camera.frames_sent >= 20, f"Expected >=20 frames from host FakeUvcCamera, got {camera.frames_sent}"
        print(f"[flash_gate] PASS (frames sent by fake camera: {camera.frames_sent})")

    finally:
        try:
            g['stop']()
        except Exception:
            pass
        camera.stop()


if __name__ == '__main__':
    main()
