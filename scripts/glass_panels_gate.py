#!/usr/bin/env python3
"""Frosted glass behind menus and panels (slice S6b) on a booted desktop.

The calculator is opened from Spindle and its title bar right-clicked: the
window menu shows, and Display says it drew the menu as glass
(`[display.glass] panel=window_menu glass=1 radius=R alpha=A rect=... tint=...
floor=F ceil=C`). The screen before the menu (pointer parked away) and with
it are compared: inside the menu every background pixel is what
`servers/sexdisplay/src/glass.rs` makes of the desktop under it (two box
passes each way of radius R, the tint at A/255, luminance kept in F..C),
re-computed here from the first picture, and not what a flat tint of the
unblurred desktop would be. Where the desktop had a hard edge under the menu
the glass has none. Every row's label still reads at 4.5:1 or more against the
glass around it. A click outside closes the menu; opening it again logs the
glass once more (once per show), and the Shell's own theme receipt says
Reduce transparency is off (the default)."""
import argparse, json, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
FRAME = (r'\[shell.decor.frame\] frame=(\d+) sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) title_h=(\d+) title_x=(-?\d+) ')
MENU = r'\[shell.decor.menu\] open=1 frame={frame} sid={sid} rows=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+) row_h=(\d+) pad=(\d+)'
GLASS = (r'\[display.glass\] panel=window_menu glass=1 radius=(\d+) alpha=(\d+) rect=(\d+)x(\d+)\+(\d+)\+(\d+) '
         r'tint=([0-9a-f]{6}) floor=(\d+) ceil=(\d+)')
METRICS = r'\[display.chrome.metrics\] scale=(\d+) bar=(\d+) '

# ── servers/sexdisplay/src/glass.rs, line for line ──────────────────────────
def _lin(v):
    v /= 255
    return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
LINEAR = [round(_lin(i) * 10000) for i in range(256)]

def luminance(c):
    """WCAG relative luminance times 10000, from glass.rs's table."""
    return (2126 * LINEAR[c >> 16 & 255] + 7152 * LINEAR[c >> 8 & 255] + 722 * LINEAR[c & 255] + 5000) // 10000

