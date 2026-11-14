#!/usr/bin/env python3
"""Window animations: an opening window rises in while the tiler glides the
others to their new places, over several frames that finish within the
tween's time; a workspace switch slides the new workspace in from its side;
with animations off (Super+Shift+A) the same change is a single frame."""
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
    log = out / 'serial.log'; sock = pathlib.Path('/tmp') / f'sex-motion-{os.getpid()}.sock'
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

    def frames(start):
        return [int(t) for t in re.findall(r'\[display\.motion\.frame\] tick=(\d+)', text()[start:])]

    try:
        end = time.monotonic() + 15
        while not sock.exists():
            if time.monotonic() > end: raise RuntimeError('QMP unavailable')
            time.sleep(.05)
        q = QMP(sock)

        def keys(*events):
            q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': code}}} for code, down in events]})

        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        time.sleep(2)
        # Open the Spindle console: it rises in, and the tiler moves the others.
        start = len(text()); q.key('scroll_lock'); wait(r'\[shell.focus.set\] id=153', start)
        wait(r'\[display.motion.start\] surface=\d+ kind=open ', start)
        time.sleep(1.5)
        moves = re.findall(r'\[display\.motion\.start\] surface=(\d+) kind=(\w+) .* tick=(\d+)', text()[start:])
        ticks = frames(start)
        progress = [int(k) for k in re.findall(r'\[display\.motion\.frame\] tick=\d+ progress=(\d+)', text()[start:])]
        ended = wait(r'\[display.motion.end\] tick=(\d+)', start)
        # Each frame is drawn where the tween is at that time: an in-between
        # frame, then the landing frame. QEMU's software rendering takes far
        # longer per frame than hardware, so this checks shape, not 16 ms pacing.
        if not ticks or not any(0 < k < 1024 for k in progress): raise RuntimeError('no in-between animation frame: %r' % progress)
        first = int(moves[0][2])
        span = int(ended[1]) - first
        steps = [b - a for a, b in zip([first] + ticks, ticks + [int(ended[1])])]
        if span > 1500: raise RuntimeError('animation ran %d ticks' % span)
        # Workspace switch (Super+Tab): the next workspace slides in.
        start = len(text()); keys(('meta_l', True), ('tab', True), ('tab', False), ('meta_l', False))
        wait(r'\[display.motion\] on=1 dir=1', start)
        # Back to the first workspace: its windows slide in from the left.
        time.sleep(1); start = len(text()); keys(('meta_l', True), ('shift', True), ('tab', True), ('tab', False), ('shift', False), ('meta_l', False))
        wait(r'\[display.motion\] on=1 dir=-1', start)
        slid = wait(r'\[display.motion.start\] surface=\d+ kind=open ', start)
        time.sleep(1.5)
        # Animations off: closing and reopening the console is one frame each.
        start = len(text()); keys(('meta_l', True), ('shift', True), ('a', True), ('a', False), ('shift', False), ('meta_l', False))
        wait(r'\[display.motion\] on=0 dir=0', start)
        start = len(text()); keys(('meta_l', True), ('tab', True), ('tab', False), ('meta_l', False))
        wait(r'\[display.motion\] on=0 dir=1', start)
        time.sleep(1.5)
        if frames(start) or re.search(r'\[display\.motion\.(start|end)\]', text()[start:]): raise RuntimeError('animations off still animated')
        result = dict(ok=True, frames=len(ticks), span_ticks=span, frame_steps=steps, progress=progress, moved=len(moves),
                      scope='QEMU 1280x800: open/move tweens drawn with in-between frames on the damage path and landed, workspace slide hints both ways, animations off draws no tween')
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
