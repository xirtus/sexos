#!/usr/bin/env python3
# GPL-3.0-only
r"""12-Hour CI-able Long-Run Soak Gate for SexOS.

A continuous working session repeated for hours with three mandatory workload pillars:
1. Open & Close Apps: Rotates through user-space apps (jot, fortune, abacus, pdf,
   view, and mesh/monitor), verifying [shell.program.launch] and clean retirement
   [app.private.retired] on F11 close.
2. Play Audio: Plays sound through Bell (beep tone, hum records, and play tone),
   asserting [play.*] / [hum.*] / [bell.*] receipts complete without hang or crash.
3. Copy & Verify Files: Creates/fetches round-seeded binary payloads, copies/moves
   them via Spindle file commands, verifies exact byte length & content hash, retains
   hourly canaries across segments, and deletes scratch files to prevent disk exhaustion.

Also features rigorous memory leak & key latency analysis (SoakAnalyzer):
- Samples free_pages from Mesh/Monitor ([monitor.sample] ... free_pages=(\d+)) every round.
- Tracks post-warmup baseline, maximum drop (--leak-pages, default 256 pages = 1 MiB),
  and ordinary-least-squares linear regression slope (pages/hour and pages/round).
- Measures key-to-pixel latency (key_ms, failing if > --max-key-ms).
- Seamless multi-segment execution (--resume-report and --max-segment-hours) allowing
  a full 12-hour soak across two 6-hour runner segments without losing baseline or canaries.
- On any timeout, panic, or threshold breach, writes structured failure diagnostics to
  long-soak.json + long-soak-summary.md and screendump before exiting non-zero.
"""

import argparse
import hashlib
import http.server
import json
import os
import re
import shutil
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

# Dynamic HTTP server for payload fetches
PAGE = b'<html><head><title>SexOS Long Soak</title></head><body><p>SexOS Soak Server</p></body></html>'
FILES = {'/': PAGE}
FILES_LOCK = threading.Lock()


class SoakHTTPHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        with FILES_LOCK:
            body = FILES.get(self.path)
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class QuietServer(http.server.ThreadingHTTPServer):
    daemon_threads = True


def start_http_server():
    server = QuietServer(('127.0.0.1', 0), SoakHTTPHandler)
    port = server.server_address[1]
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, port

KEYS = {
    ' ': 'spc',
    '/': 'slash',
    '.': 'dot',
    '-': 'minus',
    '_': 'minus',
    ':': 'semicolon',
    '=': 'equal',
    '+': 'equal',
    '*': '8',
}


