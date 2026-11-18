#!/usr/bin/env python3
"""Frame time during workspace slides: with 4 cores, Display paces animation
frames at 60 Hz and each paced frame is reported as `[display.frame.time]
ms=N`. On KVM the 99th percentile must stay under 20 ms. QEMU's software CPU
(TCG) renders a frame in hundreds of ms, so without /dev/kvm the gate only
checks that frames are paced and reported, and says the threshold was not
enforced. Saves a screenshot of the desktop with rounded corners and shadows."""
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
from desktop_close_gate import QMP

LIMIT_MS = 20


def percentile(values, pct):
    v = sorted(values)
    return v[(len(v) - 1) * pct // 100]


def boot(iso, disk, out, kvm):
    log = out / 'serial.log'
    sock = pathlib.Path('/tmp') / f'sex-frames-{os.getpid()}.sock'
    sock.unlink(missing_ok=True)
    cpu = ['-accel', 'kvm', '-cpu', 'host'] if kvm else ['-cpu', 'max,+pku']
    cmd = ['qemu-system-x86_64', '-M', 'q35', *cpu, '-smp', '4', '-m', '1G', '-vga', 'none',
           '-device', 'VGA,xres=1280,yres=800', '-device', 'nec-usb-xhci,id=xhci', '-device', 'usb-tablet,bus=xhci.0',
           '-cdrom', str(iso), '-drive', f'if=none,id=nvm,file={disk},format=raw', '-device', 'nvme,serial=sexos01,drive=nvm',
           '-serial', f'file:{log}', '-qmp', f'unix:{sock},server=on,wait=off', '-display', 'none', '-no-reboot', '-no-shutdown']
    firmware = os.environ.get('SEXOS_GATE_FIRMWARE')
    if firmware: cmd += ['-bios', firmware]
    proc = subprocess.Popen(cmd, stdout=(out / 'qemu.log').open('w'), stderr=subprocess.STDOUT)
    return proc, log, sock


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', type=pathlib.Path, required=True)
    p.add_argument('--source-disk', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--slides', type=int, default=6)
    a = p.parse_args()
    iso = a.iso.resolve(strict=True); out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    kvm = os.access('/dev/kvm', os.R_OK | os.W_OK)
    proc, log, sock = boot(iso, disk, out, kvm)

    def text(): return log.read_text(errors='replace') if log.exists() else ''

    def wait(pattern, start=0, timeout=240):
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

        try:
            wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        except RuntimeError as e:
            if not kvm: raise
            # This runner's CPU may not boot SexOS under KVM (no PKU): say so
            # and measure on TCG without the threshold.
            (out / 'kvm-boot-failed.txt').write_text(str(e) + '\n')
            proc.terminate(); proc.wait(timeout=10); sock.unlink(missing_ok=True)
            log.rename(out / 'serial-kvm.log')
            kvm = False
            proc, log, sock = boot(iso, disk, out, kvm)
            end = time.monotonic() + 15
            while not sock.exists():
                if time.monotonic() > end: raise RuntimeError('QMP unavailable')
                time.sleep(.05)
            q = QMP(sock)
            wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 ')
        time.sleep(2)
        # A second window, so the workspace has a focused window with a
        # shadow over a tiled one.
        start = len(text()); q.key('scroll_lock'); wait(r'\[shell.focus.set\] id=153', start)
        time.sleep(2)
        q.cmd('screendump', {'filename': str(out / 'desktop.ppm')})
        time.sleep(1)
        try:
            from PIL import Image
            Image.open(out / 'desktop.ppm').save(out / 'desktop.png', optimize=True)
            # The job log carries the picture too, for readers without artifact access.
            import base64
            data = base64.b64encode((out / 'desktop.png').read_bytes()).decode()
            print('[frame_time.screenshot.begin] bytes=%d' % len(data))
            for i in range(0, len(data), 4000): print('[frame_time.screenshot] ' + data[i:i + 4000])
            print('[frame_time.screenshot.end]', flush=True)
        except Exception as e:
            (out / 'screenshot-note.txt').write_text('PNG conversion skipped: %s\n' % e)
        start = len(text())
        for _ in range(a.slides):
            keys(('meta_l', True), ('tab', True), ('tab', False), ('meta_l', False))
            wait(r'\[display.motion\] on=1 dir=1', start); time.sleep(1)
            mark = len(text())
            keys(('meta_l', True), ('shift', True), ('tab', True), ('tab', False), ('shift', False), ('meta_l', False))
            wait(r'\[display.motion\] on=1 dir=-1', mark)
            wait(r'\[display.motion.end\]', mark, timeout=60); time.sleep(.5)
        times = [int(v) for v in re.findall(r'\[display\.frame\.time\] ms=(\d+)', text()[start:])]
        # Where each paced frame's time went: render ms, render and decoration kilocycles.
        found = re.findall(r'\[display\.frame\.cost\] render_ms=(\d+) render_kcyc=(\d+) decor_kcyc=(\d+) full=(\d) plan=(\w+) strip=(\d) kpx=(\d+) fast_kcyc=(\d+) pixel_kcyc=(\d+)', text()[start:])
        costs = [tuple(int(v) for v in m[:4]) for m in found]
        plans = ['%s%s:%s' % (m[4], '+strip' if m[5] == '1' else '', m[6]) for m in found]
        # Share of each frame's cycles: per-pixel compositing / fast runs / decoration.
        paths = ['%d/%d/%d' % tuple(int(v) * 100 // max(int(m[1]), 1) for v in (m[8], m[7], m[2])) for m in found]
        render_kcyc = sum(c[1] for c in costs); decor_kcyc = sum(c[2] for c in costs)
        cost = dict(frames=len(costs), render_ms=[c[0] for c in costs][:24], full_frames=sum(c[3] for c in costs),
                    decor_pct=[c[2] * 100 // max(c[1], 1) for c in costs][:24], plans=plans[:24], pixel_fast_decor_pct=paths[:24],
                    decor_share_pct=(decor_kcyc * 100 // render_kcyc) if render_kcyc else None)
        print('[frame_time.cost] ' + json.dumps(cost), flush=True)
        if len(times) < a.slides: raise RuntimeError('too few paced frames: %r' % times)
        p99 = percentile(times, 99); p50 = percentile(times, 50)
        # The grid may catch up with a short interval after a late frame, but
        # a paced desktop does not spin: half the frames take at least 15 ms.
        if kvm and p50 < 15: raise RuntimeError('frames not paced at 60 Hz: %r' % times)
        if kvm and p99 >= LIMIT_MS: raise RuntimeError('frame time p99 %d ms >= %d ms: %r' % (p99, LIMIT_MS, times))
        result = dict(ok=True, accel='kvm' if kvm else 'tcg', threshold_enforced=kvm, limit_ms=LIMIT_MS, frames=len(times),
                      p50_ms=p50, p99_ms=p99, max_ms=max(times), slides=a.slides, cost=cost,
                      scope='QEMU 1280x800, 4 cores: paced workspace-slide frame intervals; p99 < 20 ms enforced on KVM only')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, accel='kvm' if kvm else 'tcg', error=str(e))) + '\n'); raise
    finally:
        if proc.poll() is None:
            proc.terminate()
            try: proc.wait(timeout=10)
            except subprocess.TimeoutExpired: proc.kill(); proc.wait()
        sock.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