def mix(a, b, t):
    u = 255 - t
    return sum((((a >> s & 255) * u + (b >> s & 255) * t + 127) // 255) << s for s in (16, 8, 0))

def mix256(a, b, k):
    return sum((((a >> s & 255) * (256 - k) + (b >> s & 255) * k + 128) >> 8) << s for s in (16, 8, 0))

def frost_pixel(c, p):
    q = mix(c, p['tint'], p['alpha'])
    ok = lambda c: p['floor'] <= luminance(c) <= p['ceil']
    if ok(q): return q
    bad, good = 0, 256
    while good - bad > 8:
        mid = (bad + good) // 2
        if ok(mix256(q, p['tint'], mid)): good = mid
        else: bad = mid
    return mix256(q, p['tint'], good)

def box_line(src, r):
    n = len(src); d = 2 * r + 1
    at = lambda i: src[min(max(i, 0), n - 1)]
    sr = sg = sb = 0
    for i in range(-r, r + 1):
        c = at(i); sr += c >> 16 & 255; sg += c >> 8 & 255; sb += c & 255
    out = [0] * n
    for x in range(n):
        out[x] = ((sr + d // 2) // d) << 16 | ((sg + d // 2) // d) << 8 | (sb + d // 2) // d
        i, o = at(x + r + 1), at(x - r)
        sr += (i >> 16 & 255) - (o >> 16 & 255); sg += (i >> 8 & 255) - (o >> 8 & 255); sb += (i & 255) - (o & 255)
    return out

def frost(get, inner, r, bounds, p, blur=True):
    """{(x, y): glass} for the panel interior, as Display draws it over `get`."""
    x0, y0, w, h = inner
    x1, y1 = x0 + w, y0 + h
    m = 2 * r
    rx0, ry0, rx1, ry1 = max(x0 - m, bounds[0]), max(y0 - m, bounds[1]), min(x1 + m, bounds[2]), min(y1 + m, bounds[3])
    if not blur:
        return {(x, y): frost_pixel(get(x, y), p) for y in range(y0, y1) for x in range(x0, x1)}
    rows = {}
    for y in range(ry0, ry1):
        line = box_line(box_line([get(x, y) for x in range(rx0, rx1)], r), r)
        rows[y] = line[x0 - rx0:x1 - rx0]
    out = {}
    for x in range(x0, x1):
        col = box_line(box_line([rows[y][x - x0] for y in range(ry0, ry1)], r), r)
        for y in range(y0, y1): out[(x, y)] = frost_pixel(col[y - ry0], p)
    return out

# ── pictures ────────────────────────────────────────────────────────────────
def ppm(path):
    data = path.read_bytes(); parts = data.split(b'\n', 3); w, h = map(int, parts[1].split())
    px = parts[3]
    return w, h, lambda x, y: int.from_bytes(px[(y * w + x) * 3:(y * w + x) * 3 + 3], 'big')

def wcag(a, b):
    la, lb = (sum(k * _lin(c >> s & 255) for k, s in ((0.2126, 16), (0.7152, 8), (0.0722, 0))) for c in (a, b))
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)

def diff(a, b): return max(abs((a >> s & 255) - (b >> s & 255)) for s in (16, 8, 0))
def luma(c): return (54 * (c >> 16 & 255) + 183 * (c >> 8 & 255) + 19 * (c & 255)) >> 8


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
    install(disk, [('abacus', (ROOT / 'iso_root/programs/abacus').read_bytes())])
    rw, rh = map(int, a.resolution.split('x'))
    g = boot(a.iso, disk, out, 'glass', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def move(x, y):
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
                                              {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]})
        time.sleep(.3)

    def click(name):
        for down in (True, False):
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': name}}]}); time.sleep(.15)

    def shot(name):
        time.sleep(1.5); path = out / f'{name}.ppm'; q.cmd('screendump', {'filename': str(path)}); time.sleep(.5); return ppm(path)

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        step(r'\[shell.theme\] mode=\S+ accent=[0-9a-f]{6} status=ok reduce_transparency=0', 0, 60)
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in 'run abacus':
            q.key({' ': 'spc'}.get(k, k)); time.sleep(.08)
        q.key('ret')
        ready = wait(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 300)
        sid = int(wait(rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)', s, 60)[1])
        wait(rf'\[display.app.pixels\] target={ready[1]} ', s, 60); time.sleep(2)
        g['focus'](sid); time.sleep(1)
        wait(FRAME.format(sid=sid), 0, 30)
        fid, fx, fy, fw, fh, title_h, title_x = map(int, re.findall(FRAME.format(sid=sid), t())[-1])
        scale, bar = map(int, re.findall(METRICS, t())[-1])

        # The desktop with no menu, the pointer parked away from where it opens.
        move(rw - 2, rh - 2)
        w, h, before = shot('00-before')
        tx, ty = max(title_x + 40, fx + fw // 2), fy + title_h // 2
        move(tx, ty); s = len(t()); click('right')
        menu = step(MENU.format(frame=fid, sid=sid), s)
        rows, mx, my, mw, mh, row_h, pad = map(int, menu.groups())
        shown = step(rf'\[display.window_menu\] shown=1 rows={rows} x=\d+ y=\d+ hover=(-?\d+)', s)
        gl = step(GLASS, s)
        radius, alpha, iw, ih, ix, iy = map(int, gl.groups()[:6])
        params = dict(tint=int(gl[7], 16), alpha=alpha, floor=int(gl[8]), ceil=int(gl[9]))
        if (ix, iy, iw, ih) != (mx + scale, my + scale, mw - 2 * scale, mh - 2 * scale):
            raise RuntimeError(f'glass rect {iw}x{ih}+{ix}+{iy} is not the menu {mw}x{mh}+{mx}+{my} inside its rim')
        if radius != 6 * min(max(scale, 1), 3): raise RuntimeError(f'radius {radius} at scale {scale}')
        time.sleep(.5)
        hover = int(re.findall(rf'\[display.window_menu\] shown=1 rows={rows} x=\d+ y=\d+ hover=(-?\d+)', t()[s:])[-1])
        _, _, after = shot('01-menu')

        inner = (ix, iy, iw, ih)
        blurred = frost(before, inner, radius, (0, bar, w, h), params)
        flat = frost(before, inner, radius, (0, bar, w, h), params, blur=False)

        # Pixels that are not glass: the rows' text, a hovered row, the rule
        # above Close, the pointer, and two pixels around each of them.
        lo, hi = params['floor'] - 40, params['ceil'] + 40
        skip = set()
        for (x, y) in blurred:
            if not lo <= luminance(after(x, y)) <= hi: skip.add((x, y))
        if hover >= 0:
            top = my + pad + hover * row_h
            skip.update((x, y) for y in range(top, top + row_h) for x in range(ix, ix + iw))
        close_top = my + pad + (rows - 1) * row_h
        skip.update((x, y) for y in range(close_top, close_top + scale) for x in range(ix, ix + iw))
        skip.update((x, y) for y in range(ty - 2, ty + 20 * scale) for x in range(tx - 2, tx + 12 * scale))
        grown = set()
        for (x, y) in skip:
            for dy in range(-2, 3):
                for dx in range(-2, 3): grown.add((x + dx, y + dy))
        glass = [k for k in blurred if k not in grown]
        if len(glass) < iw * ih // 4: raise RuntimeError(f'only {len(glass)} of {iw * ih} menu pixels look like glass')

        # The glass is the blurred desktop, re-computed from the first picture.
        close = sum(1 for k in glass if diff(after(*k), blurred[k]) <= 2)
        if close < len(glass) * 95 // 100:
            worst = sorted(glass, key=lambda k: -diff(after(*k), blurred[k]))[:5]
            raise RuntimeError(f'glass differs from the blurred desktop: {close}/{len(glass)} within 2; worst ' +
                               ', '.join(f'{k}: {after(*k):06x} want {blurred[k]:06x} flat {flat[k]:06x}' for k in worst))
        # Where blur and no blur differ the picture follows the blur.
        edge = [k for k in glass if diff(blurred[k], flat[k]) >= 10]
        if len(edge) < 25: raise RuntimeError(f'too little detail under the menu to tell blur from a flat tint ({len(edge)} pixels)')
        follows = sum(1 for k in edge if diff(after(*k), blurred[k]) + 2 < diff(after(*k), flat[k]))
        if follows < len(edge) * 9 // 10: raise RuntimeError(f'only {follows} of {len(edge)} detail pixels follow the blur')
        # Hard steps of the desktop under the menu are gone in the glass.
        steps_before, steps_after = [], []
        gset = set(glass)
        for (x, y) in glass:
            for (nx, ny) in ((x + 1, y), (x, y + 1)):
                if (nx, ny) in gset and abs(luma(before(x, y)) - luma(before(nx, ny))) >= 48:
                    steps_before.append(abs(luma(before(x, y)) - luma(before(nx, ny))))
                    steps_after.append(abs(luma(after(x, y)) - luma(after(nx, ny))))
        if not steps_after: raise RuntimeError('no hard edge in the desktop under the menu')
        steps_after.sort()
        if steps_after[len(steps_after) // 2] * 4 > min(steps_before): raise RuntimeError(f'edges stay hard in the glass: {steps_after[-5:]}')

        # Every row's label reads against the glass around it.
        worst = 99.0
        for i in range(rows):
            if i == hover: continue
            top = my + pad + i * row_h
            cells = [(x, y) for y in range(top + scale, top + row_h) for x in range(ix, ix + iw)
                     if not (tx - 2 <= x < tx + 12 * scale and ty - 2 <= y < ty + 20 * scale)]
            ground = [after(*k) for k in cells if k in gset]
            ink = [after(*k) for k in cells if k in skip]
            if not ground or not ink: continue
            ground.sort(key=luminance); g0 = ground[len(ground) // 2]
            best = max(wcag(c, g0) for c in ink)
            worst = min(worst, best)
            if best < 4.5: raise RuntimeError(f'row {i}: label contrast {best:.2f}:1 on glass {g0:06x}')
        if worst == 99.0: raise RuntimeError('no label found on the menu rows')

        # A click outside closes it; shown again, the glass is logged again.
        s = len(t()); move(min(mx + mw + 40, rw - 4), my + mh // 2); click('left')
        step(r'\[shell.decor.menu\] open=0 reason=outside', s)
        step(r'\[display.window_menu\] shown=0', s)
        time.sleep(1)
        s = len(t()); move(tx, ty); click('right')
        step(MENU.format(frame=fid, sid=sid), s)
        step(GLASS, s)
        s = len(t()); move(min(mx + mw + 40, rw - 4), my + mh // 2); click('left')
        step(r'\[display.window_menu\] shown=0', s)

        text = t()
        for bad in ('KERNEL PANIC', 'panicked', '[abacus.fatal]'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, menu=[mx, my, mw, mh], glass=dict(radius=radius, **params), glass_pixels=len(glass), within_2=close,
                      detail_pixels=len(edge), follow_blur=follows, hard_edges=len(steps_before), worst_label_contrast=round(worst, 2), steps=steps,
                      scope=f'QEMU {a.resolution}: the window menu over a window is frosted glass computed from the desktop under it '
                            '(blurred, tinted, luminance kept for 4.5:1 text), edges under it smoothed, labels readable, logged once per show')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n')
        keep = re.compile(r'glass|window_menu|decor\.menu|theme|PANIC|panic|fault', re.I)
        for line in [l for l in t().splitlines() if keep.search(l)][-80:]: print('[serial]', line[:200])
        raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
