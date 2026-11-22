#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""The other end of Spindle's `tether`: watch and drive a SexOS machine from this computer.

On SexOS (Spindle, Scroll Lock):   tether 192.168.1.20      (this computer's address)

Here:
  python3 scripts/tether_host.py                  serve: wait for SexOS, keep everything it sends
  python3 scripts/tether_host.py send hw wifi     run `hw wifi` there and print what came back
  python3 scripts/tether_host.py tail [-n 80]     the newest lines of the current session
  python3 scripts/tether_host.py status           connected or not, bytes so far

`serve` prints this computer's addresses, then waits on TCP port 7177. A session is kept
as DIR/session-DATE.log (DIR defaults to ./tether-logs; DIR/current.log names the newest):
the kernel's boot log head first, then every line Spindle shows. Commands for SexOS are
lines appended to DIR/inbox.txt (which `send` does); each runs there as if typed and shows
on that screen as `remote> LINE`. When SexOS connects, the `--auto` commands run first
(default: hw, crashes, df).

The connection is plain TCP on the local network, made from SexOS to here; nothing on
SexOS listens. On a Mac, allow Python in the firewall prompt the first time.
"""
import argparse
import datetime
import json
import os
import select
import socket
import sys
import time
from pathlib import Path

PORT = 7177


def addresses():
    found = set()
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.connect(('192.0.2.1', 9))  # no packet is sent; this picks the outward address
            found.add(s.getsockname()[0])
    except OSError:
        pass
    try:
        for info in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET):
            found.add(info[4][0])
    except OSError:
        pass
    return sorted(a for a in found if not a.startswith('127.'))


def write_state(d: Path, **state):
    tmp = d / 'state.json.tmp'
    tmp.write_text(json.dumps(state))
    tmp.replace(d / 'state.json')


def read_state(d: Path):
    try:
        return json.loads((d / 'state.json').read_text())
    except (OSError, ValueError):
        return {'connected': False}


def serve(d: Path, port: int, auto: list, once: bool, quiet: bool):
    d.mkdir(parents=True, exist_ok=True)
    inbox = d / 'inbox.txt'
    inbox.touch()
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(('0.0.0.0', port))
    listener.listen(1)
    shown = ', '.join(addresses()) or '(no network address found)'
    print(f'tether: waiting on port {port}. On SexOS, in Spindle: tether ADDRESS  (this computer: {shown})', flush=True)
    write_state(d, connected=False, port=port, waiting_since=time.time())
    while True:
        conn, peer = listener.accept()
        stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
        path = d / f'session-{stamp}.log'
        current = d / 'current.log'
        try:
            if current.is_symlink() or current.exists():
                current.unlink()
            current.symlink_to(path.name)
        except OSError:
            (d / 'current.txt').write_text(path.name)
        print(f'tether: {peer[0]} connected; keeping {path}', flush=True)
        inbox_at = inbox.stat().st_size
        received = 0
        started = time.time()
        write_state(d, connected=True, peer=peer[0], session=path.name, started=started, bytes=0)
        conn.setblocking(False)
        pending = b''.join(c.encode() + b'\n' for c in auto)
        last_state = 0.0
        with path.open('ab') as log:
            while True:
                readable, writable, _ = select.select([conn], [conn] if pending else [], [], 0.2)
                if readable:
                    try:
                        data = conn.recv(65536)
                    except (BlockingIOError, InterruptedError):
                        data = None
                    except OSError:
                        data = b''
                    if data == b'':
                        break
                    if data:
                        log.write(data)
                        log.flush()
                        received += len(data)
                        if not quiet:
                            sys.stdout.write(data.decode('utf-8', 'replace'))
                            sys.stdout.flush()
                if writable and pending:
                    try:
                        sent = conn.send(pending)
                        pending = pending[sent:]
                    except (BlockingIOError, InterruptedError):
                        pass
                    except OSError:
                        break
                size = inbox.stat().st_size
                if size < inbox_at:
                    inbox_at = 0
                if size > inbox_at:
                    with inbox.open('rb') as f:
                        f.seek(inbox_at)
                        new = f.read()
                    whole = new[:new.rfind(b'\n') + 1]
                    inbox_at += len(whole)
                    pending += whole
                if time.time() - last_state > 1:
                    last_state = time.time()
                    write_state(d, connected=True, peer=peer[0], session=path.name, started=started, bytes=received)
        conn.close()
        print(f'\ntether: {peer[0]} disconnected after {received} bytes', flush=True)
        write_state(d, connected=False, port=port, last_session=path.name, bytes=received, ended=time.time())
        if once:
            return


def current_log(d: Path):
    c = d / 'current.log'
    if c.exists():
        return c
    t = d / 'current.txt'
    if t.exists():
        return d / t.read_text().strip()
    return None


def send(d: Path, command: str, wait: float):
    state = read_state(d)
    if not state.get('connected'):
        print('tether: SexOS is not connected (run serve, then `tether ADDRESS` in Spindle).', file=sys.stderr)
        return 2
    log = current_log(d)
    start = log.stat().st_size if log else 0
    with (d / 'inbox.txt').open('a') as f:
        f.write(command.replace('\n', ' ') + '\n')
    deadline = time.time() + wait
    at = start
    while time.time() < deadline:
        time.sleep(0.25)
        if not log:
            log = current_log(d)
            continue
        size = log.stat().st_size
        if size > at:
            with log.open('rb') as f:
                f.seek(at)
                sys.stdout.write(f.read(size - at).decode('utf-8', 'replace'))
                sys.stdout.flush()
            at = size
    return 0


def tail(d: Path, n: int):
    log = current_log(d)
    if not log:
        print('tether: no session yet.', file=sys.stderr)
        return 1
    lines = log.read_bytes().decode('utf-8', 'replace').splitlines()
    print('\n'.join(lines[-n:]))
    return 0


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--dir', type=Path, default=Path(os.environ.get('SEXOS_TETHER_DIR', 'tether-logs')))
    sub = p.add_subparsers(dest='what')
    s = sub.add_parser('serve')
    s.add_argument('--port', type=int, default=PORT)
    s.add_argument('--auto', default='hw;crashes;df', help='commands run when SexOS connects, ;-separated')
    s.add_argument('--once', action='store_true', help='end after the first session')
    s.add_argument('--quiet', action='store_true', help='keep the stream out of this terminal')
    c = sub.add_parser('send')
    c.add_argument('command', nargs='+')
    c.add_argument('--wait', type=float, default=6.0, help='seconds to print what comes back')
    t = sub.add_parser('tail')
    t.add_argument('-n', type=int, default=80)
    sub.add_parser('status')
    a = p.parse_args(argv)
    if a.what in (None, 'serve'):
        port = getattr(a, 'port', PORT)
        auto = [x.strip() for x in getattr(a, 'auto', 'hw;crashes;df').split(';') if x.strip()]
        serve(a.dir, port, auto, getattr(a, 'once', False), getattr(a, 'quiet', False))
        return 0
    if a.what == 'send':
        return send(a.dir, ' '.join(a.command), a.wait)
    if a.what == 'tail':
        return tail(a.dir, a.n)
    print(json.dumps(read_state(a.dir), indent=1))
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        pass
