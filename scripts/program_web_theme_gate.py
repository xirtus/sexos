#!/usr/bin/env python3
"""Kaleidoscope chrome follows Settings without changing the site's pixels."""
import argparse
import shutil
import time
from pathlib import Path

from PIL import Image

from install_program import install
from quil_autosave_gate import boot
from settings_persist_gate import first_start
from web_surface_geometry import canvas_box, latest_frame, latest_paint

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--iso', required=True, type=Path)
    parser.add_argument('--source-disk', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--resolution', default='1280x800')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    disk = args.out / 'nvme.img'
    shutil.copyfile(args.source_disk, disk)
    install(disk, [(name, (ROOT / f'iso_root/programs/{name}').read_bytes())
                   for name in ('kaleidoscope', 'tatami')])
    guest = boot(args.iso, disk, args.out, 'web-theme', args.resolution)
    q, text = guest['q'], guest['text']

    def run(name):
        guest['focus'](153)
        time.sleep(.5)
        start = len(text())
        for key in f'run {name}':
            q.key('spc' if key == ' ' else key)
            time.sleep(.05)
        q.key('ret')
        return start

    def screenshot(name, surface):
        time.sleep(1)
        path = args.out / f'web-{name}.ppm'
        q.cmd('screendump', {'filename': str(path)})
        tile = latest_frame(text(), surface)
        w, h = latest_paint(text())
        x, y, _, _ = canvas_box(ROOT, tile, w, h)
        image = Image.open(path).convert('RGB')
        return image.getpixel((x + 2, y + 2))

    try:
        guest['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        start = run('kaleidoscope')
        ready = guest['wait'](r'\[web.ready\] target=(\d+) task=(\d+)', start, 240)
        guest['wait'](r'\[web.theme\] mode=Light generation=\d+', start, 60)
        launch = guest['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)', start, 60)
        surface = int(launch[1])
        guest['wait'](rf'\[display.app.pixels\] target={ready[1]} surface={surface} w=\d+ h=\d+', start, 60)
        light = screenshot('light', surface)
        assert light == (255, 255, 255), ('light browser bar', light)

        start = run('tatami')
        first_start(guest, q, text, start)
        guest['wait'](r'\[settings.open\] generation=\d+', start, 120)
        for key in 'theme':
            q.key(key)
            time.sleep(.1)
        start = len(text())
        q.key('right')
        guest['wait'](r'\[settings.set\] key=24 value=1 status=0', start, 60)
        guest['wait'](r'\[web.theme\] mode=Dark generation=\d+', start, 60)
        q.key('f11')
        guest['focus'](surface)
        dark = screenshot('dark', surface)
        assert dark == (38, 35, 43), ('dark browser bar', dark)
        output = text()
        assert 'KERNEL PANIC' not in output and '[web.fatal]' not in output
        print({'ok': True, 'light': light, 'dark': dark})
    finally:
        guest['stop']()


if __name__ == '__main__':
    main()
