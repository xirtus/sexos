#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Spindle's `fetch` survives a download that drops part way, as Wi-Fi downloads do.

A host server (the guest sees it as 10.0.2.2) serves one 1 MiB file four ways:
1. /fin/   the first answer stops early with a clean FIN; the resume asks with `Range`
           and gets 206: the file arrives whole (Spindle's `sum` matches the host's FNV).
2. /rst/   the first answer is cut by a reset; the server ignores `Range` and sends the
           whole file again (200): Spindle cuts the file and still ends whole.
3. /bad/   every answer is cut: after its resumes Spindle says the download is
           incomplete with how much arrived, never "Saved".
4. /ok/    a plain download: no resume.
"""
import argparse, http.server, os, random, re, shutil, socket, struct, sys, threading, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('SEXOS_GATE_NIC', '-netdev user,id=n0 -device e1000e,netdev=n0')
sys.path.insert(0, str(ROOT / 'scripts'))
from quil_autosave_gate import boot  # noqa: E402

KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}
SIZE = 1024 * 1024
CUT = 300 * 1024


def fnv(data):
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024 ** 3)
    body = random.Random(7).randbytes(SIZE)
    want = f'{fnv(body):016x}'
    seen = []  # (mode, range header) per request

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.0'

        def do_GET(self):
            mode = self.path.strip('/').split('/')[0]
            rng = self.headers.get('Range')
            seen.append((mode, rng))
            first = sum(1 for m, _ in seen if m == mode) == 1
            start = 0
            m = re.fullmatch(r'bytes=(\d+)-', rng or '')
            if m and mode == 'fin':
                start = int(m[1])
                self.send_response(206)
                self.send_header('Content-Range', f'bytes {start}-{SIZE - 1}/{SIZE}')
            else:
                self.send_response(200)
            self.send_header('Content-Length', str(SIZE - start))
            self.end_headers()
            cut = (mode in ('fin', 'rst') and first) or mode == 'bad'
            data = body[start:start + CUT] if cut else body[start:]
            for i in range(0, len(data), 8192):
                self.wfile.write(data[i:i + 8192])
            self.wfile.flush()
            if cut and mode != 'fin':
                # A reset, as a dropped Wi-Fi link or a timed-out flow looks to the guest.
                self.connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
                self.close_connection = True

        def log_message(self, *x):
            pass

    class Quiet(http.server.ThreadingHTTPServer):
        daemon_threads = True

    server = Quiet(('0.0.0.0', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f'http://10.0.2.2:{server.server_address[1]}'

    g = boot(a.iso, disk, a.out, 'fetch-resume', a.resolution)
    t, q = g['text'], g['q']

    def line(text):
        for k in text:
            if k == ':':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'semicolon'}], 'hold-time': 80})
            else:
                q.key(KEYS.get(k, k))
            time.sleep(.04)
        q.key('ret')

    report = {}
    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 180)
        time.sleep(2)
        g['focus'](153)
        time.sleep(.5)
        for mode in ('fin', 'rst', 'ok'):
            s = len(t())
            line(f'fetch {base}/{mode}/f {mode}.bin')
            m = g['wait'](r'\[spindle\.fetch\.done\] bytes=(\d+)|\[spindle\.fetch\.line\] fetch: (?!.*resuming)[^\n]*', s, 300)
            assert m[1] == str(SIZE), (mode, m[0])
            resumes = re.findall(r'\[spindle\.fetch\.resume\][^\n]*', t()[s:])
            assert (len(resumes) >= 1) == (mode != 'ok'), (mode, resumes)
            s = len(t())
            line(f'sum {mode}.bin')
            m = g['wait'](r'(\d+) bytes, fnv ([0-9a-f]{16})', s, 120)
            assert m[1] == str(SIZE) and m[2] == want, (mode, m[0], want)
            report[mode] = {'resumes': len(resumes), 'fnv': m[2]}
        ranges = [r for md, r in seen if md == 'fin' and r]
        assert ranges and all(0 < int(r.split('=')[1][:-1]) <= CUT for r in ranges), seen
        s = len(t())
        line(f'fetch {base}/bad/f bad.bin')
        m = g['wait'](r'\[spindle\.fetch\.incomplete\][^\n]*|\[spindle\.fetch\.done\][^\n]*', s, 300)
        assert 'incomplete' in m[0] and f'of={SIZE}' in m[0], m[0]
        assert re.search(r'fetch: download incomplete \([^)]*\): got \d+ of 1048576 bytes', t()[s:]), t()[s:][-600:]
        report['bad'] = m[0]
        assert 'KERNEL PANIC' not in t()
    finally:
        g['stop']()
        server.shutdown()
    print({'ok': True, **report})


if __name__ == '__main__':
    main()
