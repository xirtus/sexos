#!/usr/bin/env python3
"""Bluetooth panel (Cufflinks W2b): Super+Shift+B opens the Shell's Bluetooth panel,
Display draws it under the bar at the right, and with no controller (QEMU has no
Bluetooth controller model) it says "No Bluetooth controller"; Escape closes it.

Also checks the wiring the prompt needs: BOOT pairs the panel region between the
Shell and Cufflinks, both sides find it, and Cufflinks still sleeps. Pairing itself
(the comparison dialog, Pair, Forget, the policy) is host-tested in bt_host_gate.sh."""
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
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-bt-panel-{os.getpid()}.sock'
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
            if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|\[cufflinks\.panic\]', value): raise RuntimeError('guest fault')
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
            time.sleep(.1)

        def shot(name):
            path = out / (name + '.ppm'); q.cmd('screendump', {'filename': str(path)}); return ppm(path)

        wait(r'\[boot\.bt\.ui\] shell=3 cufflinks=52 slot=1007 notify=1008 runtime_mint=0')
        wait(r'\[cufflinks\.ui\.route\] ok=1 slot=1007')
        wait(r'\[cufflinks\.idle\] controller=none')
        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        wait(r'\[shell\.bt\.route\] ok=1 slot=1007', 0, 30)
        time.sleep(2)
        w, h, before = shot('before')

        # Super+Shift+B opens the panel.
        start = len(text())
        keys(('meta_l', True), ('shift', True), ('b', True), ('b', False), ('shift', False), ('meta_l', False))
        panel = wait(r'\[shell\.bt\.panel\] open=1 phase=(\d) lines=(\d+) selected=-1 dialog=0 status="([^"]*)"', start, 30)
        if panel[1] != '1' or panel[3] != 'No Bluetooth controller':
            raise RuntimeError('panel says %r (phase %s), not "No Bluetooth controller"' % (panel[3], panel[1]))
        wait(r'\[display\.bt\.panel\] lines=%s selected=-1' % panel[2], start, 10)
        draw = wait(r'\[display\.bt\.panel\.draw\] lines=%s panel=(\d+)x(\d+)\+(\d+)\+(\d+)' % panel[2], start, 10)
        pw, ph, px0, py0 = map(int, draw.groups())
        time.sleep(1.5)
        w, h, after = shot('open')
        # Inside the panel the pixels are the panel's dark background with light text;
        # count changed pixels against the desktop before, and bright text pixels.
        changed = bright = 0
        for y in range(py0, min(py0 + ph, h)):
            for x in range(px0, min(px0 + pw, w)):
                i = (y * w + x) * 3
                if before[i:i + 3] != after[i:i + 3]: changed += 1
                if after[i] > 0xB0 and after[i + 1] > 0xB0 and after[i + 2] > 0xB0: bright += 1
        if changed < pw * ph // 3: raise RuntimeError('panel not drawn (%d of %d pixels changed)' % (changed, pw * ph))
        if bright < 60: raise RuntimeError('panel text missing (%d bright pixels)' % bright)

        # Keys stay with the panel while it shows; Escape closes it.
        start = len(text())
        keys(('esc', True), ('esc', False))
        wait(r'\[shell\.bt\.panel\] open=0 ', start, 10)
        wait(r'\[display\.bt\.panel\] lines=0 ', start, 10)
        time.sleep(1.5)
        w, h, closed = shot('closed')
        back = sum(1 for y in range(py0, min(py0 + ph, h)) for x in range(px0, min(px0 + pw, w))
                   if closed[(y * w + x) * 3:(y * w + x) * 3 + 3] == before[(y * w + x) * 3:(y * w + x) * 3 + 3])
        if back < pw * ph * 9 // 10: raise RuntimeError('panel still drawn after Escape (%d of %d pixels back)' % (back, pw * ph))
        if '[cufflinks.panic]' in text(): raise RuntimeError('Cufflinks panicked')

        result = dict(ok=True, status=panel[3], lines=int(panel[2]), panel=[px0, py0, pw, ph], changed=changed, bright=bright,
                      scope='QEMU 1280x800, no Bluetooth controller: Super+Shift+B panel drawn by Display saying "No Bluetooth controller"; Escape closes it')
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
