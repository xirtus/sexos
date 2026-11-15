#!/usr/bin/env python3
"""Panel tray: network, volume and battery icons render in the bar; clicking
one opens quick settings, whose slider sets Bell's master volume, whose mute
toggles, whose keyboard row switches layout; a click outside closes it. The
clock opens a month calendar (checked against Python's calendar) that steps
months; Escape closes it."""
import argparse
import calendar
import datetime
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


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=pathlib.Path, required=True)
    p.add_argument('--source-disk', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    a = p.parse_args()
    iso = a.iso.resolve(strict=True); out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-tray-{os.getpid()}.sock'
    cmd = ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku', '-m', '512M', '-vga', 'none',
           '-device', 'VGA,xres=1280,yres=800', '-device', 'nec-usb-xhci,id=xhci', '-device', 'usb-tablet,bus=xhci.0',
           '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={disk},format=raw', '-device', 'nvme,serial=sexos01,drive=nvm',
           '-rtc', 'base=utc', '-serial', f'file:{log}', '-qmp', f'unix:{sock},server=on,wait=off', '-display', 'none', '-no-reboot', '-no-shutdown']
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

        def click(x, y):
            q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / 1279)}}, {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / 799)}}]})
            time.sleep(.4)
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': True, 'button': 'left'}}]}); time.sleep(.15)
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': False, 'button': 'left'}}]}); time.sleep(.3)

        def shot(name):
            path = out / (name + '.ppm'); q.cmd('screendump', {'filename': str(path)}); return ppm(path)

        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        # The tray looks once boot settles and the Shell next wakes (any input);
        # Bell answers with the volume.
        for i in range(60):
            if re.search(r'\[display.tray\] menu=0 ', text()): break
            q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': 20000 + i * 50}}, {'type': 'abs', 'data': {'axis': 'y', 'value': 20000}}]})
            time.sleep(1)
        wait(r'\[display.tray\] menu=0 ', 0, 10)
        wait(r'\[shell.tray\] menu=0 offset=0 volume=\d+ muted=\d .* status=0x0')
        # sexnet answers next (one reply at a time): Wi-Fi state, or none in QEMU.
        wait(r'\[shell.tray\] menu=0 offset=0 .* wifi=(?!255)\d+ ', 0, 30)
        time.sleep(1.5)
        # Icons: the three chips left of the bell (1280 wide, scale 1: x 1024..1096).
        w, h, px = shot('bar')
        fg = sum(1 for y in range(0, 28) for x in range(1024, 1096)
                 if px[(y * w + x) * 3] > 0xB0 and px[(y * w + x) * 3 + 2] > 0xD0)
        if fg < 40: raise RuntimeError('tray icons look missing (%d bright pixels)' % fg)

        # Volume icon: quick settings open on the status panel surface.
        start = len(text()); click(1060, 13)
        wait(r'\[shell.status.open.ok\]', start)
        geo = wait(r'\[shell.panel.geometry\] surface=147 x=(\d+) y=(\d+) w=(\d+) h=(\d+)', start)
        x0, y0, pw, ph = map(int, geo.groups())
        wait(r'\[display.tray.draw\] menu=1 ', start)
        time.sleep(1.5); shot('quick')
        # The slider: row 0 starts pad=12 in; the track is 84 in and 136 long.
        row_y = y0 + 12 + 20
        sx, sw = x0 + 12 + 84, pw - 24 - 84 - 64 - 12
        start = len(text()); click(sx + sw // 4, row_y)
        wait(r'\[shell.tray.click\] menu=quick hit=Volume\(2[3-7]\)', start)
        vol = wait(r'\[display.tray\] menu=1 offset=0 volume=(2[3-7]) muted=0', start)
        # Mute toggles on and off.
        mute_x = x0 + pw - 12 - 32
        start = len(text()); click(mute_x, row_y)
        wait(r'\[display.tray\] menu=1 offset=0 volume=%s muted=1' % vol[1], start)
        time.sleep(1); shot('quick-muted')
        start = len(text()); click(mute_x, row_y)
        wait(r'\[display.tray\] menu=1 offset=0 volume=%s muted=0' % vol[1], start)
        # Keyboard row switches layout.
        start = len(text()); click(x0 + 40, y0 + 12 + 3 * 40 + 20)
        wait(r'\[shell.tray.layout\] label=', start)
        # A click outside closes.
        start = len(text()); click(300, 500)
        wait(r'\[shell.status.close.ok\]', start)
        wait(r'\[display.tray\] menu=0 ', start)

        # Clock: the calendar for this month.
        start = len(text()); click(1200, 13)
        wait(r'\[shell.clock.open.ok\] id=0x94', start)
        cal = wait(r'\[display.tray.calendar\] year=(\d+) month=(\d+) first=(\d) days=(\d+) today=(\d+)', start)
        year, month, first, days, today = map(int, cal.groups())
        if (calendar.monthrange(year, month)[0], calendar.monthrange(year, month)[1]) != (first, days):
            raise RuntimeError('calendar grid wrong for %d-%d: %r' % (year, month, cal.groups()))
        now = datetime.datetime.now(datetime.timezone.utc)
        if (year, month) != (now.year, now.month) and now.day != 1: raise RuntimeError('calendar month is not today\'s')
        if not 1 <= today <= days: raise RuntimeError('today not marked')
        time.sleep(1.5); shot('calendar')
        cg = wait(r'\[shell.panel.geometry\] surface=148 x=(\d+) y=(\d+) w=(\d+) h=(\d+)', start)
        cx, cy, cw, ch = map(int, cg.groups())
        # Next month (the > button at the header's right), then back twice.
        start = len(text()); click(cx + cw - 12 - 16, cy + 12 + 16)
        nxt = wait(r'\[display.tray\] menu=2 offset=1 ', start)
        ny, nm = (year + month // 12, month % 12 + 1)
        n = wait(r'\[display.tray.calendar\] year=%d month=%d first=(\d) days=(\d+) today=0' % (ny, nm), start)
        if (int(n[1]), int(n[2])) != calendar.monthrange(ny, nm): raise RuntimeError('next month grid wrong')
        start = len(text()); click(cx + 12 + 16, cy + 12 + 16)
        wait(r'\[display.tray\] menu=2 offset=0 ', start)
        start = len(text()); click(cx + 12 + 16, cy + 12 + 16)
        wait(r'\[display.tray\] menu=2 offset=-1 ', start)
        # Escape closes.
        start = len(text()); keys(('esc', True), ('esc', False))
        wait(r'\[shell.clock.close.ok\] id=0x94', start)
        wait(r'\[display.tray\] menu=0 ', start)

        result = dict(ok=True, icon_pixels=fg, volume=int(vol[1]), calendar=[year, month, first, days, today],
                      scope='QEMU 1280x800: tray icons drawn; quick settings slider/mute/layout/close; clock calendar month grid, next/prev, Escape')
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
