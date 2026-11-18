#!/usr/bin/env python3
"""Native Voice fetches a fixture WAV over the optional network grant and saves it."""
import argparse
import http.server
import os
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}


class ProgramServer(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != '/voice':
            self.send_error(404)
            return
        body = (ROOT / 'iso_root/programs/voice').read_bytes()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def check_worker_ranges(port):
    """Exercise the same HTTP frame and range endpoints before starting QEMU."""
    base = f'http://127.0.0.1:{port}'
    frame = (b'SXVC' + bytes((1, 1, 0, 0)) + struct.pack('<Q', 0x56)
             + struct.pack('<B3xQQQIBBH', 2, 0, 0, 0, 4_000_000, 2, 6, 5)
             + b'enactivehello')
    for _ in range(100):
        try:
            with urllib.request.urlopen(urllib.request.Request(base + '/v1/frame', data=frame), timeout=2) as reply:
                assert reply.status == 200 and reply.read()[5] == 4
            break
        except urllib.error.URLError:
            time.sleep(.05)
    else:
        raise RuntimeError('fixture voice worker did not start')
    for _ in range(2):
        with urllib.request.urlopen(urllib.request.Request(base + '/v1/frame',
                                      data=frame[:5] + bytes((3,)) + frame[6:16]), timeout=2) as reply:
            reply.read()
    request = urllib.request.Request(base + '/v1/assets/86', headers={'Range': 'bytes=0-31'})
    with urllib.request.urlopen(request, timeout=2) as reply:
        data = reply.read()
        assert reply.status == 206 and reply.headers['Content-Range'] == 'bytes 0-31/52'
        assert len(data) == 32 and data.startswith(b'RIFF')
    request = urllib.request.Request(base + '/v1/assets/86', headers={'Range': 'bytes=32-63'})
    with urllib.request.urlopen(request, timeout=2) as reply:
        assert reply.status == 206 and reply.headers['Content-Range'] == 'bytes 32-51/52'
        assert len(reply.read()) == 20


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    if any(args.out.iterdir()):
        raise RuntimeError('fresh evidence directory required')
    disk = args.out / 'nvme.img'
    shutil.copyfile(args.source_disk.resolve(strict=True), disk)
    with disk.open('r+b') as file:
        file.truncate(1024**3)

    program_server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), ProgramServer)
    threading.Thread(target=program_server.serve_forever, daemon=True).start()
    voice_port = free_port()
    worker_env = os.environ.copy()
    worker_env.pop('MOSS_VOICE_TOKEN', None)
    worker = subprocess.Popen([sys.executable, str(ROOT / 'crates/sex-voice/adapters/mosslanding.py'),
                               '--fixture', '--port', str(voice_port)],
                              stdout=(args.out / 'voice-worker.log').open('w'), stderr=subprocess.STDOUT,
                              env=worker_env)
    os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'
    from quil_autosave_gate import boot

    guest = None
    try:
        check_worker_ranges(voice_port)
        guest = boot(args.iso.resolve(strict=True), disk, args.out, 'voice', '1280x800')
        text, q = guest['text'], guest['q']

        def keys(line):
            for char in line:
                if char == ':':
                    q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'semicolon'}], 'hold-time': 80})
                elif char == '+':
                    q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': 'equal'}], 'hold-time': 80})
                else:
                    q.key(KEYS.get(char, char))
                time.sleep(.04)

        def line(command, receipt, timeout=120):
            guest['focus'](153)
            time.sleep(.3)
            start = len(text())
            keys(command)
            q.key('ret')
            return guest['wait'](receipt, start, timeout)

        guest['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        source = f'http://10.0.2.2:{program_server.server_address[1]}/voice'
        line(f'fetch {source} /bin/voice', r'\[spindle.fetch.line\] Saved \d+ bytes', 240)
        endpoint = f'http://10.0.2.2:{voice_port}'
        start = len(text())
        line(f'run +net voice {endpoint}', r'\[voice.ready\]', 180)
        launch = guest['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)', start, 60)
        guest['focus'](int(launch[3]))
        time.sleep(.5)
        keys('hello')
        q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'ctrl'}, {'type': 'qcode', 'data': 'g'}], 'hold-time': 80})
        guest['wait'](r'\[voice.saved\] job=\d+ bytes=52 rate=8000 channels=1 frames=4', start, 180)
        q.key('f11')
        guest['wait'](rf'\[app.private.retired\] target={launch[1]} task={launch[2]} ', start, 60)
        line('ls /data/voice', r'\[spindle.fs.line\] take-\d+\.wav  52')
        line('ls /data/voice', r'\[spindle.fs.line\] take-\d+\.sxvc')
        line('ls /data/voice', r'\[spindle.fs.line\] take-\d+\.txt  5')
        if 'KERNEL PANIC' in text() or 'panicked' in text():
            raise RuntimeError('guest panic')
        guest['stop']()
        guest = boot(args.iso.resolve(strict=True), disk, args.out, 'voice-reboot', '1280x800')
        text, q = guest['text'], guest['q']
        guest['wait'](r'\[app.private.retired\] target=41 ', 0, 120)
        line('ls /data/voice', r'\[spindle.fs.line\] take-\d+\.wav  52')
        line('ls /data/voice', r'\[spindle.fs.line\] take-\d+\.sxvc')
        line('ls /data/voice', r'\[spindle.fs.line\] take-\d+\.txt  5')
        print({'ok': True, 'voice_bytes': 52, 'files': 3, 'reboot': True})
    finally:
        if guest is not None:
            guest['stop']()
        worker.terminate()
        worker.wait(timeout=5)
        program_server.shutdown()


if __name__ == '__main__':
    main()
