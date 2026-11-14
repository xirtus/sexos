#!/usr/bin/env python3
"""Alt+Tab window switcher and Super+W window overview: held Alt shows Display's card row with live
thumbnails, Tab and Shift+Tab step it, letting go of Alt keeps the choice,
Escape goes back, and a quick Alt+Tab returns to the previous window
(most-recently-used order, as in KDE and GNOME)."""
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


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=pathlib.Path, required=True)
    p.add_argument('--source-disk', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    a = p.parse_args()
    iso = a.iso.resolve(strict=True); out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-switcher-{os.getpid()}.sock'
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

        def focus():
            return int(re.findall(r'\[shell.focus.set\] id=(\d+)', text())[-1])

        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        # A second window: the Spindle console.
        start = len(text()); q.key('scroll_lock'); wait(r'\[shell.focus.set\] id=153', start)
        time.sleep(1)
        first = focus()

        # Hold Alt, Tab twice: the switcher shows and steps; focus follows it.
        start = len(text()); keys(('alt', True), ('tab', True), ('tab', False))
        opened = wait(r'\[shell.switcher\] open=1 count=(\d+) selected=1 ', start)
        count = int(opened[1])
        if count < 2: raise RuntimeError('switcher needs two windows, saw %d' % count)
        drawn = wait(r'\[display.switcher.draw\] count=%d selected=1 grid=0 cols=\d+ panel=(\d+)x(\d+)\+(\d+)\+(\d+)' % count, start)
        second = focus()
        if second == first: raise RuntimeError('Alt+Tab did not move focus')
        time.sleep(1.5)
        shot = out / 'switcher-open.ppm'; q.cmd('screendump', {'filename': str(shot)})
        w, h, px = ppm(shot)
        pw, ph, x0, y0 = map(int, drawn.groups())
        # The panel holds live thumbnails: many distinct colours, not a flat fill.
        colours = {px[(y * w + x) * 3:(y * w + x) * 3 + 3] for y in range(y0, y0 + ph, 2) for x in range(x0, x0 + pw, 2)}
        if len(colours) < 24: raise RuntimeError('switcher thumbnails look empty (%d colours)' % len(colours))
        # Shift+Tab steps back to where it started, Tab forward again.
        start = len(text()); keys(('shift', True), ('tab', True), ('tab', False), ('shift', False))
        wait(r'\[shell.switcher\] open=1 count=%d selected=0 ' % count, start)
        start = len(text()); keys(('tab', True), ('tab', False))
        wait(r'\[shell.switcher\] open=1 count=%d selected=1 ' % count, start)
        # Letting go of Alt keeps the second window and hides the switcher.
        start = len(text()); keys(('alt', False))
        ended = wait(r'\[shell.switcher.end\] keep=1 focus=(\d+)', start)
        wait(r'\[display.switcher\] count=0 selected=0', start)
        if int(ended[1]) != second: raise RuntimeError('Alt release did not keep the selection')
        time.sleep(1.5)
        q.cmd('screendump', {'filename': str(out / 'switcher-closed.ppm')})
        # A quick Alt+Tab returns to the previous window (recent first).
        start = len(text()); keys(('alt', True), ('tab', True), ('tab', False), ('alt', False))
        back = wait(r'\[shell.switcher.end\] keep=1 focus=(\d+)', start)
        if int(back[1]) != first: raise RuntimeError('quick Alt+Tab did not return to the previous window')
        # Escape cancels: focus stays where it was.
        start = len(text()); keys(('alt', True), ('tab', True), ('tab', False), ('esc', True), ('esc', False), ('alt', False))
        cancel = wait(r'\[shell.switcher.end\] keep=0 focus=(\d+)', start)
        if int(cancel[1]) != first: raise RuntimeError('Escape did not return to the window it started from')
        # Overview (Super+W): every window as a live thumbnail in a grid.
        start = len(text()); keys(('meta_l', True), ('w', True), ('w', False), ('meta_l', False))
        wait(r'\[shell.overview.open\] windows=%d' % count, start)
        grid = wait(r'\[display.switcher.draw\] count=%d selected=0 grid=1 cols=(\d+) ' % count, start)
        cards = {int(i): (int(sid), int(x), int(y), int(cw), int(ch)) for i, sid, x, y, cw, ch in
                 re.findall(r'\[shell.overview.card\] index=(\d+) sid=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+)', text()[start:])}
        if len(cards) != count or cards[0][0] != focus(): raise RuntimeError('overview cards differ')
        time.sleep(1.5)
        oshot = out / 'overview-open.ppm'; q.cmd('screendump', {'filename': str(oshot)})
        w, h, px = ppm(oshot)
        colours = {px[(y * w + x) * 3:(y * w + x) * 3 + 3] for i in cards for y in range(cards[i][2], cards[i][2] + cards[i][4], 3) for x in range(cards[i][1], cards[i][1] + cards[i][3], 3)}
        if len(colours) < 24: raise RuntimeError('overview thumbnails look empty')
        # Right arrow selects the next card; Enter goes there.
        start = len(text()); keys(('right', True), ('right', False))
        wait(r'\[display.switcher\] count=%d selected=1 grid=1' % count, start)
        start = len(text()); keys(('ret', True), ('ret', False))
        picked = wait(r'\[shell.overview.close\] keep=1 focus=(\d+)', start)
        if int(picked[1]) != cards[1][0]: raise RuntimeError('Enter did not go to the selected window')
        # Click the last card.
        start = len(text()); keys(('meta_l', True), ('w', True), ('w', False), ('meta_l', False))
        wait(r'\[shell.overview.open\]', start)
        cards = {int(i): (int(sid), int(x), int(y), int(cw), int(ch)) for i, sid, x, y, cw, ch in
                 re.findall(r'\[shell.overview.card\] index=(\d+) sid=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+)', text()[start:])}
        last = cards[count - 1]
        cx, cy = last[1] + last[3] // 2, last[2] + last[4] // 2
        start = len(text())
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(cx * 32767 / 1279)}}, {'type': 'abs', 'data': {'axis': 'y', 'value': round(cy * 32767 / 799)}}]})
        time.sleep(.5)
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': True, 'button': 'left'}}]}); time.sleep(.2)
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': False, 'button': 'left'}}]})
        clicked = wait(r'\[shell.overview.close\] keep=1 focus=(\d+)', start)
        if int(clicked[1]) != last[0]: raise RuntimeError('click did not go to the clicked window')
        # Escape closes without moving focus.
        start = len(text()); keys(('meta_l', True), ('w', True), ('w', False), ('meta_l', False))
        wait(r'\[shell.overview.open\]', start)
        start = len(text()); keys(('right', True), ('right', False), ('esc', True), ('esc', False))
        esc = wait(r'\[shell.overview.close\] keep=0 focus=(\d+)', start)
        if int(esc[1]) != last[0]: raise RuntimeError('Escape moved focus')
        result = dict(ok=True, windows=count, first=first, second=second, panel=[pw, ph, x0, y0], overview_cols=int(grid[1]),
                      scope='QEMU 1280x800: Alt+Tab switcher open/step/back/keep/cancel and Super+W overview keys/Enter/click/Escape, with live-thumbnail pixels present')
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
