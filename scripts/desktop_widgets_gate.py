#!/usr/bin/env python3
"""Desktop widgets (slice S5b) on a booted desktop: Super+Shift+W puts the
clock and system cards on the wallpaper, and again takes them off.

On an empty workspace (Super+2), Super+Shift+W: the Shell's `[shell.widgets]
on=1 ok=1`, Display's `[display.widgets] cards=3` and `[display.widgets.draw]`
with the card column, the time (the guest clock runs on UTC, the default zone,
so it matches the host's UTC time to a minute or two), the date, and once two
samples are in, the processor load. The screen shows the cards: the column
differs from the bare wallpaper and holds light text. Super+Shift+W again
hides them and the column is wallpaper again."""
import argparse, datetime, json, re, shutil, time
from pathlib import Path
from quil_autosave_gate import boot

DRAW = r'\[display.widgets.draw\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) time="([^"]*)" date="([^"]*)" cpu="([^"]*)" free="([^"]*)"'


def ppm(path):
    data = path.read_bytes()
    fields, at = [], 0
    while len(fields) < 4:
        while data[at:at + 1].isspace(): at += 1
        end = at
        while not data[end:end + 1].isspace(): end += 1
        fields.append(data[at:end]); at = end
    w, h = int(fields[1]), int(fields[2])
    return w, h, data[at + 1:]


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(x for x in out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f: f.truncate(1024 ** 3)
    g = boot(a.iso, disk, out, 'widgets', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def chord(*keys):
        q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': True, 'key': {'type': 'qcode', 'data': k}}} for k in keys]})
        time.sleep(.1)
        q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': False, 'key': {'type': 'qcode', 'data': k}}} for k in reversed(keys)]})
        time.sleep(.4)

    def shot(name):
        time.sleep(1.5); path = out / f'{name}.ppm'; q.cmd('screendump', {'filename': str(path)}); time.sleep(.5); return ppm(path)

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        # An empty workspace: nothing covers the wallpaper.
        s = len(t()); chord('meta_l', '2')
        step(r'\[shell.wm.action\] action=Workspace\(1\) done=1', s)
        bw, bh, bare = shot('bare')

        s = len(t()); chord('meta_l', 'shift', 'w')
        step(r'\[shell.widgets\] on=1 ok=1', s)
        step(r'\[display.widgets\] cards=3', s)
        first = step(DRAW, s)
        x, y, w, h = (int(v) for v in first.groups()[:4])
        if not (x + w <= bw and y + h <= bh and w >= 200 and h >= 150): raise RuntimeError('odd card column: ' + first[0])
        hhmm = first[5]
        now = datetime.datetime.now(datetime.timezone.utc)
        near = {(now + datetime.timedelta(minutes=d)).strftime('%H:%M') for d in range(-3, 4)}
        if hhmm not in near: raise RuntimeError(f'the clock card says {hhmm}, the host says {now:%H:%M} UTC')
        day = now.strftime('%A ') + str(now.day) + now.strftime(' %B')
        if first[6] != day and now.hour not in (0, 23): raise RuntimeError(f'the clock card says {first[6]!r}, not {day!r}')
        if not re.fullmatch(r'Memory free \d+(\.\d)? [MG]B', first[8]): raise RuntimeError('memory line: ' + first[8])
        # A second sample gives the processor load.
        loaded = step(DRAW.replace(r'cpu="([^"]*)"', r'cpu="(Processor \d+%)"'), s, 60)
        sw, sh, shown = shot('widgets')
        changed = bright = 0
        for yy in range(y, y + h):
            for xx in range(x, x + w):
                i = (yy * sw + xx) * 3
                if shown[i:i + 3] != bare[i:i + 3]: changed += 1
                if min(shown[i:i + 3]) > 0xD0: bright += 1
        if changed < w * h // 2: raise RuntimeError(f'the cards look missing: {changed} of {w * h} pixels changed')
        if bright < 150: raise RuntimeError(f'no text on the cards: {bright} light pixels')

        s = len(t()); chord('meta_l', 'shift', 'w')
        step(r'\[shell.widgets\] on=0 ok=1', s)
        step(r'\[display.widgets\] cards=0', s)
        _, _, gone = shot('gone')
        left = sum(1 for yy in range(y, y + h) for xx in range(x, x + w)
                   if gone[(yy * sw + xx) * 3:(yy * sw + xx) * 3 + 3] != bare[(yy * sw + xx) * 3:(yy * sw + xx) * 3 + 3])
        if left > w * h // 50: raise RuntimeError(f'the cards stayed: {left} pixels still differ from the wallpaper')

        text = t()
        for bad in ('KERNEL PANIC', 'panicked'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, column=[x, y, w, h], time=hhmm, date=first[6], cpu=loaded[7], free=loaded[8],
                      changed=changed, light=bright, steps=steps,
                      scope=f'QEMU {a.resolution}: Super+Shift+W shows the clock and system cards on the wallpaper of an empty '
                            'workspace (time matching the host, processor load and free memory) and hides them again')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
