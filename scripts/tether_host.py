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
    # IPv6 too: a network whose router hands out IPv6 by itself but no IPv4 (SexOS's
    # Wi-Fi before DHCP works) is still reachable that way.
    for probe in ('2001:db8::1', 'fd00::1'):
        try:
            with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as s:
                s.connect((probe, 9))  # no packet is sent either
                a = s.getsockname()[0]
                if not a.lower().startswith('fe80') and a != '::1':
                    found.add(a)
        except OSError:
            pass
    return sorted(a for a in found if not a.startswith('127.'))


def scan_targets(base=None):
    """Every host address of this computer's /24 networks (or of BASE's)."""
    nets = set()
    for a in ([base] if base else addresses()):
        parts = a.split('.')
        if len(parts) == 4:
            nets.add('.'.join(parts[:3]))
    mine = set(addresses())
    return [f'{n}.{i}' for n in sorted(nets) for i in range(1, 255) if f'{n}.{i}' not in mine]


def find_listener(hosts, port, timeout=0.6):
    """The first host with PORT open, trying them all at once, and its connected socket.

    The socket is kept, not reconnected: SexOS opens its next listener only when Spindle
    polls ACCEPT, so a second connection right after this one can be refused."""
    socks = {}
    for h in hosts:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.setblocking(False)
        try:
            s.connect_ex((h, port))
        except OSError:
            s.close()
            continue
        socks[s] = h
    found = None
    end = time.time() + timeout * 3
    try:
        while socks and time.time() < end and not found:
            _, writable, _ = select.select([], list(socks), [], timeout)
            for s in writable:
                if s.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR) == 0:
                    found = (socks.pop(s), s)
                    break
                socks.pop(s).close()
    finally:
        for s in socks:
            s.close()
    if found:
        host, conn = found
        conn.setblocking(True)
        return host, conn
    return None


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
    try:
        # Both families on one socket: IPv4 and IPv6 (`tether [ADDRESS]:PORT` on SexOS).
        listener = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        listener.bind(('::', port))
    except OSError:
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(('0.0.0.0', port))
    listener.listen(1)
    shown = ', '.join(addresses()) or '(no network address found)'
    print(f'tether: waiting on port {port}. On SexOS, in Spindle: tether ADDRESS  (this computer: {shown})', flush=True)
    write_state(d, connected=False, port=port, waiting_since=time.time())
    while True:
        conn, peer = listener.accept()
        session(d, conn, peer[0], auto, quiet)
        if once:
            return


HOTSWAP_MAX_BYTES = 8 * 1024 * 1024


def fnv64(data: bytes) -> int:
    """64-bit FNV-1a checksum matching sex_pdx::crash_log::fnv64."""
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x00000100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def build_hotswap_frame(server_name: str, elf_bytes: bytes) -> bytes:
    """Build a framed SEXOS-HOTSWAP binary upload for Spindle."""
    if not server_name or any(ch.isspace() for ch in server_name):
        raise ValueError(f'invalid server name: {server_name!r}')
    if not elf_bytes or len(elf_bytes) > HOTSWAP_MAX_BYTES:
        raise ValueError(f'ELF size {len(elf_bytes)} out of bounds (1..={HOTSWAP_MAX_BYTES})')
    if not elf_bytes.startswith(b'\x7fELF'):
        raise ValueError('not an ELF binary (missing 0x7fELF magic)')
    header = f'SEXOS-HOTSWAP {server_name} {len(elf_bytes)} {fnv64(elf_bytes):016x}\n'.encode('ascii')
    return header + elf_bytes


PUSHFILE_MAX_BYTES = 16 * 1024 * 1024
PUSHFILE_MAX_PATH = 240


def build_pushfile_frame(remote_path: str, file_bytes: bytes) -> bytes:
    """Build a framed SEXOS-PUSHFILE binary upload for Spindle."""
    if not remote_path or any(ch.isspace() or ord(ch) < 0x21 or ord(ch) == 0x7f for ch in remote_path):
        raise ValueError(f'invalid remote path: {remote_path!r}')
    path_bytes = remote_path.encode('utf-8')
    if len(path_bytes) > PUSHFILE_MAX_PATH:
        raise ValueError(f'remote path too long ({len(path_bytes)} > {PUSHFILE_MAX_PATH})')
    if len(file_bytes) > PUSHFILE_MAX_BYTES:
        raise ValueError(f'file size {len(file_bytes)} exceeds limit ({PUSHFILE_MAX_BYTES})')
    header = f'SEXOS-PUSHFILE {remote_path} {len(file_bytes)} {fnv64(file_bytes):016x}\n'.encode('utf-8')
    return header + file_bytes


