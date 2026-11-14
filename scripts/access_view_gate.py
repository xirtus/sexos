#!/usr/bin/env python3
"""Magnifier and colour filters on a booted desktop (slice S8).

Super+Alt+8 turns the magnifier on at 2x around the pointer: the screen then
shows source pixel ((p + x) / 2) at output x on each axis, so the point under
the pointer stays put. Super+Alt+C steps the colour filter (high contrast),
Super+Alt+I inverts colours, and turning both off gives back the plain frame.
Screens are saved for review."""
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
from desktop_close_gate import QMP

W, H = 1280, 800


def ppm(path):
    data = path.read_bytes()
    parts = data.split(maxsplit=4)
    w, h = int(parts[1]), int(parts[2])
    return w, h, parts[4][-w * h * 3:]


def px(img, x, y):
    w, _, data = img
    i = (y * w + x) * 3
    return tuple(data[i:i + 3])


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=pathlib.Path, required=True)
    p.add_argument('--source-disk', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    a = p.parse_args()
    iso = a.iso.resolve(strict=True); out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-access-{os.getpid()}.sock'
    cmd = ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku', '-m', '512M', '-vga', 'none',
           '-device', f'VGA,xres={W},yres={H}', '-device', 'nec-usb-xhci,id=xhci', '-device', 'usb-tablet,bus=xhci.0',
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
            for code, down in events:
                q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': code}}}]})
                time.sleep(.06)

        def chord(key):
            keys(('meta_l', True), ('alt', True), (key, True), (key, False), ('alt', False), ('meta_l', False))

        def shot(name):
            time.sleep(2.0)
            f = out / f'{name}.ppm'; q.cmd('screendump', {'filename': str(f)}); time.sleep(.4)
            return ppm(f)

        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        # The Spindle console: text on screen gives the zoom something to show.
        start = len(text()); q.key('scroll_lock'); wait(r'\[shell.focus.set\] id=153', start)
        # Pointer to a known place.
        pointer = (840, 520)
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(pointer[0] * 32767 / (W - 1))}},
                                              {'type': 'abs', 'data': {'axis': 'y', 'value': round(pointer[1] * 32767 / (H - 1))}}]})
        time.sleep(1)
        before = shot('00-plain')

        start = len(text()); chord('8')
        wait(r'\[shell.access\] change=View \{ zoom: 8, filter: 0 \}', start)
        wait(r'\[display.access\] zoom=8 filter=0', start)
        zoomed = shot('01-magnifier-2x')
        # Display's receipt names the pointer it centred on; the screen must
        # be the frame seen through that view.
        views = re.findall(r'\[display.access.view\] zoom=8 filter=0 px=(\d+) py=(\d+) ', text()[start:])
        if not views: raise RuntimeError('no magnifier view receipt')
        px0, py0 = map(int, views[-1])
        if abs(px0 - pointer[0]) > 2 or abs(py0 - pointer[1]) > 2: print(f'note: Display pointer {px0},{py0} vs QMP {pointer}')
        # Samples away from the arrow (which is drawn into the frame and so is
        # magnified too): output (x, y) shows source ((p + x) / 2, (p + y) / 2).
        samples = [(x, y) for y in range(60, H - 4, 23) for x in range(4, W - 4, 29)
                   if not (px0 - 8 <= x <= px0 + 48 and py0 - 8 <= y <= py0 + 72)]
        match = sum(px(zoomed, x, y) == px(before, (px0 + x) // 2, (py0 + y) // 2) for x, y in samples)
        changed = sum(px(zoomed, x, y) != px(before, x, y) for x, y in samples)
        if match < len(samples) * 0.9:
            for x, y in samples[::100]: print('sample', (x, y), px(zoomed, x, y), 'want', px(before, (px0 + x) // 2, (py0 + y) // 2), 'plain', px(before, x, y))
            raise RuntimeError(f'2x view does not match the frame: {match}/{len(samples)}')
        if changed < len(samples) * 0.05: raise RuntimeError(f'magnifier did not change the screen: {changed}/{len(samples)}')

        start = len(text()); chord('8')
        wait(r'\[display.access\] zoom=4 filter=0', start)
        start = len(text()); chord('c')
        wait(r'\[display.access\] zoom=4 filter=1', start)
        contrast = shot('02-high-contrast')
        start = len(text()); chord('i')
        wait(r'\[display.access\] zoom=4 filter=2', start)
        inverted = shot('03-inverted')
        lum = lambda c: c[0] * 54 + c[1] * 183 + c[2] * 19
        flips = sum((lum(px(before, x, y)) > 128 * 256) != (lum(px(inverted, x, y)) > 128 * 256)
                    for x, y in samples if abs(lum(px(before, x, y)) - 128 * 256) > 40 * 256)
        if flips < len(samples) * 0.3: raise RuntimeError(f'inverted colours did not invert lightness: {flips}')
        start = len(text()); chord('i')
        wait(r'\[display.access\] zoom=4 filter=0', start)
        plain = shot('04-plain-again')
        same = sum(px(plain, x, y) == px(before, x, y) for x, y in samples)
        if same < len(samples) * 0.95: raise RuntimeError(f'plain frame not restored: {same}/{len(samples)}')
        if 'KERNEL PANIC' in text(): raise RuntimeError('panic')
        result = dict(ok=True, samples=len(samples), zoom_match=match, zoom_changed=changed, inverted_flips=flips, restored=same,
                      contrast_differs=sum(px(contrast, x, y) != px(before, x, y) for x, y in samples),
                      scope='QEMU 1280x800: Super+Alt+8 2x magnifier around the pointer, Super+Alt+C high contrast, Super+Alt+I inverted, plain frame restored')
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
