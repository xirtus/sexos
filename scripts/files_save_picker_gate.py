#!/usr/bin/env python3
"""Save through the Files picker: Jot asks to save (Ctrl+Shift+S), the person names a
new file in Files (Ctrl+S, a name, Enter), Files grants Jot that one path, Jot writes
it there and can save to it again; a cancelled save leaves nothing behind and Jot
still holds only the file it was given. Spindle reads the saved bytes back."""
import argparse, http.server, os, re, shutil, threading, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
# Files is staged as Loom; Spindle's `files` runs /bin/loom.
FILES = {'/files': (ROOT / 'iso_root/programs/loom').read_bytes(), '/jot': (ROOT / 'iso_root/programs/jot').read_bytes()}


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = FILES.get(self.path)
        if body is None:
            self.send_response(404); self.end_headers(); return
        self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)

    def log_message(self, *a):
        pass


class Quiet(http.server.ThreadingHTTPServer):
    daemon_threads = True


server = Quiet(('127.0.0.1', 0), Handler)
PORT = server.server_address[1]
threading.Thread(target=server.serve_forever, daemon=True).start()
os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot  # noqa: E402
KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}
FIRST = 'saved through the picker'
MORE = ' twice'


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path); p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path); p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    shutil.rmtree(a.out, ignore_errors=True); a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f:
        f.truncate(1024**3)
    g = boot(a.iso, disk, a.out, 'savepick', a.resolution); t = g['text']; q = g['q']

    def keys(text):
        for k in text:
            if k.isupper():
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': k.lower()}], 'hold-time': 80})
            else:
                q.key(KEYS.get(k, k))
            time.sleep(.06)

    def line(text): g['focus'](153); time.sleep(.5); keys(text); q.key('ret')
    def expect(text, pattern, timeout=60): s = len(t()); line(text); return g['wait'](pattern, s, timeout)
    def chord(key, *mods): q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': m} for m in ('ctrl',) + mods] + [{'type': 'qcode', 'data': key}], 'hold-time': 80})
    def launched(s): return g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)', s, 60)
    result = {}
    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        for n, dest in (('files', '/bin/loom'), ('jot', '/bin/jot')):
            m = expect(f'fetch http://10.0.2.2:{PORT}/{n} {dest}', r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)', 240); assert m[1].startswith('Saved'), m[1]
        expect('mkdir /Saves', r'\[spindle.fs.result\] kind=3 status=0')
        # Files, given the whole filesystem, answers the questions.
        s = len(t()); line('files'); m = launched(s); fa_target, fa_sid = m[1], int(m[3])
        g['wait'](r'\[files.app.ready\] target=\d+ task=\d+ root=/ portal=1', s, 60)
        g['wait'](rf'\[display.text.committed\] target={fa_target} ', s, 60); time.sleep(1)
        # Jot, holding nothing but its own folder, types and asks to save.
        s = len(t()); line('run jot'); m = launched(s); jot_sid = int(m[3]); g['wait'](r'\[jot.ready\]', s, 60); time.sleep(2)
        g['focus'](jot_sid); time.sleep(.5); keys(FIRST); time.sleep(.5)
        s = len(t()); chord('s', 'shift')
        g['wait'](r'\[files.pick.asked\] row=\d+ mode=1 ', s, 30)
        g['wait'](r'\[files.app.question\] program=jot mode=Save', s, 30)
        # In Files (at /): Ctrl+S names the file; the suggestion is cleared first.
        g['focus'](fa_sid); time.sleep(.5)
        chord('s'); time.sleep(.5)
        for _ in range(40):
            q.key('backspace'); time.sleep(.02)
        keys('Saves/picked.txt'); time.sleep(.3)
        q.cmd('screendump', {'filename': str(a.out / 'files-save-picker.ppm')}); time.sleep(.5)
        s2 = len(t()); q.key('ret')
        g['wait'](r'\[files.pick.granted\] row=\d+ path=/Saves/picked.txt', s2, 30)
        g['wait'](r'\[files.app.answered\] program=jot path=/Saves/picked.txt', s2, 30)
        g['wait'](r'\[jot.picked\] path=@/Saves/picked.txt', s2, 30)
        m = g['wait'](r'\[jot.saved\] bytes=(\d+) ', s2, 60)
        result['first_bytes'] = int(m[1])
        assert int(m[1]) in (len(FIRST), len(FIRST) + 1), m[0]
        expect('cat /Saves/picked.txt', rf'\[spindle.fs.cat\] {FIRST}')
        # The granted path stays Jot's: Ctrl+S saves to it again.
        g['focus'](jot_sid); time.sleep(.5); keys(MORE); time.sleep(.3)
        s = len(t()); chord('s'); m = g['wait'](r'\[jot.saved\] bytes=(\d+) ', s, 60)
        assert int(m[1]) in (len(FIRST + MORE), len(FIRST + MORE) + 1), m[0]
        expect('cat /Saves/picked.txt', rf'\[spindle.fs.cat\] {FIRST + MORE}')
        # A cancelled save: Esc in Files, Jot hears no, nothing new appears.
        g['focus'](jot_sid); time.sleep(.5)
        s = len(t()); chord('s', 'shift')
        g['wait'](r'\[files.app.question\] program=jot mode=Save', s, 30)
        g['focus'](fa_sid); time.sleep(.5)
        s = len(t()); q.key('esc')
        g['wait'](r'\[files.pick.cancelled\] row=\d+', s, 30)
        g['wait'](r'\[jot.pick.refused\] status=\d+', s, 30)
        s = len(t()); line('ls /Saves'); g['wait'](r'\[spindle.fs.result\] kind=2 status=0', s, 60); time.sleep(1)
        listed = re.findall(r'\[spindle.fs.line\] ([^\n]*)', t()[s:])
        assert [x.split('  ')[0] for x in listed if x.strip()] == ['picked.txt'], listed
        result['listed'] = listed
        text = t()
        assert 'KERNEL PANIC' not in text and 'panicked' not in text and '[files.app.fatal]' not in text
    finally:
        g['stop']()
    result['ok'] = True; print(result)


if __name__ == '__main__':
    main()
