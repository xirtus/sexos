#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Deterministic app open & close-to-reclaim latency gate.

Measures end-to-end latency in live QEMU using high-precision serial timestamps:
  1. Disk program open (Enter on `run <name>` -> ELF fetch -> kernel load ->
     Shell launch -> first presented frame `[display.app.present.reply]`),
     across cold (run 0) and warm (runs 1..N-1) launches.
  2. Disk program close (`F11` -> `[shell.app.close.request]` -> surface hidden
     `[shell.app.cleanup]` -> `[app.private.retired]`), verifying 0 unreturned pages.
  3. Built-in launcher reopen (Quil successor 16: Enter -> `[app.surface.presented]`)
     and close-to-reclaim (`F11` -> `[sext.app.pool.ready]`), verifying 0 leaked pages.

Fails on any guest fault, missing marker, leaked/unreturned page, or latency
regression above the configured bounds.
"""
import argparse
import json
import os
import re
import shutil
import statistics
import time
from pathlib import Path

KVM = os.access('/dev/kvm', os.R_OK | os.W_OK)
if KVM:
    os.environ.setdefault('SEXOS_GATE_ACCEL', 'kvm')
    os.environ.setdefault('SEXOS_GATE_CPU', 'host')
os.environ.setdefault('SEXOS_GATE_CORES', '4' if KVM else '1')
os.environ.setdefault('SEXOS_GATE_MEMORY', '1G')
os.environ.setdefault('SEXOS_GATE_BOUND', '2400')

ROOT = Path(__file__).resolve().parents[1]
import sys
sys.path.insert(0, str(ROOT / 'scripts'))

from install_program import install
from quil_autosave_gate import boot

SPINDLE = 153
KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus', ':': 'semicolon'}


class Trace:
    """Serial line reader stamping every line with host monotonic arrival time."""
    def __init__(self, path: Path):
        self.path = path
        self.f = None
        self.buf = b''
        self.lines = []
        self.t0 = 0.0

    def start(self) -> float:
        if self.f is None:
            self.f = open(self.path, 'rb')
        self.f.seek(0, 2)
        self.buf = b''
        self.lines = []
        self.t0 = time.monotonic()
        return self.t0

    def poll(self):
        new = self.f.read()
        if new:
            now = time.monotonic()
            self.buf += new
            *whole, self.buf = self.buf.split(b'\n')
            self.lines += [(now, l.decode('utf-8', 'replace').rstrip('\r')) for l in whole]

    def until(self, pattern: str, timeout: float = 30.0, after: int = 0):
        rx = re.compile(pattern)
        end = time.monotonic() + timeout
        i = after
        while True:
            self.poll()
            while i < len(self.lines):
                at, line = self.lines[i]
                if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at', line):
                    raise RuntimeError(f'guest fault: {line}')
                m = rx.search(line)
                if m:
                    return (at - self.t0) * 1000.0, m, i
                i += 1
            if time.monotonic() > end:
                tail = '\n'.join(l for _, l in self.lines[-25:])
                raise TimeoutError(f'no {pattern!r} within {timeout}s\n--- tail ---\n{tail}')
            time.sleep(0.0003)

    def first_matching(self, pattern: str):
        rx = re.compile(pattern)
        for at, line in self.lines:
            m = rx.search(line)
            if m:
                return round((at - self.t0) * 1000.0, 2), m
        return None, None


def tap_key(q, qcode: str):
    """Send press+release via input-send-event (avoids send-key's ~35-80ms hold overhead)."""
    q.cmd('input-send-event', {
        'events': [
            {'type': 'key', 'data': {'down': True, 'key': {'type': 'qcode', 'data': qcode}}},
            {'type': 'key', 'data': {'down': False, 'key': {'type': 'qcode', 'data': qcode}}},
        ]
    })


def median_or_none(vals):
    clean = [v for v in vals if v is not None]
    return round(statistics.median(clean), 2) if clean else None


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', type=Path, default=None)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--programs', default='jot,tatami')
    p.add_argument('--runs', type=int, default=3, help='launches per program (run 0 = cold, runs 1..N-1 = warm)')
    p.add_argument('--max-warm-open-ms', type=float, default=150.0 if KVM else 800.0,
                   help='max allowed median warm open-to-first-frame latency (ms)')
    p.add_argument('--max-close-reclaim-ms', type=float, default=150.0 if KVM else 800.0,
                   help='max allowed median close-to-reclaim latency (ms)')
    p.add_argument('--no-fail-bounds', action='store_true',
                   help='measure and report without failing on latency thresholds (useful for baseline capture)')
    a = p.parse_args()

    shutil.rmtree(a.out, ignore_errors=True)
    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    if a.source_disk is not None:
        shutil.copyfile(a.source_disk, disk)
        with disk.open('r+b') as f:
            f.truncate(max(f.seek(0, 2), 1024 ** 3))
    else:
        with disk.open('wb') as f:
            f.truncate(1024 ** 3)

    programs = [n for n in a.programs.split(',') if n]
    install(disk, [(n, (ROOT / 'iso_root/programs' / n).read_bytes()) for n in programs])

    g = boot(a.iso, disk, a.out, 'openclose', a.resolution)
    t = g['text']
    q = g['q']
    tr = Trace(a.out / 'openclose-serial.log')

    def keys(text: str):
        for k in text:
            q.key(KEYS.get(k, k))
            time.sleep(0.05)

    results = {
        'accel': 'kvm' if KVM else 'tcg',
        'cores': os.environ['SEXOS_GATE_CORES'],
        'cpu': os.environ.get('SEXOS_GATE_CPU', ''),
        'programs': {},
        'launcher_reopen': {},
        'unreturned_pages_total': 0,
    }

    try:
        g['wait'](r'\[app\.private\.retired\] target=41 ', 0, 180)
        g['wait'](r'\[linen\.disk\.publish\.done\]', 0, 180)
        time.sleep(1.5)

        # ── 1. Disk-stored program open & close cycles (cold + warm) ──
        for name in programs:
            runs_out = []
            for run in range(a.runs):
                g['focus'](SPINDLE)
                time.sleep(0.25)
                cmd = f'run {name}'
                s0 = len(t())
                keys(cmd)
                g['wait'](rf'\[spindle\.line\.cursor\] pos={len(cmd)} len={len(cmd)}\b', s0, 30)
                time.sleep(0.15)

                tr.start()
                tap_key(q, 'ret')

                load_ms, lm, load_i = tr.until(
                    r'\[app\.program\.loaded\] target=(\d+) task=(\d+) bytes=(\d+) pages=(\d+) ',
                    timeout=90.0,
                )
                pd, task, elf_bytes, loaded_pages = int(lm[1]), int(lm[2]), int(lm[3]), int(lm[4])
                launch_ms, sm, launch_i = tr.until(
                    rf'\[shell\.program\.launch\] target={pd} task={task} surface=(\d+)',
                    timeout=60.0,
                    after=load_i,
                )
                surface = int(sm[1])
                present_ms, _, present_i = tr.until(
                    rf'\[display\.app\.present\.reply\] target={pd} ',
                    timeout=60.0,
                    after=launch_i,
                )
                tr.poll()
                files_ms, fm = tr.first_matching(r'\[files\.program\.read\] bytes=(\d+) lba=(\d+)(?: cached=(\d+))?')
                cached_files = int(fm[3]) if (fm and fm[3]) else 0

                # Brief settle so any immediate startup heap growth is accounted for.
                time.sleep(0.25)
                tr.poll()
                held_pages = 0
                for _, line in tr.lines:
                    mm = re.search(rf'\[app\.program\.memory\] target={pd} task={task} granted=\d+ held=(\d+) ', line)
                    if mm:
                        held_pages = max(held_pages, int(mm[1]))
                active_pages = loaded_pages + held_pages

                # ── Close & Reclaim ──
                g['focus'](surface)
                time.sleep(0.15)
                tr.start()
                tap_key(q, 'f11')
                req_ms, _, req_i = tr.until(
                    rf'\[shell\.app\.close\.request\] target={pd} task={task} surface={surface}',
                    timeout=30.0,
                )
                hidden_ms, _, hidden_i = tr.until(
                    rf'\[shell\.app\.cleanup\] target={pd} task={task} surface={surface} .*retired=1',
                    timeout=60.0,
                    after=req_i,
                )
                retire_ms, rm, _ = tr.until(
                    rf'\[app\.private\.retired\] target={pd} task={task} pages=(\d+)',
                    timeout=60.0,
                    after=req_i,
                )
                reclaimed_pages = int(rm[1])
                unreturned = max(0, active_pages - reclaimed_pages)
                results['unreturned_pages_total'] += unreturned

                row = {
                    'run': run,
                    'kind': 'cold' if run == 0 else 'warm',
                    'target': pd,
                    'task': task,
                    'bytes': elf_bytes,
                    'files_cached': cached_files,
                    'files_read_ms': files_ms,
                    'elf_loaded_ms': round(load_ms, 2),
                    'shell_launch_ms': round(launch_ms, 2),
                    'first_frame_ms': round(present_ms, 2),
                    'close_request_ms': round(req_ms, 2),
                    'surface_hidden_ms': round(hidden_ms, 2),
                    'reclaimed_ms': round(retire_ms, 2),
                    'active_pages': active_pages,
                    'reclaimed_pages': reclaimed_pages,
                    'unreturned_pages': unreturned,
                }
                runs_out.append(row)
                print(
                    f'[open_close] {name} run={run} ({row["kind"]}) '
                    f'files={row["files_read_ms"]}ms load={row["elf_loaded_ms"]}ms '
                    f'launch={row["shell_launch_ms"]}ms first_frame={row["first_frame_ms"]}ms | '
                    f'close_req={row["close_request_ms"]}ms hidden={row["surface_hidden_ms"]}ms '
                    f'reclaimed={row["reclaimed_ms"]}ms pages={reclaimed_pages}/{active_pages} leak={unreturned}'
                )
                time.sleep(0.2)

            warm_runs = [r for r in runs_out if r['kind'] == 'warm'] or runs_out
            results['programs'][name] = {
                'cold_first_frame_ms': runs_out[0]['first_frame_ms'],
                'warm_first_frame_p50_ms': median_or_none([r['first_frame_ms'] for r in warm_runs]),
                'warm_elf_load_p50_ms': median_or_none([r['elf_loaded_ms'] for r in warm_runs]),
                'close_hidden_p50_ms': median_or_none([r['surface_hidden_ms'] for r in runs_out]),
                'close_reclaim_p50_ms': median_or_none([r['reclaimed_ms'] for r in runs_out]),
                'unreturned_pages': sum(r['unreturned_pages'] for r in runs_out),
                'runs': runs_out,
            }

        # ── 2. Built-in Launcher Reopen (Quil 9 -> Close -> Launcher Reopen Quil 16 -> Close) ──
        g['focus'](201)
        time.sleep(0.2)
        s_q9 = len(t())
        tap_key(q, 'f11')
        m = g['wait'](r'\[sext\.app\.pool\.ready\] target=9 task=\d+ .*ok=1|\[shell\.app\.close\.declined\] target=9 ', s_q9, 90)
        if 'declined' in m[0]:
            tap_key(q, 'f11')
            g['wait'](r'\[sext\.app\.pool\.ready\] target=9 task=\d+ .*ok=1', s_q9, 90)
        time.sleep(0.2)

        # Open launcher, select Quil (row 1), press Enter and time click->first presented frame.
        s_l = len(t())
        tap_key(q, 'grave_accent')
        g['wait'](r'\[shell\.palette\.item\] idx=0', s_l, 15)
        s_j = len(t())
        tap_key(q, 'j')
        g['wait'](r'\[shell\.palette\.select\] old=0 new=1', s_j, 15)
        time.sleep(0.1)

        tr.start()
        tap_key(q, 'ret')
        relaunch_ms, _, rl_i = tr.until(r'\[shell\.app\.relaunch\] predecessor=9 successor=16 task=16 surface=301', timeout=30.0)
        presented_ms, _, _ = tr.until(r'\[app\.surface\.presented\] target=16 surface=301 sequence=1 ok=1', timeout=60.0, after=rl_i)

        time.sleep(0.2)
        g['focus'](301)
        time.sleep(0.15)
        tr.start()
        tap_key(q, 'f11')
        q16_req_ms, _, q16_i = tr.until(r'\[shell\.app\.close\.request\] target=16 task=16 surface=301', timeout=30.0)
        q16_hid_ms, _, _ = tr.until(r'\[shell\.app\.cleanup\] target=16 task=16 surface=301 .*retired=1', timeout=60.0, after=q16_i)
        q16_ret_ms, q16_rm, _ = tr.until(r'\[app\.private\.retired\] target=16 task=16 pages=(\d+) bytes=(\d+) ', timeout=60.0, after=q16_i)
        q16_pool_ms, q16_pm, _ = tr.until(r'\[sext\.app\.pool\.ready\] target=16 task=16 bytes=(\d+) zeroed=(\d+) .*ok=1', timeout=60.0, after=q16_i)
        q16_leak = 0 if (q16_rm[2] == q16_pm[1] == q16_pm[2]) else 1
        results['unreturned_pages_total'] += q16_leak
        results['launcher_reopen'] = {
            'target': 16,
            'relaunch_ms': round(relaunch_ms, 2),
            'first_frame_ms': round(presented_ms, 2),
            'close_request_ms': round(q16_req_ms, 2),
            'surface_hidden_ms': round(q16_hid_ms, 2),
            'reclaimed_ms': round(q16_ret_ms, 2),
            'pool_ready_ms': round(q16_pool_ms, 2),
            'retired_pages': int(q16_rm[1]),
            'unreturned_pages': q16_leak,
        }
        print(
            f'[open_close] launcher(quil16) relaunch={results["launcher_reopen"]["relaunch_ms"]}ms '
            f'first_frame={results["launcher_reopen"]["first_frame_ms"]}ms | '
            f'hidden={results["launcher_reopen"]["surface_hidden_ms"]}ms '
            f'reclaimed={results["launcher_reopen"]["reclaimed_ms"]}ms '
            f'pool_ready={results["launcher_reopen"]["pool_ready_ms"]}ms leak={q16_leak}'
        )

    finally:
        g['stop']()

    (a.out / 'open-close-speed.json').write_text(json.dumps(results, indent=2) + '\n')

    # ── Assertions ──
    if results['unreturned_pages_total'] != 0:
        raise SystemExit(f'FAIL: unreturned pages = {results["unreturned_pages_total"]}')

    if not a.no_fail_bounds:
        failures = []
        for name, data in results['programs'].items():
            if data['warm_first_frame_p50_ms'] is None or data['warm_first_frame_p50_ms'] > a.max_warm_open_ms:
                failures.append(f'{name} warm open p50 {data["warm_first_frame_p50_ms"]}ms > {a.max_warm_open_ms}ms')
            if data['close_reclaim_p50_ms'] is None or data['close_reclaim_p50_ms'] > a.max_close_reclaim_ms:
                failures.append(f'{name} close-to-reclaim p50 {data["close_reclaim_p50_ms"]}ms > {a.max_close_reclaim_ms}ms')
        lr = results['launcher_reopen']
        if lr.get('first_frame_ms', 999999) > a.max_warm_open_ms:
            failures.append(f'launcher(quil16) open {lr.get("first_frame_ms")}ms > {a.max_warm_open_ms}ms')
        if lr.get('pool_ready_ms', 999999) > a.max_close_reclaim_ms:
            failures.append(f'launcher(quil16) close-to-pool-ready {lr.get("pool_ready_ms")}ms > {a.max_close_reclaim_ms}ms')
        if failures:
            raise SystemExit('FAIL: ' + '; '.join(failures))

    print('[app.open_close.speed.gate] PASS')


if __name__ == '__main__':
    main()