def sweep_reply(header: bytes) -> bytes:
    """The data a `SEXOS-SWEEP ... down=N` trial asks for: N bytes of 64-byte lines."""
    if not header.startswith(b'SEXOS-SWEEP'):
        return b''
    want = 0
    for word in header.split():
        if word.startswith(b'down=') and word[5:].isdigit():
            want = min(int(word[5:]), 64 * 1024 * 1024)
    out = bytearray()
    while len(out) < want:
        line = f'd {len(out):09} '.encode().ljust(63, b'.') + b'\n'
        out += line
    return bytes(out[:want])


def session(d: Path, conn, peer: str, auto: list, quiet: bool):
    """One SexOS session on a connected socket, whichever side connected."""
    inbox = d / 'inbox.txt'
    inbox.touch()
    stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
    path = d / f'session-{stamp}.log'
    current = d / 'current.log'
    try:
        if current.is_symlink() or current.exists():
            current.unlink()
        current.symlink_to(path.name)
    except OSError:
        (d / 'current.txt').write_text(path.name)
    print(f'tether: {peer} connected; keeping {path}', flush=True)
    inbox_at = inbox.stat().st_size
    received = 0
    started = time.time()
    write_state(d, connected=True, peer=peer, session=path.name, started=started, bytes=0)
    conn.setblocking(False)
    pending = b''.join(c.encode() + b'\n' for c in auto)
    # `tether sweep`: the first line names the trial; `down=N` asks for N bytes back.
    first = b''
    sweep = False
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
                if data and first is not None:
                    first += data
                    if b'\n' in first or len(first) > 4096:
                        header = first.split(b'\n', 1)[0]
                        reply = sweep_reply(header)
                        if header.startswith(b'SEXOS-SWEEP'):
                            # Test data is not kept: only the trial's line and its size.
                            sweep = True
                            log.write(header + b'\n')
                            log.flush()
                            print(f'tether: sweep {header.decode("utf-8", "replace")}', flush=True)
                        pending += reply
                        first = None
                if data and sweep:
                    received += len(data)
                elif data:
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
                for line in whole.splitlines(keepends=True):
                    if line.startswith(b'@HOTSWAP '):
                        rel = line[9:].strip().decode('utf-8', 'replace')
                        staged = (d / Path(rel).name)
                        try:
                            pending += staged.read_bytes()
                            staged.unlink(missing_ok=True)
                        except OSError as exc:
                            print(f'tether: hotswap read failed ({staged}): {exc}', file=sys.stderr, flush=True)
                    elif line.startswith(b'@PUSHFILE '):
                        rel = line[10:].strip().decode('utf-8', 'replace')
                        staged = (d / Path(rel).name)
                        try:
                            pending += staged.read_bytes()
                            staged.unlink(missing_ok=True)
                        except OSError as exc:
                            print(f'tether: push-file read failed ({staged}): {exc}', file=sys.stderr, flush=True)
                    else:
                        pending += line
            if time.time() - last_state > 1:
                last_state = time.time()
                write_state(d, connected=True, peer=peer, session=path.name, started=started, bytes=received)
    conn.close()
    if sweep:
        with path.open('ab') as log:
            log.write(f'sweep session ended after {received} bytes\n'.encode())
    print(f'\ntether: {peer} disconnected after {received} bytes', flush=True)
    write_state(d, connected=False, last_session=path.name, bytes=received, ended=time.time())


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


def push_server(d: Path, server_name: str, elf_path: Path, wait: float):
    """Push a compiled server ELF over the active tether and wait for Spindle's hotswap result."""
    state = read_state(d)
    if not state.get('connected'):
        print('tether: SexOS is not connected (run serve/connect, then `tether ADDRESS` in Spindle).', file=sys.stderr)
        return 2
    try:
        elf_bytes = elf_path.read_bytes()
        frame = build_hotswap_frame(server_name, elf_bytes)
    except (OSError, ValueError) as exc:
        print(f'tether: push-server failed: {exc}', file=sys.stderr)
        return 2
    log = current_log(d)
    start = log.stat().st_size if log else 0
    staged_name = f'hotswap-{int(time.time() * 1000)}-{os.getpid()}.bin'
    (d / staged_name).write_bytes(frame)
    with (d / 'inbox.txt').open('a') as f:
        f.write(f'@HOTSWAP {staged_name}\n')
    deadline = time.time() + wait
    at = start
    seen = ''
    while time.time() < deadline:
        time.sleep(0.15)
        if not log:
            log = current_log(d)
            continue
        size = log.stat().st_size
        if size > at:
            with log.open('rb') as f:
                f.seek(at)
                chunk = f.read(size - at).decode('utf-8', 'replace')
            sys.stdout.write(chunk)
            sys.stdout.flush()
            at = size
            seen += chunk
            if 'hotswap: ok ' in seen:
                return 0
            if 'hotswap: err ' in seen:
                return 1
    print('tether: timed out waiting for hotswap confirmation.', file=sys.stderr)
    return 1


