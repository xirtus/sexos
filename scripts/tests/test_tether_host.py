# SPDX-License-Identifier: GPL-3.0-only
"""tether_host.py against a pretend SexOS: the stream is kept, auto and inbox commands go out."""
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

HOST = Path(__file__).resolve().parents[1] / 'tether_host.py'


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class TetherHost(unittest.TestCase):
    def test_sweep_down(self):
        d = Path(tempfile.mkdtemp(prefix='tether-sweep-'))
        port = free_port()
        serve = subprocess.Popen([sys.executable, str(HOST), '--dir', str(d), 'serve', '--port', str(port), '--once', '--quiet', '--auto', ''],
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            for _ in range(100):
                try:
                    s = socket.create_connection(('127.0.0.1', port), timeout=1)
                    break
                except OSError:
                    time.sleep(0.05)
            else:
                self.fail('serve never listened')
            s.sendall(b'SEXOS-SWEEP 1 trial=7 shape=down ba=on replay=on up=0 down=10000\n' + b'x' * 5000)
            s.settimeout(5)
            got = b''
            while len(got) < 10000:
                part = s.recv(65536)
                if not part:
                    break
                got += part
            s.close()
            serve.wait(timeout=10)
            self.assertEqual(len(got), 10000)
            self.assertTrue(got.startswith(b'd 000000000 '))
            kept = next(d.glob('session-*.log')).read_bytes()
            self.assertIn(b'trial=7 shape=down', kept)
            self.assertNotIn(b'xxxx', kept)
            self.assertIn(b'sweep session ended after', kept)
        finally:
            serve.kill()

    def test_session(self):
        d = Path(tempfile.mkdtemp(prefix='tether-test-'))
        port = free_port()
        serve = subprocess.Popen([sys.executable, str(HOST), '--dir', str(d), 'serve', '--port', str(port), '--once', '--quiet', '--auto', 'hw;crashes'],
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            for _ in range(100):
                try:
                    s = socket.create_connection(('127.0.0.1', port), timeout=1)
                    break
                except OSError:
                    time.sleep(0.05)
            else:
                self.fail('serve never listened')
            got = []

            def pretend():
                s.settimeout(0.2)
                s.sendall(b'SEXOS-TETHER 1\n--- boot log head ---\n[hw.census] devices=3\n')
                buf = b''
                end = time.time() + 6
                while time.time() < end and len(got) < 3:
                    try:
                        buf += s.recv(256)
                    except socket.timeout:
                        continue
                    while b'\n' in buf:
                        line, buf = buf.split(b'\n', 1)
                        got.append(line)
                        s.sendall(b'remote> ' + line + b'\nok ' + line + b'\n')
            t = threading.Thread(target=pretend)
            t.start()
            for _ in range(100):
                if b'"connected": true' in (d / 'state.json').read_bytes() if (d / 'state.json').exists() else False:
                    break
                time.sleep(0.05)
            out = subprocess.run([sys.executable, str(HOST), '--dir', str(d), 'send', 'hw', 'wifi', '--wait', '2'], capture_output=True, text=True)
            t.join()
            s.close()
            serve.wait(10)
            self.assertEqual(got, [b'hw', b'crashes', b'hw wifi'])
            self.assertIn('ok hw wifi', out.stdout)
            log = (d / 'current.log').read_text()
            self.assertTrue(log.startswith('SEXOS-TETHER 1\n--- boot log head ---\n[hw.census] devices=3\n'), log)
            self.assertIn('remote> crashes\nok crashes\n', log)
            tail = subprocess.run([sys.executable, str(HOST), '--dir', str(d), 'tail', '-n', '1'], capture_output=True, text=True)
            self.assertEqual(tail.stdout.strip(), 'ok hw wifi')
        finally:
            serve.kill()

    def test_send_without_session(self):
        d = Path(tempfile.mkdtemp(prefix='tether-test-'))
        r = subprocess.run([sys.executable, str(HOST), '--dir', str(d), 'send', 'hw'], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn('not connected', r.stderr)


if __name__ == '__main__':
    unittest.main()
