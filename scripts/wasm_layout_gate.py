#!/usr/bin/env python3
"""Wasm layout plugins (slice S4 follow-up) on a booted desktop.

Three calculator windows are run from Spindle on the shown workspace. The
sample plugin (crates/sex-wasm/guests/layouts/spiral.wasm, built from its .wat)
and two bad ones are fetched from a host HTTP server into the filesystem.

`silk layout WS plugin:/sample.wasm`: Spindle has Files prepare a run of
`wasm layout @/sample.wasm`, the Shell starts it hidden, asks it for the
workspace's tiles (`[shell.layout.plugin.ask]`), the run answers
(`[wasm.layout.answer] ... ok=1`), the Shell applies it
(`[shell.layout.plugin] ... ok=1 applied=1`), and the windows' geometry
receipts (`[shell.tile.apply]`) are exactly the spiral's rectangles, which this
gate computes itself.

`plugin:/outside.wasm` answers with rectangles past the work area: refused
(`ok=0 reason=outside`), and the windows tile as before any plugin. The
sample's run, used by no workspace now, is closed. `plugin:/spin.wasm` never
returns: its fuel runs out (`ok=0 reason=fuel`) and tiling stays the same.
`silk layout WS master` ends the plugin and its run."""
import argparse, http.server, json, os, re, shutil, threading, time
from pathlib import Path
from install_program import install

ROOT = Path(__file__).resolve().parents[1]
GUESTS = ROOT / 'crates/sex-wasm/guests/layouts'
FILES = {'/sample.wasm': (GUESTS / 'spiral.wasm').read_bytes(), '/outside.wasm': (GUESTS / 'outside.wasm').read_bytes(),
         '/spin.wasm': (GUESTS / 'spin.wasm').read_bytes()}


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
os.environ.setdefault('SEXOS_GATE_BOUND', '900')
from quil_autosave_gate import boot  # noqa: E402  (after the NIC is chosen)

DONE = r'\[spindle.silk.done\] tag=(\d+) lines=(\d+) ok=([01])'
LINE = r'\[spindle.silk.line\] tag={} n=\d+ text=([^\r\n]*)'
KEYS = {' ': 'spc', '/': 'slash', '.': 'dot', '-': 'minus'}
APPLY = r'\[shell.tile.apply\] sid=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)'