class SoakAnalyzer:
    """Rigorous memory leak, latency, and regression analyzer for long-run soak."""

    def __init__(self, warmup=3, leak_pages=256, max_key_ms=3000.0, baseline_pages=None, cumulative_rounds=None):
        self.warmup = warmup
        self.leak_pages = leak_pages
        self.max_key_ms = max_key_ms
        self.baseline_pages = baseline_pages
        self.rounds = list(cumulative_rounds or [])

    def record_round(self, round_info):
        self.rounds.append(round_info)
        free_pages = round_info['free_pages']
        if self.baseline_pages is None:
            if len(self.rounds) <= self.warmup:
                warmup_samples = [r['free_pages'] for r in self.rounds]
                self.baseline_pages = max(warmup_samples)
            else:
                self.baseline_pages = free_pages
        elif len(self.rounds) <= self.warmup and not round_info.get('resumed', False):
            if free_pages > self.baseline_pages:
                self.baseline_pages = free_pages

    def check_round_health(self, round_info):
        n = round_info['n']
        key_ms = round_info.get('key_ms', 0.0)
        if key_ms > self.max_key_ms:
            return False, f"round {n}: key latency {key_ms} ms exceeded max threshold {self.max_key_ms} ms"

        free_pages = round_info['free_pages']
        if (len(self.rounds) > self.warmup or round_info.get('resumed', False)) and self.baseline_pages is not None:
            drop = self.baseline_pages - free_pages
            if drop > self.leak_pages:
                return False, (
                    f"round {n}: free program memory {free_pages} pages dropped {drop} pages "
                    f"below baseline {self.baseline_pages} (leak threshold: {self.leak_pages} pages / "
                    f"{round(self.leak_pages * 4 / 1024, 2)} MiB)"
                )

        # OLS slope trend check: require >= 6 post-warmup rounds before projecting 12h slope
        slope_info = self.compute_ols_slope()
        if len(self.rounds) >= self.warmup + 6 and slope_info['r_squared'] >= 0.75:
            # If projected drop over 12 hours exceeds leak_pages
            proj_12h_drop = -slope_info['slope_per_hour'] * 12.0
            if slope_info['slope_per_hour'] < -15.0 and proj_12h_drop > self.leak_pages * 1.5:
                return False, (
                    f"round {n}: steady linear memory leak detected via OLS regression: "
                    f"{slope_info['slope_per_hour']} pages/hr (R²={slope_info['r_squared']}), "
                    f"projected 12h drop {round(proj_12h_drop)} pages > {self.leak_pages}"
                )

        return True, None

    def compute_ols_slope(self):
        if len(self.rounds) < 2:
            single = self.rounds[0]['free_pages'] if self.rounds else 0
            return {
                'slope_per_round': 0.0,
                'slope_per_hour': 0.0,
                'r_squared': 0.0,
                'min_free_pages': single,
                'max_drop': 0,
            }

        # Analyze trend across post-warmup rounds once >= 2 post-warmup samples exist
        data = self.rounds[self.warmup:] if len(self.rounds) >= self.warmup + 2 else self.rounds
        n_pts = len(data)
        xs = [r['n'] for r in data]
        ys = [r['free_pages'] for r in data]
        ts = [r.get('elapsed_s', 0.0) / 3600.0 for r in data]

        mean_x = sum(xs) / n_pts
        mean_y = sum(ys) / n_pts
        mean_t = sum(ts) / n_pts

        var_x = sum((x - mean_x) ** 2 for x in xs)
        var_t = sum((t - mean_t) ** 2 for t in ts)
        cov_xy = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys))
        cov_ty = sum((t_val - mean_t) * (y - mean_y) for t_val, y in zip(ts, ys))
        var_y = sum((y - mean_y) ** 2 for y in ys)

        slope_round = cov_xy / var_x if var_x > 0 else 0.0
        slope_hour = cov_ty / var_t if var_t > 0 else 0.0
        r_sq = (cov_xy ** 2) / (var_x * var_y) if (var_x > 0 and var_y > 0) else 0.0

        all_pages = [r['free_pages'] for r in self.rounds]
        min_pages = min(all_pages)
        max_drop = (self.baseline_pages - min_pages) if self.baseline_pages is not None else 0

        return {
            'slope_per_round': round(slope_round, 4),
            'slope_per_hour': round(slope_hour, 4),
            'r_squared': round(r_sq, 4),
            'min_free_pages': min_pages,
            'max_drop': max(0, max_drop),
        }


