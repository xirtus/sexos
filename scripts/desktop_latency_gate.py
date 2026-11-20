#!/usr/bin/env python3
"""Desktop latency: how long everyday interactions take, end to end, measured from the
host's input event to the serial marker that says the result reached the screen.

  key      a key typed into Spindle -> Display commits Spindle's new text
  launch   Enter on `run NAME` in Spindle -> Shell opens the window -> Display shows
           the program's first pixels -> the program says it is ready
  resize   one mouse move while dragging Quil's corner -> Shell applies the new size
           -> Display applies Quil's next paint
  open     Enter on a text file in Files -> Files asks Shell -> Jot's window opens ->
           Jot has the bytes -> Jot's first pixels

First pixels is Display's answer to the program's present request
([display.app.present.reply]), sent once a published frame shows the window.

Every serial line is stamped with the host time it arrived, so each interaction also
gets a stage-by-stage breakdown (median ms from the input to each marker), and the
first few are printed whole as traces. With /dev/kvm the guest runs on KVM with
the host CPU and 4 cores, the configuration the plan's targets are written for;
without it the numbers come from QEMU's software CPU and are only comparable with
themselves. Results go to latency.json and the job log. The gate fails on a fault, a
missing marker, or a p50 above the generous --max-* bounds (regressions are judged
from the printed numbers)."""
import argparse, http.server, json, os, re, shutil, statistics, threading, time
from pathlib import Path

KVM = os.access('/dev/kvm', os.R_OK | os.W_OK)
if KVM:
    os.environ.setdefault('SEXOS_GATE_ACCEL', 'kvm')
    os.environ.setdefault('SEXOS_GATE_CPU', 'host')
os.environ.setdefault('SEXOS_GATE_CORES', '4' if KVM else '1')
os.environ.setdefault('SEXOS_GATE_MEMORY', '1G')
os.environ.setdefault('SEXOS_GATE_BOUND', '2400')
ROOT = Path(__file__).resolve().parents[1]

# Files is staged as Loom; Spindle's `files` runs /bin/loom. Jot must be in /bin for
# Files to open a text file with it. Both are served to the guest's `fetch`.
FILES = {'/files': (ROOT / 'iso_root/programs/loom').read_bytes(), '/jot': (ROOT / 'iso_root/programs/jot').read_bytes()}
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = FILES.get(self.path)
        if body is None: self.send_response(404); self.end_headers(); return
        self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
class Quiet(http.server.ThreadingHTTPServer): daemon_threads = True
server = Quiet(('127.0.0.1', 0), Handler); PORT = server.server_address[1]
threading.Thread(target=server.serve_forever, daemon=True).start()
os.environ['SEXOS_GATE_NIC'] = '-netdev user,id=n0 -device e1000e,netdev=n0'

from install_program import install
from quil_autosave_gate import boot

SPINDLE = 153
KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus', ':': 'semicolon'}
LAUNCH_TAG = {'gestalt': 'shapes.ready', 'mesh': 'monitor.ready'}


class Trace:
    """Serial lines with the host time each one arrived. Reads only what is new."""
    def __init__(self, path):
        self.path = path; self.f = None; self.buf = b''; self.lines = []; self.t0 = 0.0
    def start(self):
        if self.f is None: self.f = open(self.path, 'rb')
        self.f.seek(0, 2); self.buf = b''; self.lines = []; self.t0 = time.monotonic()
        return self.t0
    def poll(self):
        new = self.f.read()
        if new:
            now = time.monotonic(); self.buf += new
            *whole, self.buf = self.buf.split(b'\n')
            self.lines += [(now, l.decode('utf-8', 'replace').rstrip('\r')) for l in whole]
    def until(self, pattern, timeout=30.0, after=0):
        """(ms since start, match, index) of the first line from index `after` matching."""
        rx = re.compile(pattern); end = time.monotonic() + timeout; i = after
        while True:
            self.poll()
            while i < len(self.lines):
                at, line = self.lines[i]
                m = rx.search(line)
                if m: return (at - self.t0) * 1000, m, i
                i += 1
            if time.monotonic() > end: raise TimeoutError(f'no {pattern!r} within {timeout}s')
            time.sleep(0.0003)
    def soon(self, pattern, timeout, after=0):
        try: return self.until(pattern, timeout, after)
        except TimeoutError: return None, None, after
    def stages(self):
        """First arrival (ms) of each marker name in this window."""
        first = {}
        for at, line in self.lines:
            m = re.match(r'\s*\[([A-Za-z0-9_.:-]+)\]', line)
            if m and m[1] not in first: first[m[1]] = round((at - self.t0) * 1000, 2)
        return first
    def dump(self, title, limit=80):
        print(f'--- trace: {title} ({len(self.lines)} lines) ---')
        for at, line in self.lines[:limit]: print(f'{(at - self.t0) * 1000:9.2f} ms  {line[:200]}')