def spiral(n, w, h):
    """crates/sex-wasm/guests/layouts/spiral.wat, relative to the work area."""
    x = y = 0; out = []
    for i in range(n):
        if i + 1 == n: out.append((x + 4, y + 4, w - 8, h - 8)); break
        k = i % 4
        if k == 0: half = w // 2; out.append((x + 4, y + 4, half - 8, h - 8)); x += half; w -= half
        elif k == 1: half = h // 2; out.append((x + 4, y + 4, w - 8, half - 8)); y += half; h -= half
        elif k == 2: half = w // 2; out.append((x + w - half + 4, y + 4, half - 8, h - 8)); w -= half
        else: half = h // 2; out.append((x + 4, y + h - half + 4, w - 8, half - 8)); h -= half
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--binding', type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(x for x in out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    with disk.open('r+b') as f: f.truncate(1024 ** 3)
    install(disk, [('abacus', (ROOT / 'iso_root/programs/abacus').read_bytes()), ('wasm', (ROOT / 'iso_root/programs/wasm').read_bytes())])
    screen_w, screen_h = (int(v) for v in a.resolution.split('x'))
    g = boot(a.iso, disk, out, 'wasmlayout', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def step(pattern, start, timeout=60):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def shot(name):
        time.sleep(1.5); q.cmd('screendump', {'filename': str(out / f'{name}.ppm')})

    def shifted(k): q.cmd('send-key', {'keys': [{'type': 'qcode', 'data': 'shift'}, {'type': 'qcode', 'data': k}], 'hold-time': 80})

    def typed(line):
        """Types a line into Spindle and presses Enter; returns where the log stood."""
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in line:
            if k == ':': shifted('semicolon')
            elif k == '@': shifted('2')
            else: q.key(KEYS.get(k, k))
            time.sleep(.08)
        q.key('ret')
        return s

    def silk(words, timeout=60):
        """`silk WORDS` in Spindle: where the log stood, the answer's lines, and whether it ended ok."""
        s = typed('silk ' + words)
        tag = step(r'\[spindle.silk.request\] kind=\w+ tag=(\d+) sent=1', s, timeout)[1]
        done = wait(DONE.replace(r'tag=(\d+)', f'tag={tag}'), s, timeout); steps.append(done[0])
        lines = re.findall(LINE.format(tag), t()[s:])
        if len(lines) != int(done[1]): raise RuntimeError(f'silk {words}: {len(lines)} lines, done says {done[1]}')
        return s, lines, done[2] == '1'

    def applied_after(marker_at, n, timeout=60):
        """The n `[shell.tile.apply]` receipts that follow log offset marker_at."""
        d = time.monotonic() + timeout
        while True:
            got = re.findall(APPLY, t()[marker_at:])
            if len(got) >= n: return [(int(s), int(x), int(y), int(w), int(h)) for s, x, y, w, h in got[:n]]
            if time.monotonic() > d: raise TimeoutError(f'{len(got)} of {n} tile receipts after the marker')
            time.sleep(.2)

    def last_tiling(n):
        """The geometry of the last complete tiling of n windows: sid -> rect."""
        text = t(); begins = [m.end() for m in re.finditer(r'\[shell.tile.begin\]', text)]
        for at in reversed(begins):
            got = re.findall(APPLY, text[at:text.find('[shell.tile.begin]', at) if '[shell.tile.begin]' in text[at:] else len(text)])
            if len(got) == n: return {int(s): (int(x), int(y), int(w), int(h)) for s, x, y, w, h in got}
        raise RuntimeError(f'no tiling of {n} windows found')

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        sids = []
        for i in range(3):
            s = typed('run abacus')
            ready = step(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 300)
            sids.append(int(step(rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)', s, 60)[1]))
            wait(rf'\[display.app.pixels\] target={ready[1]} ', s, 60); time.sleep(2)

        # The plugins reach the filesystem as any downloaded file does.
        for name in FILES:
            s = typed(f'fetch http://10.0.2.2:{PORT}{name} {name}')
            m = wait(r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)', s, 240)
            if not m[1].startswith('Saved'): raise RuntimeError(f'fetch {name}: {m[1]}')
            time.sleep(1)

        s, lines, ok = silk('windows')
        if not ok: raise RuntimeError(f'silk windows did not end ok: {lines}')
        ws = int(re.match(r'ok windows=\d+ ws=(\d)', lines[-1])[1])
        tiled = [l for l in lines[:-1] if f' ws={ws} ' in l and 'state=tiled' in l]
        mine = [l for l in tiled if any(re.match(rf'frame=\d+ sid={sid} ', l) for sid in sids)]
        if len(mine) != 3: raise RuntimeError(f'the three calculators are not tiled on workspace {ws}: {lines}')
        n = len(tiled)
        before = last_tiling(n)
        if not set(sids) <= set(before): raise RuntimeError(f'the last tiling {before} lacks the calculators {sids}')
        shot('01-master')

        # The sample plugin: asked, answered, checked and applied.
        s, lines, ok = silk(f'layout {ws} plugin:/sample.wasm')
        if not ok or not re.match(rf'ok layout ws={ws} plugin=[0-9a-f]{{16}} run=started', lines[-1]): raise RuntimeError(f'silk layout plugin: {lines}')
        key = step(r'\[spindle.silk.plugin\] ws=\d what=/sample.wasm key=([0-9a-f]+) prepare=1', s)[1]
        step(rf'\[shell.layout.plugin.run\] key={key} state=starting ws={ws} ', s)
        step(rf'\[shell.layout.plugin.run\] key={key} state=running target=\d+ task=\d+', s, 120)
        step(r'\[wasm.layout.ready\] target=\d+ task=\d+ what=@/sample.wasm ok=1 reason=-', s, 120)
        ask = step(rf'\[shell.layout.plugin.ask\] ws={ws} key={key} gen=(\d+) n=(\d+) w=(\d+) h=(\d+) sent=1', s)
        gen, count, aw, ah = (int(v) for v in ask.groups())
        if count != n: raise RuntimeError(f'the plugin was asked for {count} tiles, {n} windows are tiled')
        step(rf'\[wasm.layout.answer\] gen={gen} n={n} w={aw} h={ah} ok=1 reason=- sent=1', s)
        done = step(rf'\[shell.layout.plugin\] ws={ws} key={key} gen={gen} n={n} ok=1 applied=1', s)
        ax, ay = 0, screen_h - ah  # the work area spans the width, below the bar
        expect = [(ax + x, ay + y, w, h) for x, y, w, h in spiral(n, aw, ah)]
        rects = [tuple(int(v) for v in r) for r in re.findall(rf'\[shell.layout.plugin.rect\] ws={ws} gen={gen} i=\d+ x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)', t()[s:])][:n]
        if rects != expect: raise RuntimeError(f'plugin rectangles {rects} are not the spiral {expect}')
        tile = step(rf'\[shell.layout.plugin.tile\] ws={ws} key={key} gen={gen} n={n}', s)
        marker = s + tile.end()
        got = applied_after(marker, n)
        if sorted(r[1:] for r in got) != sorted(expect): raise RuntimeError(f'window geometry {got} is not the spiral {expect}')
        if not set(sids) <= {r[0] for r in got}: raise RuntimeError(f'the calculators {sids} are not among the windows placed {got}')
        shot('02-spiral')

        # A plugin whose rectangles leave the work area: refused, the previous layout stays.
        s, lines, ok = silk(f'layout {ws} plugin:/outside.wasm')
        if not ok: raise RuntimeError(f'silk layout plugin:/outside.wasm: {lines}')
        bad = step(r'\[spindle.silk.plugin\] ws=\d what=/outside.wasm key=([0-9a-f]+) prepare=1', s)[1]
        step(rf'\[shell.layout.plugin.run\] key={key} state=stopping ', s, 60)
        step(rf'\[shell.layout.plugin.run\] key={key} state=stopped ', s, 60)
        step(r'\[wasm.layout.answer\] gen=(\d+) n=\d+ w=\d+ h=\d+ ok=1 reason=- sent=1', s, 120)
        refused = step(rf'\[shell.layout.plugin\] ws={ws} key={bad} gen=\d+ n={n} ok=0 reason=outside index=0 kept=(\w+)', s, 60)
        time.sleep(2)
        after = last_tiling(n)
        if after != before: raise RuntimeError(f'after the refused answer the windows are {after}, not the previous layout {before}')
        shot('03-outside-refused')

        # A plugin that never returns: out of fuel, the previous layout stays.
        s, lines, ok = silk(f'layout {ws} plugin:/spin.wasm')
        if not ok: raise RuntimeError(f'silk layout plugin:/spin.wasm: {lines}')
        spin = step(r'\[spindle.silk.plugin\] ws=\d what=/spin.wasm key=([0-9a-f]+) prepare=1', s)[1]
        step(r'\[wasm.layout.answer\] gen=\d+ n=\d+ w=\d+ h=\d+ ok=0 reason=fuel sent=1', s, 120)
        fuel = step(rf'\[shell.layout.plugin\] ws={ws} key={spin} gen=\d+ ok=0 reason=fuel kept=(\w+)', s, 60)
        time.sleep(2)
        after = last_tiling(n)
        if after != before: raise RuntimeError(f'after the fuel ran out the windows are {after}, not the previous layout {before}')

        # A named layout ends the plugin, and its run.
        s, lines, ok = silk(f'layout {ws} {refused[1]}')
        if not ok: raise RuntimeError(f'silk layout {ws} {refused[1]}: {lines}')
        step(rf'\[shell.layout.plugin\] ws={ws} key={spin} cleared=1 reason=silk_layout', s)
        step(rf'\[shell.layout.plugin.run\] key={spin} state=stopped ', s, 60)

        text = t()
        for bad_mark in ('KERNEL PANIC', 'panicked', '[abacus.fatal]', '[wasm.fatal]', '[shell.silk.reject]', '[shell.layout.plugin.reject]', 'reason=ended'):
            if bad_mark in text: raise RuntimeError('guest reported ' + bad_mark)
        result = dict(ok=True, workspace=ws, windows=n, generation=gen, area=dict(x=ax, y=ay, w=aw, h=ah), spiral=expect,
                      placed=got, previous=before, refused=refused[0], fuel=fuel[0], applied=done[0], steps=steps,
                      scope=f'QEMU {a.resolution}: a Wasm layout plugin\'s own run tiles {n} windows as a spiral; '
                            'a plugin with rectangles outside the work area and one that runs out of fuel are refused '
                            'and the previous layout stays')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