def generate_summary_markdown(analyzer, ok, why, total_hours, elapsed_seconds, canary_files, serial_tail=None):
    slope_info = analyzer.compute_ols_slope()
    total_rounds = len(analyzer.rounds)
    status_alert = "> [!NOTE]\n> **Status**: PASS — Zero leaks, hangs, or faults detected." if ok else f"> [!CAUTION]\n> **Status**: FAILED\n> **Reason**: {why}"

    lines = [
        "# SexOS 12-Hour Long-Run Soak Gate Report",
        "",
        status_alert,
        "",
        "## Soak Gate Telemetry Summary",
        "",
        "| Metric | Value | Threshold / Limit |",
        "|---|---|---|",
        f"| **Status** | `{'PASS' if ok else 'FAIL'}` | `PASS` |",
        f"| **Total Soak Duration** | `{round(elapsed_seconds / 3600.0, 2)} hrs` (`{round(elapsed_seconds, 1)} s`) | Target: `{total_hours} hrs` |",
        f"| **Completed Rounds** | `{total_rounds}` | Sustained |",
        f"| **Baseline Free Pages** | `{analyzer.baseline_pages or 'N/A'}` | Post-warmup reference |",
        f"| **Minimum Free Pages** | `{slope_info['min_free_pages']}` | - |",
        f"| **Peak Memory Drop** | `{slope_info['max_drop']} pages` (`{round(slope_info['max_drop'] * 4 / 1024, 2)} MiB`) | Max: `{analyzer.leak_pages} pages` (`{round(analyzer.leak_pages * 4 / 1024, 2)} MiB`) |",
        f"| **OLS Leak Slope (per hour)** | `{slope_info['slope_per_hour']} pages/hr` | Near zero |",
        f"| **OLS Leak Slope (per round)** | `{slope_info['slope_per_round']} pages/round` (R²=`{slope_info['r_squared']}`) | Near zero |",
        f"| **Max Key-to-Pixel Latency** | `{max((r.get('key_ms', 0.0) for r in analyzer.rounds), default=0.0)} ms` | Max: `{analyzer.max_key_ms} ms` |",
        f"| **Historical Canaries Verified** | `{len(canary_files)}` files{f' ({chr(44).join(sorted(canary_files.keys()))})' if canary_files else ''} | 100% SHA-256 match |",
        "",
        "## Sampled Workload Rounds",
        "",
        "| Round | Elapsed (min) | Duration (s) | Free Pages | Drop | Key Latency (ms) | App Tested | Audio Verified | File Verification |",
        "|---|---|---|---|---|---|---|---|---|",
    ]

    # Show first 5, last 5, and every 10th round
    rounds_to_show = []
    for i, r in enumerate(analyzer.rounds):
        if i < 5 or i >= total_rounds - 5 or i % 10 == 0:
            if not rounds_to_show or rounds_to_show[-1]['n'] != r['n']:
                rounds_to_show.append(r)

    for r in rounds_to_show:
        drop = (analyzer.baseline_pages - r['free_pages']) if analyzer.baseline_pages is not None else 0
        lines.append(
            f"| {r['n']} | {round(r.get('elapsed_s', 0.0) / 60.0, 1)} | {r.get('round_s', 0.0)} | "
            f"{r['free_pages']} | {drop} | {r.get('key_ms', 0.0)} | `{r.get('app', 'none')}` | "
            f"`{r.get('audio', 'none')}` | `{r.get('file_status', 'ok')}` |"
        )

    if not ok and serial_tail:
        lines.extend([
            "",
            "## Failure Diagnostics & Serial Log Tail",
            "",
            "```text",
            *serial_tail[-40:],
            "```",
        ])

    return '\n'.join(lines) + '\n'


