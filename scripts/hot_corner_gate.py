#!/usr/bin/env python3
"""Hot corners (slice S5) on a booted desktop: the top-left corner opens the
window overview.

The calculator is run from Spindle so the overview has two windows. The
pointer goes to the middle of the screen, then to the top-left pixel: the
Shell's `[shell.hot_corner] corner=top_left action=overview done=1` and the
overview's own `[shell.overview.open]` show it opened, and the screen shows
the overview's cards. Staying in the corner does nothing more. Away and back
again closes it (the corner toggles, as Super+W does). The top-right corner
does nothing by default."""
import argparse, json, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--binding', type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(x for x in out.iterdir()): raise RuntimeError('fresh output required')
    W, H = (int(v) for v in a.resolution.split('x'))
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f: f.truncate(1024 ** 3)
    install(disk, [('abacus', (ROOT / 'iso_root/programs/abacus').read_bytes())])
    g = boot(a.iso, disk, out, 'corner', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def point(x, y):
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (W - 1))}},
                                              {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (H - 1))}}]})
        time.sleep(.6)

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in 'run abacus':
            q.key({' ': 'spc'}.get(k, k)); time.sleep(.08)
        q.key('ret')
        ready = wait(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 300)
        wait(rf'\[display.app.pixels\] target={ready[1]} ', s, 60); time.sleep(2)

        # Middle of the screen first: the corner arms once the pointer has been away.
        point(W // 2, H // 2); point(W // 3, H // 3)
        s = len(t()); point(0, 0)
        step(r'\[shell.hot_corner\] corner=top_left action=overview done=1', s)
        opened = step(r'\[shell.overview.open\] windows=(\d+)', s)
        if int(opened[1]) < 2: raise RuntimeError('the overview opened without both windows: ' + opened[0])
        cards = re.findall(r'\[shell.overview.card\] index=(\d+) sid=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+)', t()[s:])
        if len(cards) != int(opened[1]): raise RuntimeError(f'{len(cards)} overview cards for {opened[1]} windows')
        step(r'\[display.switcher.draw\] count=%s selected=\d+ grid=1 ' % opened[1], s)
        time.sleep(1.5); q.cmd('screendump', {'filename': str(out / 'overview.ppm')})

        # Staying in the corner does nothing more.
        s = len(t()); point(0, 0); time.sleep(1)
        if re.search(r'\[shell.hot_corner\]', t()[s:]): raise RuntimeError('the corner fired again without the pointer leaving')

        # Away and back: the corner closes the overview, focus where it was.
        point(200, 200); s = len(t()); point(0, 0)
        step(r'\[shell.hot_corner\] corner=top_left action=overview done=1', s)
        step(r'\[shell.overview.close\] keep=0 focus=\d+', s)

        # The top-right corner does nothing by default.
        point(W // 2, H // 2); s = len(t()); point(W - 1, 0)
        step(r'\[shell.hot_corner\] corner=top_right action=none done=0', s)
        if re.search(r'\[shell.overview.open\]', t()[s:]): raise RuntimeError('the top-right corner opened the overview')

        text = t()
        for bad in ('KERNEL PANIC', 'panicked', '[abacus.fatal]'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, windows=int(opened[1]), cards=len(cards), steps=steps,
                      scope=f'QEMU {a.resolution}: pushing the pointer into the top-left corner opens the window overview '
                            '(once, until the pointer leaves), again closes it; the top-right corner does nothing by default')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