def push_file(d: Path, local_path: Path, remote_path: str, wait: float):
    """Push any local file over the active tether to remote_path on SexOS."""
    state = read_state(d)
    if not state.get('connected'):
        print('tether: SexOS is not connected (run serve/connect, then `tether ADDRESS` in Spindle).', file=sys.stderr)
        return 2
    try:
        file_bytes = local_path.read_bytes()
        frame = build_pushfile_frame(remote_path, file_bytes)
    except (OSError, ValueError) as exc:
        print(f'tether: push-file failed: {exc}', file=sys.stderr)
        return 2
    log = current_log(d)
    start = log.stat().st_size if log else 0
    staged_name = f'pushfile-{int(time.time() * 1000)}-{os.getpid()}.bin'
    (d / staged_name).write_bytes(frame)
    with (d / 'inbox.txt').open('a') as f:
        f.write(f'@PUSHFILE {staged_name}\n')
    deadline = time.time() + wait
    at = start
    seen = ''
    while time.time() < deadline:
        time.sleep(0.15)
        if not log:
            log = current_log(d)
            continue
        size = log.stat().st_size
        if size > at:
            with log.open('rb') as f:
                f.seek(at)
                chunk = f.read(size - at).decode('utf-8', 'replace')
            sys.stdout.write(chunk)
            sys.stdout.flush()
            at = size
            seen += chunk
            if 'push-file: ok ' in seen:
                return 0
            if 'push-file: err ' in seen:
                return 1
    print('tether: timed out waiting for push-file confirmation.', file=sys.stderr)
    return 1


class PullStream:
    """Reassembles Spindle's `@@PULL-BEGIN / @@PULL-CHUNK / @@PULL-END` framed hex stream."""

    def __init__(self, expected_path=None):
        self.expected_path = expected_path
        self.started = False
        self.path = None
        self.expected_len = None
        self.buf = bytearray()
        self.fnv_hex = None
        self.done = False
        self.error = None

    def feed_line(self, line: str) -> bool:
        """Processes one line. Returns True if the line was a pull protocol frame."""
        line = line.rstrip('\r\n')
        if line.startswith('@@PULL-ERR '):
            self.error = line[11:].strip() or 'remote pull error'
            return True
        if line.startswith('@@PULL-BEGIN '):
            parts = line[13:].strip().split()
            if len(parts) != 2 or not parts[1].isdigit():
                self.error = f'malformed @@PULL-BEGIN: {line!r}'
                return True
            self.path = parts[0]
            self.expected_len = int(parts[1])
            self.buf = bytearray()
            self.started = True
            return True
        if line.startswith('@@PULL-CHUNK '):
            if not self.started:
                return True
            hex_data = line[13:].strip()
            try:
                self.buf.extend(bytes.fromhex(hex_data))
            except ValueError as exc:
                self.error = f'invalid hex chunk: {exc}'
            return True
        if line.startswith('@@PULL-END '):
            if not self.started:
                return True
            fnv_hex = line[11:].strip()
            try:
                want_fnv = int(fnv_hex, 16)
            except ValueError:
                self.error = f'invalid FNV in @@PULL-END: {fnv_hex!r}'
                return True
            if self.expected_len is not None and len(self.buf) != self.expected_len:
                self.error = f'size mismatch: got {len(self.buf)} bytes, expected {self.expected_len}'
                return True
            got_fnv = fnv64(bytes(self.buf))
            if got_fnv != want_fnv:
                self.error = f'checksum mismatch: got {got_fnv:016x}, expected {want_fnv:016x}'
                return True
            self.fnv_hex = f'{got_fnv:016x}'
            self.done = True
            return True
        return False


