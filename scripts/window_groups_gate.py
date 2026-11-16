#!/usr/bin/env python3
"""Tabbed window groups, the dwindle layout and per-workspace layout memory
(slice S3) on a booted desktop, with Super chords only.

The calculator is run three times from Spindle. Super+T puts workspace 1 in
dwindle: each window takes half of the tile before it, side by side then
stacked in turn, as the Shell's `[shell.layout.tile]` receipts show. With the
third calculator focused, Super+G twice pulls the other two into its group
(one frame, three tabs). Super+. steps through the tabs in order and back to
the first, Super+, steps back; each step focuses that tab's window. Super+2
shows workspace 2 in its own layout (master), and Super+1 comes back to
dwindle with the group of three still there and still cycling. Super+Shift+G
takes the focused tab out of the group again. Every step waits for the
Shell's `[shell.group]` / `[shell.layout*]` receipts."""
import argparse, json, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
ACTION = r'\[shell.wm.action\] action={} done=1 scene=(\d)->(\d) focus=(\d+)->(\d+) layout=(\w+)'
FRESH = r'\[shell.layout\] scene=0 layout=dwindle windows=(\d+) leaves=(\d+) fresh=1'
TILE = r'\[shell.layout.tile\] scene=0 n=(\d+)/(\d+) frame=(\d+) sid=(\d+) depth=(\d+) split=([hv-]) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)'
JOIN = r'\[shell.group\] join frame=(\d+) took=(\d+) tabs={} active=(\d+) sid=(\d+) members=([\d,]+) ok=1'
TAB = r'\[shell.group\] tab frame={} step={} tab=(\d+)/3 sid=(\d+) focus=(\d+) members=([\d,]+) ok=1'
SCENE = r'\[shell.layout.scene\] scene={} layout=(\w+) leaves=(\d+) groups=(\d+) tabs=(\d+)'
SPLIT = r'\[shell.group\] split frame={} sid={} to=(\d+) tabs=2 active=(\d+) members=([\d,]+) ok=1'


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
    g = boot(a.iso, disk, out, 'groups', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def send(down, key):
        q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': key}}}]})
        time.sleep(.08)

    def chord(keys, action):
        """Press in order, release in reverse; the Shell's record of the action."""
        s = len(t())
        for k in keys: send(True, k)
        for k in reversed(keys): send(False, k)
        m = wait(ACTION.format(re.escape(action)), s, 30); steps.append(m[0])
        return s, m

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def shot(name):
        time.sleep(1.5); q.cmd('screendump', {'filename': str(out / f'{name}.ppm')})

    def calc():
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in 'run abacus':
            q.key({' ': 'spc'}.get(k, k)); time.sleep(.08)
        q.key('ret')
        ready = wait(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 300)
        sid = int(wait(rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)', s, 60)[1])
        wait(rf'\[display.app.pixels\] target={ready[1]} ', s, 60); time.sleep(2)
        return sid

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        sids = [calc() for _ in range(3)]
        if len(set(sids)) != 3: raise RuntimeError(f'three runs, not three windows: {sids}')

        # Super+T: dwindle. Window n+1 takes the second half of window n's
        # tile: right of it after an even number of splits, below after an odd.
        s, m = chord(['meta_l', 't'], 'ToggleDwindle')
        if m[5] != 'dwindle': raise RuntimeError('Super+T did not choose dwindle: ' + m[0])
        fresh = wait(FRESH, s, 30)
        count = int(fresh[1])
        if count < 3: raise RuntimeError(f'dwindle tiled {count} windows, want at least 3')
        block = t()[s + t()[s:].index(fresh[0]):]
        tiles = {}
        for row in re.findall(TILE, block):
            n = int(row[0])
            if int(row[1]) != count or n in tiles: break
            tiles[n] = dict(frame=int(row[2]), sid=int(row[3]), depth=int(row[4]), split=row[5],
                            x=int(row[6]), y=int(row[7]), w=int(row[8]), h=int(row[9]))
        if sorted(tiles) != list(range(count)): raise RuntimeError(f'dwindle tiles {sorted(tiles)}, want 0..{count - 1}')
        for n in range(count - 1):
            a1, b1 = tiles[n], tiles[n + 1]
            if n % 2 == 0 and b1['x'] < a1['x'] + a1['w']: raise RuntimeError(f'tile {n + 1} not right of tile {n}: {a1} {b1}')
            if n % 2 == 1 and b1['y'] < a1['y'] + a1['h']: raise RuntimeError(f'tile {n + 1} not below tile {n}: {a1} {b1}')
        for n, tile in tiles.items():
            want = '-' if tile['depth'] == 0 else 'hv'[(tile['depth'] - 1) % 2]
            if tile['split'] != want: raise RuntimeError(f'tile {n} split {tile["split"]} at depth {tile["depth"]}, want {want}')
        if not all(sid in {x['sid'] for x in tiles.values()} for sid in sids): raise RuntimeError(f'calculators not all tiled: {tiles}')
        shot('01-dwindle')

        # Super+G twice from the third calculator: the other two join its group.
        g['focus'](sids[2]); time.sleep(1)
        s, _ = chord(['meta_l', 'g'], 'Group')
        j = step(JOIN.format(2), s)
        fid = int(j[1])
        s, _ = chord(['meta_l', 'g'], 'Group')
        j = step(JOIN.format(3), s)
        if int(j[1]) != fid: raise RuntimeError('second join went to another group: ' + j[0])
        members = [int(x) for x in j[5].split(',')]
        if members != [sids[2], sids[1], sids[0]]: raise RuntimeError(f'group members {members}, want {sids[::-1]}')
        if int(j[4]) != sids[2]: raise RuntimeError('the focused calculator is not the group\'s shown tab: ' + j[0])
        shot('02-group')

        # Super+. through every tab and back to the first; Super+, back one.
        order = []
        for want in (1, 2, 0):
            s, _ = chord(['meta_l', 'dot'], 'NextTab')
            m = step(TAB.format(fid, 1), s)
            tab, sid, focus = int(m[1]), int(m[2]), int(m[3])
            if tab != want or sid != members[want] or focus != sid: raise RuntimeError(f'Super+. gave {m[0]}, want tab {want} sid {members[want]}')
            order.append(sid)
            if want == 1: shot('03-second-tab')
        s, _ = chord(['meta_l', 'comma'], 'PrevTab')
        m = step(TAB.format(fid, -1), s)
        if int(m[1]) != 2 or int(m[2]) != members[2] or int(m[3]) != members[2]: raise RuntimeError('Super+, did not step back: ' + m[0])
        order.append(int(m[2]))

        # Workspace 2 has its own layout; workspace 1 keeps dwindle and the group.
        s, m = chord(['meta_l', '2'], 'Workspace(1)')
        if (m[1], m[2]) != ('0', '1'): raise RuntimeError('Super+2 did not show workspace 2: ' + m[0])
        away = step(SCENE.format(1), s)
        if away[1] != 'master': raise RuntimeError('workspace 2 took workspace 1\'s layout: ' + away[0])
        s, m = chord(['meta_l', '1'], 'Workspace(0)')
        if m[5] != 'dwindle': raise RuntimeError('workspace 1 forgot dwindle: ' + m[0])
        back = step(SCENE.format(0), s)
        if back[1] != 'dwindle' or int(back[3]) < 1 or int(back[4]) != 3: raise RuntimeError('workspace 1 forgot its layout or group: ' + back[0])
        shot('04-back')
        s, _ = chord(['meta_l', 'dot'], 'NextTab')
        m = step(TAB.format(fid, 1), s)
        if int(m[1]) != 0 or int(m[2]) != members[0]: raise RuntimeError('the group did not cycle after the workspace switch: ' + m[0])

        # Super+Shift+G: the focused tab leaves the group for a tile of its own.
        s, _ = chord(['meta_l', 'shift', 'g'], 'Ungroup')
        m = step(SPLIT.format(fid, members[0]), s)
        if int(m[2]) != members[1] or [int(x) for x in m[3].split(',')] != members[1:]: raise RuntimeError('ungroup left the wrong tabs: ' + m[0])
        shot('05-ungrouped')

        text = t()
        for bad in ('KERNEL PANIC', 'panicked', '[abacus.fatal]'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, calculators=sids, dwindle=tiles, group=fid, members=members, cycle=order, steps=steps,
                      scope=f'QEMU {a.resolution}: Super+T dwindle with alternating splits, Super+G groups three windows into one frame, '
                            'Super+. and Super+, cycle its tabs, workspaces keep their own layout and group, Super+Shift+G ungroups')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
