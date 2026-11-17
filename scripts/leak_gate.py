#!/usr/bin/env python3
"""Leak gate: open and close each program many times, and crash and restart each
restartable service, and the memory must come back.

Memory is read by Swatch (`spawn swatch 1`, apps/hum/src/bin/swatch.rs): the kernel heap
(bytes in use, live blocks), the machine's free frames, free program memory and the pages
program rows still hold. For each program: --warm open/close cycles first (caches fill,
the row is made once), a sample, --cycles more, another sample. A program leaks when the
second sample is worse than the first by more than the tolerance. With --services (an ISO
built with SEXOS_CRASH_PROBE=1) each restartable service is stopped as a fault would stop
it, the same way. Every result goes to leak.json; the gate fails at the end with the list
of leaks, so one run finds them all.

  python3 scripts/leak_gate.py --iso sexos-v1.0.0.iso --source-disk blank.img --out out/leak
"""
import argparse, json, os, re, shutil, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('SEXOS_GATE_BOUND', '5400')
from install_program import install  # noqa: E402
from quil_autosave_gate import boot  # noqa: E402

# The fifteen programs people open most; each must close with F11.
PROGRAMS = ['jot', 'quil', 'loom', 'gaze', 'tatami', 'mesh', 'abacus', 'gestalt', 'fortune',
            'date', 'bodice', 'vellum', 'assuit', 'spreadeagle', 'flirt']
# Restartable services and their crash names (Spindle `crash NAME`), as soak_gate.py.
SERVICES = [('files', 11), ('network', 13), ('bell', 10), ('input', 4), ('shell', 3), ('display', 1)]
KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}
SAMPLE = re.compile(r'\[swatch\.sample\] heap_used=(\d+) heap_mapped=(\d+) heap_blocks=(\d+) heap_peak=(\d+) '
                    r'free_frames=(\d+) free_pages=(\d+) rows=(\d+) row_pages=(\d+)')
FIELDS = ('heap_used', 'heap_mapped', 'heap_blocks', 'heap_peak', 'free_frames', 'free_pages', 'rows', 'row_pages')


def growth(before, after):
    """How much worse memory got from one sample to the next (positive = lost)."""
    return {
        'heap_bytes': after['heap_used'] - before['heap_used'],
        'heap_blocks': after['heap_blocks'] - before['heap_blocks'],
        'frames': before['free_frames'] - after['free_frames'],
        'row_pages': after['row_pages'] - before['row_pages'],
    }


def leaks(grew, tol):
    """The measures over their tolerance, as 'measure=amount'."""
    return [f'{k}={v}' for k, v in grew.items() if v > tol[k]]


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--programs', default=','.join(PROGRAMS), help='comma list; empty for none')
    p.add_argument('--services', action='store_true', help='crash/restart cycles (SEXOS_CRASH_PROBE=1 ISO)')
    p.add_argument('--warm', type=int, default=1)
    p.add_argument('--cycles', type=int, default=4)
    p.add_argument('--service-cycles', type=int, default=2)
    p.add_argument('--tol-frames', type=int, default=48, help='free frames lost (4 KiB pages)')
    p.add_argument('--tol-blocks', type=int, default=24, help='kernel heap blocks gained')
    p.add_argument('--tol-heap-bytes', type=int, default=64 * 1024)
    a = p.parse_args()
    tol = {'heap_bytes': a.tol_heap_bytes, 'heap_blocks': a.tol_blocks, 'frames': a.tol_frames, 'row_pages': 0}
    programs = [n for n in a.programs.split(',') if n]

    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(max(f.seek(0, 2), 1024 ** 3))
    install(disk, [(n, (ROOT / 'iso_root/programs' / n).read_bytes()) for n in ['swatch', *programs]])
    g = boot(a.iso, disk, a.out, 'leak', a.resolution)
    t, q = g['text'], g['q']
    record = {'tolerance': tol, 'cycles': a.cycles, 'warm': a.warm, 'programs': {}, 'services': {}, 'samples': []}
    out = a.out / 'leak.json'
    found = []

    def keys(text):
        for k in text:
            q.key(KEYS.get(k, k))
            time.sleep(.05)

    def command(line, pattern, timeout=120):
        g['focus'](153)
        time.sleep(.4)
        s = len(t())
        keys(line)
        q.key('ret')
        return g['wait'](pattern, s, timeout)

    def sample(label):
        m = command('spawn swatch 1', SAMPLE.pattern, 60)
        got = dict(zip(FIELDS, map(int, m.groups())))
        # Let the sampling record retire before anything else is measured.
        time.sleep(1.5)
        record['samples'].append({'at': label, **got})
        return got

    def cycle(name):
        s = len(t())
        command(f'run {name}', r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', 180)
        m = re.search(r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', t()[s:])
        target, task, surface = m[1], m[2], int(m[3])
        # Most programs say when they are ready; give the rest a moment to settle.
        try:
            g['wait'](rf'\[[\w.-]+\.ready\] target={target}\b', s, 30)
        except Exception:
            pass
        time.sleep(1)
        g['focus'](surface)
        time.sleep(.3)
        s = len(t())
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ', s, 120)
        time.sleep(.5)

    def crash(name, pd):
        s = len(t())
        command(f'crash {name}', rf'\[spindle\.crash\.probe\] target={pd}\b')
        g['wait'](rf'\[service\.restarted\] pd={pd} task=\d+ restarts=\d+ [^\n]*admitted=1', s, 180)
        if pd == 3:
            g['wait'](r'\[shell\.restarted\]', s, 180)
        time.sleep(8)

    def measure(kind, name, step, n):
        for _ in range(a.warm):
            step()
        before = sample(f'{name}:before')
        for _ in range(n):
            step()
        after = sample(f'{name}:after')
        grew = growth(before, after)
        bad = leaks(grew, tol)
        record[kind][name] = {'before': before, 'after': after, 'growth': grew, 'leaks': bad}
        out.write_text(json.dumps(record, indent=1))
        print(f'[leak] {kind[:-1]}={name} ' + ' '.join(f'{k}={v}' for k, v in grew.items()) +
              (' LEAK ' + ','.join(bad) if bad else ' ok'), flush=True)
        if bad:
            found.append(f'{name}: ' + ', '.join(bad))

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        time.sleep(2)
        start = sample('start')
        for name in programs:
            measure('programs', name, lambda name=name: cycle(name), a.cycles)
        if a.services:
            for name, pd in SERVICES:
                measure('services', name, lambda name=name, pd=pd: crash(name, pd), a.service_cycles)
        end = sample('end')
        record['overall'] = growth(start, end)
        for bad in ('KERNEL PANIC', 'panicked at'):
            assert bad not in t(), bad
        record['ok'] = not found
        record['leaks'] = found
        out.write_text(json.dumps(record, indent=1))
        assert not found, 'memory did not come back: ' + '; '.join(found)
        print({'ok': True, 'programs': len(programs), 'services': a.services, 'overall': record['overall']})
    except BaseException as e:
        record['ok'] = False
        record['why'] = repr(e)
        out.write_text(json.dumps(record, indent=1))
        raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