def pull_file(d: Path, remote_path: str, local_path: Path, wait: float):
    """Pull a file from SexOS over the active tether via Spindle's `pull-file` command."""
    state = read_state(d)
    if not state.get('connected'):
        print('tether: SexOS is not connected (run serve/connect, then `tether ADDRESS` in Spindle).', file=sys.stderr)
        return 2
    if not remote_path or any(ch.isspace() or ord(ch) < 0x21 or ord(ch) == 0x7f for ch in remote_path):
        print(f'tether: invalid remote path: {remote_path!r}', file=sys.stderr)
        return 2
    log = current_log(d)
    start = log.stat().st_size if log else 0
    with (d / 'inbox.txt').open('a') as f:
        f.write(f'pull-file {remote_path}\n')
    stream = PullStream(expected_path=remote_path)
    deadline = time.time() + wait
    at = start
    carry = ''
    while time.time() < deadline:
        time.sleep(0.1)
        if not log:
            log = current_log(d)
            continue
        size = log.stat().st_size
        if size > at:
            with log.open('rb') as f:
                f.seek(at)
                chunk = f.read(size - at).decode('utf-8', 'replace')
            at = size
            carry += chunk
            while '\n' in carry:
                line, carry = carry.split('\n', 1)
                if stream.feed_line(line):
                    deadline = max(deadline, time.time() + wait)
                if stream.error:
                    print(f'tether: pull failed: {stream.error}', file=sys.stderr)
                    return 1
                if stream.done:
                    try:
                        local_path.parent.mkdir(parents=True, exist_ok=True)
                        local_path.write_bytes(bytes(stream.buf))
                    except OSError as exc:
                        print(f'tether: could not write {local_path}: {exc}', file=sys.stderr)
                        return 1
                    print(f'tether: pulled {remote_path} -> {local_path} ({len(stream.buf)} bytes, fnv={stream.fnv_hex})', flush=True)
                    return 0
    print('tether: timed out waiting for pull-file stream.', file=sys.stderr)
    return 1


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
    ps = sub.add_parser('push-server', help='live hot-swap a running SexOS server ELF over the active tether')
    ps.add_argument('server_name', help='target server name (e.g. sexbell, sexinput, sexacpi, cufflinks, sexdisplay)')
    ps.add_argument('elf_path', type=Path, help='path to compiled PIE ELF binary')
    ps.add_argument('--wait', type=float, default=10.0, help='seconds to wait for hotswap result')
    pf = sub.add_parser('push-file', help='push a local file onto SexOS over the active tether')
    pf.add_argument('local_path', type=Path, help='path to local file to upload')
    pf.add_argument('remote_path', help='destination path on SexOS (e.g. /media/Ventoy/moss.wav)')
    pf.add_argument('--wait', type=float, default=30.0, help='seconds to wait for push-file confirmation')
    pl = sub.add_parser('pull', aliases=['pull-file'], help='pull a remote file from SexOS over the active tether')
    pl.add_argument('remote_path', help='source path on SexOS (e.g. /data/screen.png)')
    pl.add_argument('local_path', type=Path, help='local destination path on this computer')
    pl.add_argument('--wait', type=float, default=30.0, help='idle timeout in seconds while waiting for pull chunks')
    t = sub.add_parser('tail')
    t.add_argument('-n', type=int, default=80)
    sub.add_parser('status')
    k = sub.add_parser('connect', help='connect to SexOS started with `tether listen` (the "report to the Mac" boot entries)')
    k.add_argument('host', nargs='?', help='its address; with --scan, found on the local network')
    k.add_argument('--scan', action='store_true', help='try every address of this computer\'s /24 networks')
    k.add_argument('--port', type=int, default=PORT)
    k.add_argument('--auto', default='hw;crashes;df')
    k.add_argument('--quiet', action='store_true')
    k.add_argument('--retry', type=float, default=600, help='seconds to keep looking')
    a = p.parse_args(argv)
    if a.what in (None, 'serve'):
        port = getattr(a, 'port', PORT)
        auto = [x.strip() for x in getattr(a, 'auto', 'hw;crashes;df').split(';') if x.strip()]
        serve(a.dir, port, auto, getattr(a, 'once', False), getattr(a, 'quiet', False))
        return 0
    if a.what == 'connect':
        auto = [x.strip() for x in a.auto.split(';') if x.strip()]
        a.dir.mkdir(parents=True, exist_ok=True)
        end = time.time() + a.retry
        while True:
            hosts = [a.host] if a.host and not a.scan else scan_targets(a.host)
            found = find_listener(hosts, a.port)
            if found:
                host, conn = found
                print(f'tether: found SexOS at {host}:{a.port}', flush=True)
                session(a.dir, conn, host, auto, a.quiet)
                return 0
            if time.time() > end:
                print('tether: no SexOS listener found (is the x17 on the same network, booted with "report to the Mac"?)', file=sys.stderr)
                return 1
            print(f'tether: not found yet among {len(hosts)} addresses; trying again', flush=True)
            time.sleep(5)
    if a.what == 'send':
        return send(a.dir, ' '.join(a.command), a.wait)
    if a.what == 'push-server':
        return push_server(a.dir, a.server_name, a.elf_path, a.wait)
    if a.what == 'push-file':
        return push_file(a.dir, a.local_path, a.remote_path, a.wait)
    if a.what in ('pull', 'pull-file'):
        return pull_file(a.dir, a.remote_path, a.local_path, a.wait)
    if a.what == 'tail':
        return tail(a.dir, a.n)
    print(json.dumps(read_state(a.dir), indent=1))
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        pass