def pct(xs, p):
    xs = sorted(x for x in xs if x is not None)
    return round(xs[min(len(xs) - 1, int(round(p * (len(xs) - 1))))], 2) if xs else None


def summary(xs):
    xs = [x for x in xs if x is not None]
    return {'n': len(xs), 'p50_ms': pct(xs, .5), 'p90_ms': pct(xs, .9), 'max_ms': round(max(xs), 2) if xs else None}


def merged_stages(runs):
    """Median arrival of each marker over the runs it appeared in, in order."""
    names = {}
    for st in runs:
        for k, v in st.items(): names.setdefault(k, []).append(v)
    out = {k: round(statistics.median(v), 2) for k, v in names.items() if len(v) * 2 >= len(runs)}
    return dict(sorted(out.items(), key=lambda kv: kv[1]))


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path); p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path); p.add_argument('--resolution', default='1280x800')
    p.add_argument('--keys', type=int, default=40); p.add_argument('--runs', type=int, default=4)
    p.add_argument('--programs', default='gestalt,jot')
    p.add_argument('--max-key-ms', type=float, default=250.0); p.add_argument('--max-launch-ms', type=float, default=15000.0)
    p.add_argument('--key-inject', choices=('send-key', 'event'), default='event',
                   help='key phase: input-send-event press, release after the commit (default); QMP send-key adds ~35 ms of its own queueing before the guest sees the key')
    p.add_argument('--skip', default='', help='comma-separated phases to skip: key,launch,resize,open')
    a = p.parse_args()
    skip = set(filter(None, a.skip.split(',')))
    shutil.rmtree(a.out, ignore_errors=True); a.out.mkdir(parents=True, exist_ok=True)
    disk = a.out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f: f.truncate(max(f.seek(0, 2), 1024 ** 3))
    programs = [n for n in a.programs.split(',') if n]
    install(disk, [(n, (ROOT / 'iso_root/programs' / n).read_bytes()) for n in programs])
    g = boot(a.iso, disk, a.out, 'latency', a.resolution); t = g['text']; q = g['q']
    tr = Trace(a.out / 'latency-serial.log')
    res = {'accel': 'kvm' if KVM else 'tcg', 'cores': os.environ['SEXOS_GATE_CORES'], 'cpu': os.environ.get('SEXOS_GATE_CPU', '')}
    if KVM:
        try: res['host_cpu'] = next(l.split(':', 1)[1].strip() for l in open('/proc/cpuinfo') if l.startswith('model name'))
        except Exception: pass

    def keys(text):
        for k in text:
            if k.isupper() or k == ':':
                q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': KEYS.get(k, k.lower())}], 'hold-time': 80})
            else: q.key(KEYS.get(k, k))
            time.sleep(.06)

    def line(text, enter=True):
        g['focus'](SPINDLE); time.sleep(.5); keys(text)
        if enter: q.key('ret')

    def expect(text, pattern, timeout=60):
        s = len(t()); line(text); return g['wait'](pattern, s, timeout)

    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(3)

        # ── key to pixel ──
        if 'key' not in skip:
            g['focus'](SPINDLE); time.sleep(1)
            ms, st = [], []
            for i in range(a.keys):
                ch = 'abcdefghijklmnopqrstuvwxyz'[i % 26]
                ev = lambda down: q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': ch}}}]})
                tr.start()
                if a.key_inject == 'event': ev(True)
                else: q.key(ch)
                d, _, _ = tr.until(r'\[display\.text\.committed\] target=12 ', 10)
                if a.key_inject == 'event': ev(False)
                time.sleep(.03); tr.poll(); ms.append(d); st.append(tr.stages())
                if i in (2, 3): tr.dump(f'key {i}')
                time.sleep(.08)
            for _ in range(a.keys): q.key('backspace'); time.sleep(.02)
            res['key'] = {**summary(ms), 'stages': merged_stages(st), 'all_ms': [round(x, 1) for x in ms]}
            print('[latency.key]', json.dumps(res['key']))

        # ── program launch: window opens, first pixels, ready ──
        if 'launch' not in skip:
            res['launch'] = {}
            for name in programs:
                tag = LAUNCH_TAG.get(name, name + '.ready'); rows = []; st = []
                for run in range(a.runs):
                    cmd = f'run {name}'; s0 = len(t()); line(cmd, enter=False)
                    # QMP queues each key behind the last one's hold time: wait until
                    # Spindle has the whole line, so Enter is not timed behind typing.
                    g['wait'](rf'\[spindle\.line\.cursor\] pos={len(cmd)} len={len(cmd)}\b', s0, 30); time.sleep(.5)
                    tr.start(); q.key('ret')
                    win, m, i = tr.until(r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', 120)
                    pd, task, surface = m[1], m[2], int(m[3])
                    first, fm, _ = tr.soon(rf'\[display\.[a-z_.]+\] target={pd} ', 60, i)
                    # Display answers a program's present request once a published
                    # frame shows it: the window's first pixels on screen.
                    shown, _, _ = tr.soon(rf'\[display\.app\.present\.reply\] target={pd} ', 60, i)
                    ready, _, _ = tr.soon(rf'\[{re.escape(tag)}\] ', 60)
                    time.sleep(.3); tr.poll()
                    rows.append({'window_ms': round(win, 1), 'display_first_ms': first and round(first, 1),
                                 'first_marker': fm and fm[0].split(']')[0] + ']', 'first_pixels_ms': shown and round(shown, 1), 'ready_ms': ready and round(ready, 1)})
                    st.append(tr.stages())
                    if run in (0, 1): tr.dump(f'launch {name} run {run}', 120)
                    time.sleep(1.5); s = len(t())
                    g['focus'](surface); time.sleep(.3); q.key('f11')
                    g['wait'](rf'\[app.private.retired\] target={pd} task={task} ', s, 90); time.sleep(.5)
                res['launch'][name] = {'window': summary([r['window_ms'] for r in rows]),
                                       'first_pixels': summary([r['first_pixels_ms'] for r in rows]),
                                       'ready': summary([r['ready_ms'] for r in rows]),
                                       'runs': rows, 'stages': merged_stages(st)}
                print(f'[latency.launch] {name}', json.dumps(res['launch'][name]))

        # ── window resize by dragging Quil's corner ──
        if 'resize' not in skip:
            def at(x, y): q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': int(x * 32767 / 1280)}}, {'type': 'abs', 'data': {'axis': 'y', 'value': int(y * 32767 / 800)}}]})
            def btn(down): q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
            # A corner drag starts only on the focused window.
            g['focus'](201); time.sleep(2)
            x, y, w, h = map(int, re.findall(r'\[shell.tile.apply\] sid=201 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)', t())[-1])
            # The right edge's middle: a corner pixel can fall outside the rounded hit mask.
            cx, cy = x + w - 3, y + h // 2
            at(cx, cy); time.sleep(.5); btn(True); time.sleep(.4)
            apply_ms, paint_ms, st = [], [], []
            for i in range(1, 13):
                tr.start(); at(cx - i * 10, cy)
                d, _, j = tr.soon(r'\[silk\.resize\.apply\] sid=201 ', 5)
                pm, _, _ = tr.soon(r'\[quil\.ui\.applied\] paint=', 3, j) if d is not None else (None, None, 0)
                time.sleep(.05); tr.poll()
                apply_ms.append(d); paint_ms.append(pm); st.append(tr.stages())
                if i in (2, 3): tr.dump(f'resize step {i}')
                time.sleep(.15)
            btn(False); time.sleep(2)
            # The desktop after the drag: compare builds to catch pixels a partial repaint left behind.
            q.cmd('screendump', {'filename': str((a.out / 'resize.ppm').resolve())})
            res['resize'] = {'apply': summary(apply_ms), 'repaint': summary(paint_ms), 'stages': merged_stages(st),
                             'apply_all_ms': [x and round(x, 1) for x in apply_ms], 'repaint_all_ms': [x and round(x, 1) for x in paint_ms]}
            print('[latency.resize]', json.dumps(res['resize']))

        # ── Files opens a text file in Jot ──
        if 'open' not in skip:
            for n, dest in (('files', '/bin/loom'), ('jot', '/bin/jot')):
                m = expect(f'fetch http://10.0.2.2:{PORT}/{n} {dest}', r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)', 240); assert m[1].startswith('Saved'), m[1]
            expect('mkdir /0open', r'\[spindle.fs.result\] kind=3 status=0')
            expect('write /0open/a.txt opened from files', r'\[spindle.fs.result\] kind=8 status=0')
            s = len(t()); line('files /0open'); m = g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)', s, 60); fsid = int(m[3])
            g['wait'](r'\[files.app.list\] dir=/0open count=1\b', s, 60); g['wait'](rf'\[display.text.committed\] target={m[1]} ', s, 60); time.sleep(1)
            rows, st = [], []
            for run in range(a.runs):
                g['focus'](fsid); time.sleep(.6)
                tr.start(); q.key('ret')
                asked, _, _ = tr.until(r'\[files\.app\.open\] program=jot ', 30)
                win, lm, i = tr.until(r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)', 60)
                pd, task, surface = lm[1], lm[2], int(lm[3])
                loaded, _, _ = tr.soon(r'\[jot\.loaded\] bytes=', 60)
                first, fm, _ = tr.soon(rf'\[display\.[a-z_.]+\] target={pd} ', 60, i)
                shown, _, _ = tr.soon(rf'\[display\.app\.present\.reply\] target={pd} ', 60, i)
                time.sleep(.3); tr.poll()
                rows.append({'files_asked_ms': round(asked, 1), 'window_ms': round(win, 1), 'loaded_ms': loaded and round(loaded, 1),
                             'display_first_ms': first and round(first, 1), 'first_marker': fm and fm[0].split(']')[0] + ']',
                             'first_pixels_ms': shown and round(shown, 1)})
                st.append(tr.stages())
                if run in (0, 1): tr.dump(f'open run {run}', 120)
                time.sleep(1.5); s = len(t())
                g['focus'](surface); time.sleep(.3); q.key('f11')
                g['wait'](rf'\[app.private.retired\] target={pd} task={task} ', s, 90); time.sleep(.5)
            res['open'] = {'files_asked': summary([r['files_asked_ms'] for r in rows]), 'window': summary([r['window_ms'] for r in rows]),
                           'loaded': summary([r['loaded_ms'] for r in rows]), 'first_pixels': summary([r['first_pixels_ms'] for r in rows]),
                           'runs': rows, 'stages': merged_stages(st)}
            print('[latency.open]', json.dumps(res['open']))

        (a.out / 'latency.json').write_text(json.dumps(res, indent=1))
        text = t(); assert 'KERNEL PANIC' not in text and 'panicked at' not in text
        brief = {'accel': res['accel'], 'cores': res['cores']}
        if 'key' in res: brief['key_p50'] = res['key']['p50_ms']; brief['key_p90'] = res['key']['p90_ms']
        for name, r in res.get('launch', {}).items(): brief[f'{name}_window_p50'] = r['window']['p50_ms']; brief[f'{name}_pixels_p50'] = r['first_pixels']['p50_ms']; brief[f'{name}_ready_p50'] = r['ready']['p50_ms']
        if 'resize' in res: brief['resize_apply_p50'] = res['resize']['apply']['p50_ms']; brief['resize_repaint_p50'] = res['resize']['repaint']['p50_ms']
        if 'open' in res: brief['open_window_p50'] = res['open']['window']['p50_ms']; brief['open_loaded_p50'] = res['open']['loaded']['p50_ms']; brief['open_pixels_p50'] = res['open']['first_pixels']['p50_ms']
        print('[latency.summary]', json.dumps(brief))
        if 'key' in res: assert res['key']['p50_ms'] <= a.max_key_ms, f"key p50 {res['key']['p50_ms']} ms over {a.max_key_ms}"
        for name, r in res.get('launch', {}).items():
            assert r['window']['p50_ms'] is not None and r['window']['p50_ms'] <= a.max_launch_ms, (name, r['window'])
        print({'ok': True})
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
