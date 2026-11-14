#!/usr/bin/env python3
"""Screenshot UI: Print dims the desktop and shows a hint, a drag picks a
region, a click picks a window, Enter the whole screen and Escape cancels;
Shift+Print takes the whole screen at once. Each picture is saved by the first
Spindle terminal as the next free /Screenshot-N.png at the picked size."""
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
from desktop_close_gate import QMP


def ppm(path):
    data = path.read_bytes()
    parts = data.split(maxsplit=4)
    w, h = int(parts[1]), int(parts[2])
    return w, h, parts[4][-w * h * 3:]


def mean(px, w, x0, y0, x1, y1):
    total = n = 0
    for y in range(y0, y1, 4):
        for x in range(x0, x1, 4):
            i = (y * w + x) * 3
            total += px[i] + px[i + 1] + px[i + 2]; n += 3
    return total / n


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=pathlib.Path, required=True)
    p.add_argument('--source-disk', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    a = p.parse_args()
    iso = a.iso.resolve(strict=True); out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-shot-{os.getpid()}.sock'
    cmd = ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku', '-m', '512M', '-vga', 'none',
           '-device', 'VGA,xres=1280,yres=800', '-device', 'nec-usb-xhci,id=xhci', '-device', 'usb-tablet,bus=xhci.0',
           '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={disk},format=raw', '-device', 'nvme,serial=sexos01,drive=nvm',
           '-serial', f'file:{log}', '-qmp', f'unix:{sock},server=on,wait=off', '-display', 'none', '-no-reboot', '-no-shutdown']
    firmware = os.environ.get('SEXOS_GATE_FIRMWARE')
    if firmware: cmd += ['-bios', firmware]
    proc = subprocess.Popen(cmd, stdout=(out / 'qemu.log').open('w'), stderr=subprocess.STDOUT)

    def text(): return log.read_text(errors='replace') if log.exists() else ''

    def wait(pattern, start=0, timeout=180):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if proc.poll() is not None: raise RuntimeError('guest exited')
            value = text(); m = re.search(pattern, value[start:])
            if m: return m
            if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at', value): raise RuntimeError('guest fault')
            time.sleep(.05)
        raise RuntimeError('timeout: ' + pattern)

    try:
        end = time.monotonic() + 15
        while not sock.exists():
            if time.monotonic() > end: raise RuntimeError('QMP unavailable')
            time.sleep(.05)
        q = QMP(sock)

        def keys(*events):
            q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': code}}} for code, down in events]})

        def move(x, y):
            q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / 1279)}}, {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / 799)}}]})
            time.sleep(.3)

        def button(down):
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]}); time.sleep(.3)

        def saved(n, start):
            m = wait(r'\[spindle\.screenshot\.saved\] name=Screenshot-%d\.png w=(\d+) h=(\d+) bytes=(\d+)' % n, start, 240)
            return int(m[1]), int(m[2]), int(m[3])

        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        # A window to click: the Spindle console.
        start = len(text()); q.key('scroll_lock'); wait(r'\[shell.focus.set\] id=153', start)
        time.sleep(1.5)
        move(1200, 700); time.sleep(1)
        base = out / 'before.ppm'; q.cmd('screendump', {'filename': str(base)})

        # Print: the selection shows over a dimmed desktop.
        start = len(text()); keys(('print', True), ('print', False))
        wait(r'\[shell.screenshot.ui\] show=1 ', start)
        wait(r'\[display.screenshot\] show=1 x=0 y=0 w=0 h=0', start)
        wait(r'\[display.screenshot.draw\] ', start)
        # Drag a region; the drawn selection follows the pointer.
        move(200, 150); button(True); move(520, 390)
        wait(r'\[display.screenshot\] show=1 x=(?:19|20)\d y=1[45]\d w=3[12]\d h=2[34]\d', start)
        time.sleep(1.5)
        sel = out / 'selecting.ppm'; q.cmd('screendump', {'filename': str(sel)})
        w, h, before = ppm(base); _, _, during = ppm(sel)
        outside = (mean(before, w, 700, 450, 1100, 650), mean(during, w, 700, 450, 1100, 650))
        inside = (mean(before, w, 220, 170, 500, 370), mean(during, w, 220, 170, 500, 370))
        if not outside[1] < outside[0] * 0.8: raise RuntimeError('desktop outside the selection is not dimmed: %r' % (outside,))
        if not abs(inside[1] - inside[0]) < max(8, inside[0] * 0.1): raise RuntimeError('selection is not left clear: %r' % (inside,))
        start = len(text()); button(False)
        take = wait(r'\[shell.screenshot.take\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) full=0 status=0x0', start)
        wait(r'\[display.screenshot\] show=0 ', start)
        wait(r'\[spindle.screenshot.take\] x=%s y=%s w=%s h=%s full=0' % take.groups(), start)
        region = saved(1, start)
        if region[:2] != (int(take[3]), int(take[4])): raise RuntimeError('saved region size differs: %r vs %r' % (region, take.groups()))
        time.sleep(1.5)
        q.cmd('screendump', {'filename': str(out / 'saved-notice.ppm')})

        # Shift+Print: the whole screen at once, the next free name.
        start = len(text()); keys(('shift', True), ('print', True), ('print', False), ('shift', False))
        wait(r'\[shell.screenshot.take\] x=0 y=0 w=0 h=0 full=1 status=0x0', start)
        full = saved(2, start)
        if full[:2] != (1280, 800): raise RuntimeError('full screenshot size %r' % (full,))

        # Print, then a click on a window: that window's rectangle.
        start = len(text()); keys(('print', True), ('print', False))
        wait(r'\[display.screenshot\] show=1 ', start)
        move(640, 450); button(True); button(False)
        win = wait(r'\[shell.screenshot.window\] sid=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+)', start)
        window = saved(3, start)
        if window[:2] != (int(win[4]), int(win[5])): raise RuntimeError('window screenshot size %r vs %r' % (window, win.groups()))

        # Print, Enter: the whole screen.
        start = len(text()); keys(('print', True), ('print', False))
        wait(r'\[display.screenshot\] show=1 ', start)
        keys(('ret', True), ('ret', False))
        wait(r'\[shell.screenshot.take\] x=0 y=0 w=0 h=0 full=1', start)
        saved(4, start)

        # Print, Escape: nothing is taken.
        start = len(text()); keys(('print', True), ('print', False))
        wait(r'\[display.screenshot\] show=1 ', start)
        keys(('esc', True), ('esc', False))
        wait(r'\[shell.screenshot.cancel\]', start)
        wait(r'\[display.screenshot\] show=0 ', start)
        time.sleep(2)
        if re.search(r'\[shell.screenshot.take\]', text()[start:]): raise RuntimeError('Escape took a screenshot')

        result = dict(ok=True, region=list(region), full=list(full), window=list(window), window_sid=int(win[1]),
                      dim_outside=outside, clear_inside=inside,
                      scope='QEMU 1280x800: Print selection drawn and dimmed, drag region / click window / Enter / Shift+Print / Escape, PNGs saved as /Screenshot-1..4.png at the picked sizes')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e))) + '\n'); raise
    finally:
        if proc.poll() is None:
            proc.terminate()
            try: proc.wait(timeout=10)
            except subprocess.TimeoutExpired: proc.kill(); proc.wait()
        sock.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