def main():
    p = argparse.ArgumentParser(description="SexOS 12-Hour CI-able Long-Run Soak Gate")
    p.add_argument('--iso', required=True, type=Path, help="Path to SexOS bootable ISO")
    p.add_argument('--source-disk', required=True, type=Path, help="Source disk image")
    p.add_argument('--out', required=True, type=Path, help="Output evidence directory")
    p.add_argument('--resolution', default='1280x800', help="Display resolution")
    p.add_argument('--hours', type=float, default=12.0, help="Total soak duration in hours")
    p.add_argument('--minutes', type=float, default=None, help="Soak duration in minutes (overrides --hours)")
    p.add_argument('--max-rounds', type=int, default=None, help="Maximum rounds to run (for fast CI smoke)")
    p.add_argument('--resume-report', type=Path, default=None, help="Path to previous segment report JSON")
    p.add_argument('--max-segment-hours', type=float, default=None, help="Max duration for this runner segment")
    p.add_argument('--segment-id', type=int, default=None, help="Explicit segment index for logging")
    p.add_argument('--leak-pages', type=int, default=256, help="Max allowed drop in free pages from baseline")
    p.add_argument('--max-key-ms', type=float, default=3000.0, help="Max allowed key latency in ms")
    p.add_argument('--warmup', type=int, default=3, help="Number of warmup rounds before leak checks")
    p.add_argument('--cores', type=str, default=None, help="QEMU CPU cores")
    a = p.parse_args()

    # Determine total target duration
    total_hours = (a.minutes / 60.0) if a.minutes is not None else a.hours
    total_duration_s = total_hours * 3600.0

    # Environment setup
    if a.cores:
        os.environ['SEXOS_GATE_CORES'] = str(a.cores)
    os.environ.setdefault('SEXOS_GATE_NIC', '-netdev user,id=n0 -device e1000e,netdev=n0')
    os.environ.setdefault('SEXOS_GATE_DEVICES', '-audiodev none,id=snd0 -device intel-hda -device hda-duplex,audiodev=snd0')

    # Compute timeout bound for boot()
    segment_limit_s = (a.max_segment_hours * 3600.0) if a.max_segment_hours is not None else total_duration_s
    run_bound_s = int(min(total_duration_s, segment_limit_s) + 3600)
    os.environ['SEXOS_GATE_BOUND'] = str(run_bound_s)

    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'

    # Check for resume report
    resumed = False
    prior_rounds = []
    baseline_pages = None
    canary_files = {}
    prior_elapsed_s = 0.0

    if a.resume_report and a.resume_report.exists():
        report_data = json.loads(a.resume_report.read_text())
        baseline_pages = report_data.get('baseline_pages')
        prior_rounds = report_data.get('rounds', [])
        canary_files = report_data.get('canary_files', {})
        prior_elapsed_s = report_data.get('elapsed_seconds', 0.0)
        resumed = True
        print(f"[long-soak] Resuming from {a.resume_report}: baseline={baseline_pages}, "
              f"prior_rounds={len(prior_rounds)}, prior_elapsed={round(prior_elapsed_s/60, 1)} min", flush=True)

    analyzer = SoakAnalyzer(
        warmup=a.warmup,
        leak_pages=a.leak_pages,
        max_key_ms=a.max_key_ms,
        baseline_pages=baseline_pages,
        cumulative_rounds=prior_rounds,
    )

    # Initialize disk
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.seek(0, os.SEEK_END)
        cur_sz = f.tell()
        if cur_sz < 1024**3:
            f.truncate(1024**3)

    # Pre-install programs if available in iso_root/programs and not already resuming
    progs_dir = ROOT / 'iso_root/programs'
    mesh_name = 'mesh' if (progs_dir / 'mesh').exists() else 'monitor'

    if not resumed and progs_dir.exists():
        from install_program import install
        to_install = []
        for name in ['jot', 'fortune', 'hum', mesh_name, 'qupid', 'abacus', 'pdf', 'view']:
            p_bin = progs_dir / name
            if p_bin.exists():
                to_install.append((name, p_bin.read_bytes()))
        if to_install:
            install(disk, to_install)

    # Populate HTTP server files
    if progs_dir.exists():
        with FILES_LOCK:
            for name in ['jot', 'fortune', 'hum', mesh_name, 'qupid', 'abacus', 'pdf', 'view']:
                p_bin = progs_dir / name
                if p_bin.exists():
                    FILES['/' + name] = p_bin.read_bytes()

    # Determine rotating apps pool
    rotating_apps = ['jot', 'fortune']
    if (progs_dir / 'abacus').exists():
        rotating_apps.append('abacus')
    if (progs_dir / 'pdf').exists():
        rotating_apps.append('pdf')
    if (progs_dir / 'view').exists():
        rotating_apps.append('view')

    _http_server, http_port = start_http_server()

    from quil_autosave_gate import boot
    seg_num = a.segment_id if a.segment_id is not None else (2 if resumed else 1)
    tag = f"soak-seg{seg_num}"
    g = boot(a.iso, disk, a.out, tag, a.resolution)
    t = g['text']
    q = g['q']

    def keys(text):
        for k in text:
            if k == ':':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'semicolon'}], 'hold-time': 80})
            elif k == '_':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'minus'}], 'hold-time': 80})
            elif k == '+':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'equal'}], 'hold-time': 80})
            elif k == '*':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': '8'}], 'hold-time': 80})
            elif k.isupper():
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': k.lower()}], 'hold-time': 80})
            else:
                q.key(KEYS.get(k, k))
            time.sleep(0.06)

    def chord(mod, key):
        q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': mod}, {'type': 'qcode', 'data': key}], 'hold-time': 80})

    def expect(text_line, pattern, timeout=120):
        g['focus'](153)
        time.sleep(0.3)
        s = len(t())
        keys(text_line)
        q.key('ret')
        return g['wait'](pattern, s, timeout)

    def line(text_line):
        g['focus'](153)
        time.sleep(0.3)
        keys(text_line)
        q.key('ret')

    base_url = f"http://10.0.2.2:{http_port}"

    def key_ms():
        g['focus'](153)
        time.sleep(0.5)
        s = len(t())
        t0 = time.monotonic()
        q.key('x')
        while not re.search(r'\[display\.text\.committed\] target=12 ', t()[s:]):
            assert time.monotonic() - t0 < 30.0, 'a key never reached the screen'
            time.sleep(0.001)
        ms = (time.monotonic() - t0) * 1000.0
        time.sleep(0.1)
        q.key('backspace')
        return round(ms, 1)

    def sample_free_pages():
        s = len(t())
        line(f'run {mesh_name}')
        m_launch = g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', s, 60)
        target, task, sid = m_launch[1], m_launch[2], int(m_launch[3])
        m_samp = g['wait'](r'\[monitor\.sample\] [^\n]*free_pages=(\d+)', s, 60)
        pages = int(m_samp[1])
        g['focus'](sid)
        time.sleep(0.2)
        s_close = len(t())
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ', s_close, 90)
        return pages

    def run_workload_app(app_name, round_n):
        s = len(t())
        line(f'run {app_name}')
        m_launch = g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', s, 60)
        target, task, sid = m_launch[1], m_launch[2], int(m_launch[3])

        if app_name == 'jot':
            g['wait'](r'\[jot\.ready\]', s, 60)
            g['focus'](sid)
            time.sleep(0.2)
            keys(f'r{round_n}')
            chord('ctrl', 's')
            g['wait'](r'\[jot\.saved\]', s, 60)
        elif app_name == 'fortune':
            g['wait'](r'\[fortune\.ready\]', s, 60)
            time.sleep(0.2)
        elif app_name == 'abacus':
            g['wait'](r'\[abacus\.ready\]', s, 60)
            g['focus'](sid)
            time.sleep(0.2)
            keys(f'{round_n}+1')
            q.key('ret')
            g['wait'](r'\[abacus\.result\]', s, 60)
        elif app_name == 'pdf':
            g['wait'](r'\[pdf\.open\]|\[display\.text\.committed\]|\[display\.app\.pixels\]', s, 60)
            time.sleep(0.2)
        elif app_name == 'view':
            g['wait'](r'\[view\.ready\]|\[display\.text\.committed\]|\[display\.app\.pixels\]', s, 60)
            time.sleep(0.2)
        else:
            time.sleep(0.2)

        g['focus'](sid)
        time.sleep(0.2)
        s_close = len(t())
        q.key('f11')
        g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ', s_close, 90)
        return app_name

    def run_audio(round_n):
        hz = 440 + ((round_n - 1) % 4) * 110
        s_audio = len(t())
        m_tone = expect(f'beep {hz} 200 30', rf'\[bell\.sound\.tone\] caller_pd=\d+ hz={hz} ms=200 volume=30 queued=(\d+) card=(\d+)', 30)
        if int(m_tone[2]) > 0:
            g['wait'](r'\[bell\.sound\.drained\]', s_audio, 60)

        # Periodically spawn hum background tasks and clean them up
        if round_n % 3 == 0:
            expect('spawn hum 3', r'\[shell\.records\.spawned\] live=3\b', 30)
            g['focus'](153)
            time.sleep(0.2)
            keys('stopall force')
            q.key('ret')
            time.sleep(3)
            m = expect('records', r'\[records\.census\] live=(\d+) started=\d+ ended=\d+ capacity=\d+ pool_free=(\d+) pool_total=(\d+)', 30)
            assert m[1] == '0' and m[2] == m[3], f'hum records left behind: {m[0]}'

        # If play binary is present, test play --tone periodically
        if (progs_dir / 'qupid').exists() and round_n % 4 == 0:
            s_play = len(t())
            line(f'run qupid --tone {hz} 48000')
            g['wait'](rf'\[play\.tone\] hz={hz} rate=48000 frames=(\d+) played=(\d+)', s_play, 60)
            m_launch = re.search(r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', t()[s_play:])
            if m_launch:
                p_tgt, p_tsk, p_sid = m_launch[1], m_launch[2], int(m_launch[3])
                g['focus'](p_sid)
                time.sleep(0.2)
                s_c = len(t())
                q.key('f11')
                g['wait'](rf'\[app\.private\.retired\] target={p_tgt} task={p_tsk} ', s_c, 60)

        return f"beep {hz}Hz"

    def run_file_ops(round_n, cur_elapsed_s):
        # 1. Create round payload and register with HTTP server
        seed = 42000 + round_n
        payload_body = f"SOAK_PAYLOAD_ROUND_{round_n}_SEED_{seed}"
        payload_hash = hashlib.sha256(payload_body.encode()).hexdigest()
        full_payload = f"{payload_body}_HASH_{payload_hash[:16]}\n".encode()

        with FILES_LOCK:
            FILES[f'/soak_{round_n}.bin'] = full_payload

        # 2. Fetch payload via Spindle HTTP fetch
        m_fetch = expect(f'fetch {base_url}/soak_{round_n}.bin scratch_{round_n}.bin',
                         r'\[spindle\.fetch\.line\] (Saved \d+ bytes|fetch: [^\n]*)', 60)
        assert m_fetch[1] == f'Saved {len(full_payload)} bytes', f"fetch mismatch: {m_fetch[1]}"

        # 3. Copy / move scratch file
        expect(f'mv scratch_{round_n}.bin copy_{round_n}.bin', r'\[spindle\.fs\.result\] kind=\d+ status=0', 30)

        # 4. Read back and verify hash
        m_cat = expect(f'cat copy_{round_n}.bin', rf'\[spindle\.fs\.cat\] ({re.escape(payload_body)}[^\n]*)', 30)
        assert payload_hash[:16] in m_cat[1], f"cat content mismatch: {m_cat[1]}"

        # 5. Hourly canary persistence
        current_hour = int(cur_elapsed_s / 3600.0) + 1
        canary_filename = f"canary_h{current_hour}.txt"
        if canary_filename not in canary_files:
            expect(f'write {canary_filename} canary_hour_{current_hour}_hash_{payload_hash[:16]}',
                   r'\[spindle\.fs\.result\] kind=8 status=0', 30)
            canary_files[canary_filename] = payload_hash[:16]

        # 6. Verify all historical canary files
        for c_file, c_hash in canary_files.items():
            expect(f'cat {c_file}', rf'\[spindle\.fs\.cat\] [^\n]*({re.escape(c_hash)})', 30)

        # 7. Delete scratch copy and verify df
        expect(f'rm copy_{round_n}.bin', r'\[spindle\.fs\.result\] kind=\d+ status=0', 30)
        expect('df', r'MiB free of', 30)

        with FILES_LOCK:
            FILES.pop(f'/soak_{round_n}.bin', None)

        return payload_hash[:16]

    def save_state(ok, why="", completed=False, serial_tail=None):
        cur_elapsed = prior_elapsed_s + (time.monotonic() - segment_start)
        ols = analyzer.compute_ols_slope()
        report = {
            'ok': ok,
            'why': why,
            'completed': completed,
            'hours': round(cur_elapsed / 3600.0, 2),
            'elapsed_seconds': round(cur_elapsed, 1),
            'total_rounds': len(analyzer.rounds),
            'baseline_pages': analyzer.baseline_pages,
            'min_free_pages': ols['min_free_pages'],
            'max_leak_pages': ols['max_drop'],
            'leak_threshold': a.leak_pages,
            'ols_slope': ols,
            'canary_files': canary_files,
            'rounds': analyzer.rounds,
        }
        if not ok and serial_tail:
            report['diagnostics'] = {'error': why, 'serial_tail': serial_tail[-60:]}

        (a.out / 'long-soak.json').write_text(json.dumps(report, indent=2))
        md = generate_summary_markdown(analyzer, ok, why, total_hours, cur_elapsed, canary_files, serial_tail)
        (a.out / 'long-soak-summary.md').write_text(md)

    segment_start = time.monotonic()
    round_counter = len(prior_rounds) + 1
    new_rounds_in_segment = 0

    try:
        # Initial desktop readiness check
        g['wait'](r'\[app\.private\.retired\] target=41 ', 0, 180)
        time.sleep(2)

        # Audio ready check
        g['wait'](r'\[bell\.sound\.ready\]|\[bell\.sound\.absent\]', 0, 120)

        # If resuming, verify pre-existing canary files from prior segment
        if resumed and canary_files:
            print(f"[long-soak] Verifying {len(canary_files)} historical canary files on resumed disk...", flush=True)
            for c_file, c_hash in canary_files.items():
                expect(f'cat {c_file}', rf'\[spindle\.fs\.cat\] [^\n]*({re.escape(c_hash)})', 30)
            print("[long-soak] Historical canaries verified intact.", flush=True)

        while True:
            cur_elapsed = prior_elapsed_s + (time.monotonic() - segment_start)
            seg_elapsed = time.monotonic() - segment_start

            # Termination condition 1: reached total soak duration
            if cur_elapsed >= total_duration_s:
                print(f"[long-soak] Reached total duration {round(cur_elapsed/3600.0, 2)} hrs.", flush=True)
                break

            # Termination condition 2: reached segment limit
            if a.max_segment_hours is not None and seg_elapsed >= a.max_segment_hours * 3600.0:
                print(f"[long-soak] Reached segment limit {round(seg_elapsed/3600.0, 2)} hrs. Finishing segment.", flush=True)
                break

            # Termination condition 3: max rounds reached (for smoke testing)
            if a.max_rounds is not None and new_rounds_in_segment >= a.max_rounds:
                print(f"[long-soak] Reached max rounds {a.max_rounds}. Finishing smoke run.", flush=True)
                break

            r0 = time.monotonic()
            n = round_counter
            app_to_test = rotating_apps[(n - 1) % len(rotating_apps)]

            # Pillar 1: Open & Close App
            tested_app = run_workload_app(app_to_test, n)

            # Pillar 2: Play Audio through Bell
            audio_result = run_audio(n)

            # Pillar 3: Copy & Verify Files
            file_hash = run_file_ops(n, cur_elapsed)

            # Telemetry: Memory sample from Mesh/Monitor + Key latency
            free_mem = sample_free_pages()
            lat_ms = key_ms()

            round_duration = round(time.monotonic() - r0, 2)
            cur_elapsed = prior_elapsed_s + (time.monotonic() - segment_start)

            round_info = {
                'n': n,
                'elapsed_s': round(cur_elapsed, 1),
                'round_s': round_duration,
                'free_pages': free_mem,
                'key_ms': lat_ms,
                'app': tested_app,
                'audio': audio_result,
                'file_status': f"hash_{file_hash}",
                'resumed': resumed,
            }

            analyzer.record_round(round_info)
            healthy, reason = analyzer.check_round_health(round_info)
            if not healthy:
                raise RuntimeError(reason)

            # Check kernel serial log for panics or fatal faults
            text_now = t()
            for bad_marker in ('KERNEL PANIC', 'KERNEL PAGE FAULT', 'DOUBLE FAULT', 'panicked at'):
                if bad_marker in text_now:
                    raise RuntimeError(f"Detected kernel failure: {bad_marker}")

            new_rounds_in_segment += 1
            round_counter += 1

            # Periodically write progress report
            save_state(ok=True, why="", completed=False)

            drop_from_baseline = (analyzer.baseline_pages - free_mem) if analyzer.baseline_pages is not None else 0
            print(f"[long-soak] round={n} duration={round_duration}s free_pages={free_mem} drop={drop_from_baseline} "
                  f"key_ms={lat_ms} elapsed_min={round(cur_elapsed/60.0, 1)} app={tested_app}", flush=True)

        # Successful completion of segment / full run
        cur_elapsed = prior_elapsed_s + (time.monotonic() - segment_start)
        q.cmd('screendump', {'filename': str(a.out / 'end.ppm')})
        time.sleep(0.5)
        completed = cur_elapsed >= total_duration_s
        save_state(ok=True, why="", completed=completed)
        print(f"[long-soak] SUCCESS: ok=True, rounds={len(analyzer.rounds)}, elapsed_hrs={round(cur_elapsed/3600.0, 2)}", flush=True)

    except BaseException as e:
        tail = []
        try:
            if g and 'q' in g:
                g['q'].cmd('screendump', {'filename': str(a.out / 'fail.ppm')})
            if g and 'text' in g:
                tail = [line for line in g['text']().splitlines() if line.strip()][-60:]
        except Exception:
            pass

        save_state(ok=False, why=repr(e), completed=False, serial_tail=tail)
        print(f"[long-soak] FAILURE: {e}", file=sys.stderr, flush=True)
        raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
