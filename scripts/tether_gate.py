#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Spindle's `tether` and `hw record`, the first-boot helpers for the Alienware x17 R1.

1. `scripts/tether_host.py serve` waits on this host (the guest sees it as 10.0.2.2).
2. `tether 10.0.2.2:PORT` typed in Spindle: the host gets `SEXOS-TETHER 1`, the whole
   boot log head between its markers, then Spindle's lines; the host's automatic `hw`
   comes back as `remote> hw` and the hardware report follows.
3. A command sent from the host (`df`) runs and its answer streams back; `install go`
   from the host is refused; `fetch` typed while tethered waits for `tether off`.
4. `tether off` closes the flow and the host sees the session end.
5. `hw record` writes hw-record-1.txt now and hw-record-2.txt 30 s later, each holding
   the report and Spindle's newest lines.
"""
import argparse, os, re, shutil, socket, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HOST = ROOT / 'scripts/tether_host.py'


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'
sys.path.insert(0, str(ROOT / 'scripts'))
from quil_autosave_gate import boot  # noqa: E402

KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}


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
    logs = Path(tempfile.mkdtemp(prefix='tether-gate-'))
    port = free_port()
    serve = subprocess.Popen([sys.executable, str(HOST), '--dir', str(logs), 'serve', '--port', str(port), '--once', '--quiet', '--auto', 'hw'],
                             stdout=open(a.out / 'tether-host.log', 'w'), stderr=subprocess.STDOUT)

    def host(*args):
        return subprocess.run([sys.executable, str(HOST), '--dir', str(logs), *args], capture_output=True, text=True, timeout=120)

    def stream():
        c = logs / 'current.log'
        return c.read_text(errors='replace') if c.exists() else ''

    def until(pattern, timeout=180):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            m = re.search(pattern, stream())
            if m:
                return m
            time.sleep(0.5)
        raise AssertionError(f'host never saw {pattern!r}; stream tail: {stream()[-800:]}')

    g = boot(a.iso, disk, a.out, 'tether', a.resolution)
    t = g['text']
    q = g['q']

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
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        time.sleep(2)
        g['focus'](153)
        time.sleep(.5)
        s = len(t())
        line('df')
        g['wait'](r'MiB free of', s, 60)

        s = len(t())
        line(f'tether 10.0.2.2:{port}')
        g['wait'](r'\[spindle\.tether\.up\] port=', s, 180)
        g['wait'](r'\[spindle\.tether\.log\] bytes=\d+', s, 180)
        until(r'--- end of boot log head ---\n--- spindle ---\n')
        text = stream()
        assert text.startswith('SEXOS-TETHER 1\n--- boot log head ---\n'), text[:200]
        head = text.split('--- boot log head ---\n', 1)[1].split('\n--- end of boot log head ---', 1)[0]
        assert '[net.dhcp.lease]' in head or 'kernel' in head.lower(), head[:400]
        report['head_bytes'] = len(head)
        # Spindle's own lines from before the tether went up (its earlier df).
        until(r'MiB free of')
        # The host's automatic `hw`.
        until(r'remote> hw\n')
        until(r'lines from the first \d+ KiB of the boot log')
        g['wait'](r'\[spindle\.tether\.run\] len=2 ok=1', s, 60)

        r = host('send', 'df', '--wait', '8')
        assert 'remote> df' in r.stdout and 'MiB free of' in r.stdout, (r.stdout, r.stderr)
        r = host('send', 'install go', '--wait', '5')
        assert 'remote: refused' in r.stdout, r.stdout
        assert re.search(r'\[spindle\.tether\.refused\]', t()[s:]), 'refusal not logged'

        line('fetch http://10.0.2.2/x x.txt')
        until(r'The tether is using the network')

        s2 = len(t())
        line('tether off')
        g['wait'](r'\[spindle\.tether\.closed\] bytes=\d+ commands=3', s2, 60)
        serve.wait(60)
        assert re.search(r'disconnected after \d+ bytes', (a.out / 'tether-host.log').read_text())

        s3 = len(t())
        line('hw record')
        g['wait'](r'\[spindle\.hw\.record\.write\] n=1 file=hw-record-1\.txt', s3, 60)
        g['wait'](r'\[spindle\.hw\.record\.write\] n=2 file=hw-record-2\.txt', s3, 90)
        time.sleep(2)
        s4 = len(t())
        line('ls')
        time.sleep(3)
        listing = t()[s4:]
        assert 'hw-record-1.txt' in listing and 'hw-record-2.txt' in listing, listing[-600:]
        s4 = len(t())
        line('hw record off')
        g['wait'](r'\[spindle\.hw\.record\] on=0 writes=2', s4, 30)
        assert 'KERNEL PANIC' not in t()
    finally:
        g['stop']()
        serve.kill()
        shutil.copytree(logs, a.out / 'tether-logs', dirs_exist_ok=True)
    print({'ok': True, **report})


if __name__ == '__main__':
    main()
