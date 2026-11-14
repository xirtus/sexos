#!/usr/bin/env python3
"""Clipboard history (Super+V): copy two texts in Quil, Super+V brings the
earlier one back as the clipboard (named in the panel) and Ctrl+V pastes it;
Super+V again soon after steps further back."""
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
from desktop_close_gate import QMP


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=pathlib.Path, required=True)
    p.add_argument('--source-disk', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    a = p.parse_args()
    iso = a.iso.resolve(strict=True); out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-cliphist-{os.getpid()}.sock'
    cmd = ['qemu-system-x86_64', '-M', 'q35', '-cpu', 'max,+pku', '-m', '512M', '-vga', 'none',
           '-device', 'VGA,xres=1280,yres=800', '-device', 'nec-usb-xhci,id=xhci', '-device', 'usb-tablet,bus=xhci.0',
           '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={disk},format=raw', '-device', 'nvme,serial=sexos01,drive=nvm',
           '-serial', f'file:{log}', '-qmp', f'unix:{sock},server=on,wait=off', '-display', 'none', '-no-reboot', '-no-shutdown']
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
            time.sleep(.15)

        def chord(*names):
            keys(*[(n, True) for n in names]); keys(*[(n, False) for n in reversed(names)])

        def typed(word):
            for ch in word: chord(ch)

        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        time.sleep(1)
        # Focus Quil (surface 201) from the overview.
        start = len(text()); chord('meta_l', 'w')
        wait(r'\[shell.overview.open\]', start)
        card = wait(r'\[shell.overview.card\] index=\d+ sid=201 x=(\d+) y=(\d+) w=(\d+) h=(\d+)', start)
        x, y, w, h = map(int, card.groups())
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round((x + w // 2) * 32767 / 1279)}}, {'type': 'abs', 'data': {'axis': 'y', 'value': round((y + h // 2) * 32767 / 799)}}]})
        time.sleep(.5)
        start = len(text())
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': True, 'button': 'left'}}]}); time.sleep(.2)
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': False, 'button': 'left'}}]})
        wait(r'\[shell.overview.close\] keep=1 focus=201', start)
        # Copy "alpha", then "beta".
        start = len(text()); chord('ctrl', 'n'); wait(r'\[quil.new.ok\] bytes=0', start)
        start = len(text()); typed('alpha'); wait(r'\[quil.document.edit\] revision=\d+ pending=0 bytes=5', start)
        start = len(text()); chord('ctrl', 'a'); wait(r'\[quil.selection\] ', start)
        start = len(text()); chord('ctrl', 'c'); wait(r'\[quil.clipboard.published\] ticket=\d+ bytes=5 ', start)
        start = len(text()); typed('beta'); wait(r'\[quil.document.edit\] revision=\d+ pending=0 bytes=4', start)
        start = len(text()); chord('ctrl', 'a'); wait(r'\[quil.selection\] ', start)
        start = len(text()); chord('ctrl', 'c'); wait(r'\[quil.clipboard.published\] ticket=\d+ bytes=4 ', start)
        # Super+V: "alpha" is the clipboard again and the panel names it.
        start = len(text()); chord('meta_l', 'v')
        back = wait(r'\[shell.clipboard.history\] index=0 total=1 generation=(\d+) len=5 ', start)
        time.sleep(1)
        q.cmd('screendump', {'filename': str(out / 'clipboard-history.ppm')})
        start = len(text()); chord('ctrl', 'a'); wait(r'\[quil.selection\] ', start)
        start = len(text()); chord('ctrl', 'v')
        wait(r'\[quil.clipboard.paste\] ticket=\d+ bytes=5 generation=%s hash=0x[0-9a-f]+ applied=1' % back[1], start)
        # Super+V again soon after: the one before, "beta".
        start = len(text()); chord('meta_l', 'v')
        wait(r'\[shell.clipboard.history\] index=0 total=1 generation=\d+ len=4 ', start)
        result = dict(ok=True, restored_generation=int(back[1]),
                      scope='QEMU 1280x800: two Quil copies, Super+V restores the earlier one, Ctrl+V pastes it, Super+V again steps back')
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
