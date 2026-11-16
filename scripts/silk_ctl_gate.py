#!/usr/bin/env python3
"""Desktop scripting (slice S4) on a booted desktop: the `silk` command in
Spindle moves a window to workspace 3.

The calculator is run from Spindle. `silk windows` lists it (its frame= is
read from Spindle's `[spindle.silk.line]` receipts, matched by the surface
the Shell launched it on). `silk focus FRAME` focuses it, then Spindle gets
the keyboard back. `silk move FRAME 3` sends it to workspace 3: the Shell's
own scene move (`[silk.frame.scene.move.done] ... to=2`) and `silk`'s receipt
show it on scene 2, and Spindle prints `ok move ... to=3`. `silk windows`
then lists it on ws=3, and `silk events` prints the focus and move events the
Shell saw. Every step waits for the Shell's and Spindle's receipts."""
import argparse, json, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
DONE = r'\[spindle.silk.done\] tag=(\d+) lines=(\d+) ok=([01])'
LINE = r'\[spindle.silk.line\] tag={} n=\d+ text=([^\r\n]*)'


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
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f: f.truncate(1024 ** 3)
    install(disk, [('abacus', (ROOT / 'iso_root/programs/sex-calc').read_bytes())])
    g = boot(a.iso, disk, out, 'silk', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def shot(name):
        time.sleep(1.5); q.cmd('screendump', {'filename': str(out / f'{name}.ppm')})

    def typed(line):
        """Types a line into Spindle and presses Enter; returns where the log stood."""
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in line:
            q.key({' ': 'spc', '-': 'minus'}.get(k, k)); time.sleep(.08)
        q.key('ret')
        return s

    def silk(words, timeout=30):
        """`silk WORDS` in Spindle: its answer's lines and whether it ended ok."""
        s = typed('silk ' + words)
        req = step(r'\[spindle.silk.request\] kind=\w+ tag=(\d+) sent=1', s, timeout)
        tag = req[1]
        done = wait(DONE.replace(r'tag=(\d+)', f'tag={tag}'), s, timeout); steps.append(done[0])
        block = t()[s:]
        lines = re.findall(LINE.format(tag), block)
        if len(lines) != int(done[1]): raise RuntimeError(f'silk {words}: {len(lines)} lines, done says {done[1]}')
        return s, lines, done[2] == '1'

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        s = typed('run abacus')
        ready = wait(r'\[calc.ready\] target=(\d+) task=(\d+)', s, 120)
        sid = int(wait(rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)', s, 60)[1])
        wait(rf'\[display.app.pixels\] target={ready[1]} ', s, 60); time.sleep(2)

        # silk windows: the calculator is listed, on a workspace from 1 to 9.
        s, lines, ok = silk('windows')
        if not ok or not lines[-1].startswith('ok windows='): raise RuntimeError(f'silk windows did not end ok: {lines}')
        mine = [l for l in lines if re.match(rf'frame=\d+ sid={sid} ', l)]
        if len(mine) != 1: raise RuntimeError(f'silk windows does not list surface {sid} once: {lines}')
        frame = int(re.match(r'frame=(\d+) ', mine[0])[1])
        before = int(re.search(r' ws=(\d) ', mine[0])[1])
        if before == 3: raise RuntimeError('the calculator opened on workspace 3 already: ' + mine[0])
        step(rf'\[shell.silk.window\] frame={frame} sid={sid} ws={before} ', s)
        shot('01-listed')

        # silk focus FRAME, then the keyboard back to Spindle.
        s, lines, ok = silk(f'focus {frame}')
        step(rf'\[shell.silk.focus\] frame={frame} sid={sid} ws={before} focus={sid} ok=1', s)
        if not ok or lines != [f'ok focus frame={frame} sid={sid} ws={before}']: raise RuntimeError(f'silk focus: {lines}')

        # silk move FRAME 3: the Shell's own move to scene 2.
        s, lines, ok = silk(f'move {frame} 3')
        moved = step(rf'\[silk.frame.scene.move.done\] frame={frame} from={before - 1} to=2 ownership_unique=1 focus_valid=\d ok=1', s)
        step(rf'\[shell.silk.move\] frame={frame} sid={sid} from={before} to=3 scene=2 ok=1', s)
        if not ok or lines != [f'ok move frame={frame} sid={sid} from={before} to=3']: raise RuntimeError(f'silk move: {lines}')
        shot('02-moved')

        # silk windows again: on workspace 3 now.
        s, lines, ok = silk('windows')
        mine = [l for l in lines if l.startswith(f'frame={frame} sid={sid} ')]
        if not ok or len(mine) != 1 or ' ws=3 ' not in mine[0]: raise RuntimeError(f'silk windows after the move: {lines}')
        step(rf'\[shell.silk.window\] frame={frame} sid={sid} ws=3 ', s)

        # silk events: the focus and the move, in that order.
        s, lines, ok = silk('events')
        if not ok or not re.match(r'ok events=\d+ last=\d+$', lines[-1]): raise RuntimeError(f'silk events did not end ok: {lines}')
        events = lines[:-1]
        focus_at = [i for i, l in enumerate(events) if re.match(rf'event seq=\d+ kind=focused frame={frame} sid={sid}$', l)]
        move_at = [i for i, l in enumerate(events) if re.match(rf'event seq=\d+ kind=moved frame={frame} sid={sid} from={before} to=3$', l)]
        if not focus_at or len(move_at) != 1 or focus_at[0] > move_at[0]: raise RuntimeError(f'silk events lacks the focus then the move: {events}')
        seqs = [int(re.match(r'event seq=(\d+) ', l)[1]) for l in events]
        if seqs != sorted(seqs) or len(set(seqs)) != len(seqs): raise RuntimeError(f'event numbers out of order: {seqs}')
        step(rf'\[shell.silk.event\] event seq=\d+ kind=moved frame={frame} sid={sid} from={before} to=3', 0)
        step(rf'\[shell.silk.events\] pd=\d+ since=0 events={len(events)} ', s)

        text = t()
        for bad in ('KERNEL PANIC', 'panicked', '[calc.fatal]', '[shell.silk.reject]', '[shell.silk.out.full]'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, surface=sid, frame=frame, from_workspace=before, scene_move=moved[0], events=events, steps=steps,
                      scope=f'QEMU {a.resolution}: Spindle\'s silk command lists the calculator, focuses it, moves it to workspace 3 '
                            '(the Shell\'s scene 2), lists it there and prints the focus and move events')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
