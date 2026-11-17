#!/usr/bin/env python3
"""Window decorations (slice S1) on a booted desktop, with the mouse only.

A program (the calculator) is opened from Spindle. Then, on its title bar:
a double click maximises it and a second one restores it; a right click
opens the window menu, which Display draws, and its Float row floats the
window; the float button tiles it again; the maximise button maximises it;
and the close button ends the program (its own close receipt, then the
kernel retiring its private memory). Every step waits for the Shell's
`[shell.decor.*]` receipts, with button places taken from the Shell's own
`[shell.decor.frame]` receipt so the clicks land on what is drawn."""
import argparse, json, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
FRAME = (r'\[shell.decor.frame\] frame=(\d+) sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) title_h=(\d+) title_x=(-?\d+) '
         r'close=(-?\d+),(-?\d+) max=(-?\d+),(-?\d+) float=(-?\d+),(-?\d+)')
MENU = (r'\[shell.decor.menu\] open=1 frame={frame} sid={sid} rows=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+) row_h=(\d+) pad=(\d+)')


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
    g = boot(a.iso, disk, out, 'decor', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def move(x, y):
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
                                              {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]})
        time.sleep(.3)

    def button(name, down):
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': name}}]})

    def click(x, y, name='left'):
        move(x, y)
        button(name, True); time.sleep(.15); button(name, False); time.sleep(.2)

    def double_click(x, y):
        # Both presses well inside the Shell's 400 ms double-click window.
        move(x, y)
        button('left', True); time.sleep(.04); button('left', False); time.sleep(.06)
        button('left', True); time.sleep(.04); button('left', False); time.sleep(.2)

    def frame(sid, start=0):
        """The Shell's latest title bar geometry for the window, after `start`."""
        wait(FRAME.format(sid=sid), start, 30)
        m = re.findall(FRAME.format(sid=sid), t()[start:])[-1]
        keys = ('frame', 'x', 'y', 'w', 'h', 'title_h', 'title_x', 'close_x', 'close_y', 'max_x', 'max_y', 'float_x', 'float_y')
        return dict(zip(keys, map(int, m)))

    def title_point(f):
        # Right of the buttons, in the middle of the title bar.
        return (max(f['title_x'] + 40, f['x'] + f['w'] // 2), f['y'] + f['title_h'] // 2)

    def action(fid, sid, name, via, start, extra=''):
        m = wait(rf'\[shell.decor.action\] frame={fid} sid={sid} action={name} via={via} ok=1{extra}', start, 30)
        steps.append(m[0]); return m

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in 'run abacus':
            q.key({' ': 'spc'}.get(k, k)); time.sleep(.08)
        q.key('ret')
        ready = wait(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 300); target, task = ready[1], ready[2]
        sid = int(wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', s, 60)[1])
        wait(rf'\[display.app.pixels\] target={target} ', s, 60); time.sleep(2)
        g['focus'](sid); time.sleep(1)
        f = frame(sid); fid = f['frame']

        # Double click on the title: maximised; again: restored.
        s = len(t()); double_click(*title_point(f))
        action(fid, sid, 'maximise', 'double_click', s, ' zoomed=1')
        wait(rf'\[silk.lifecycle.zoom.active\] frame={fid} sid={sid} ', s, 30); time.sleep(1)
        big = frame(sid, s)
        if big['w'] <= f['w'] or big['h'] <= f['h']: raise RuntimeError(f'maximise did not grow the window: {f} -> {big}')
        s = len(t()); double_click(*title_point(big))
        action(fid, sid, 'maximise', 'double_click', s, ' zoomed=0'); time.sleep(1)
        f = frame(sid)

        # Right click on the title: the window menu, drawn by Display.
        s = len(t()); tx, ty = title_point(f); click(tx, ty, 'right')
        menu = wait(MENU.format(frame=fid, sid=sid), s, 30)
        rows, mx, my, mw, mh, row_h, pad = map(int, menu.groups())
        if rows != 11: raise RuntimeError(f'window menu has {rows} rows, want 11 (maximise, float, 8 workspaces, close)')
        wait(rf'\[display.window_menu\] shown=1 rows={rows} ', s, 30)
        wait(rf'\[display.window_menu.draw\] rows={rows} panel={mw}x{mh}\+{mx}\+{my}', s, 30)
        time.sleep(1); q.cmd('screendump', {'filename': str(out / 'window-menu.ppm')})
        # Row 1 is Float: choosing it floats the window and hides the menu.
        s = len(t()); click(mx + pad + 20, my + pad + row_h + row_h // 2)
        wait(r'\[shell.decor.menu\] open=0 reason=chose action=float', s, 30)
        action(fid, sid, 'float', 'menu', s, ' zoomed=0 floating=1')
        wait(r'\[display.window_menu\] shown=0', s, 30); time.sleep(1)

        # The float button tiles it again.
        f = frame(sid); s = len(t()); click(f['float_x'], f['float_y'])
        action(fid, sid, 'float', 'button', s, ' zoomed=0 floating=0'); time.sleep(1)

        # The maximise button maximises it.
        f = frame(sid); s = len(t()); click(f['max_x'], f['max_y'])
        action(fid, sid, 'maximise', 'button', s, ' zoomed=1'); time.sleep(1)
        q.cmd('screendump', {'filename': str(out / 'maximised.ppm')})

        # The close button ends the program.
        f = frame(sid); s = len(t()); click(f['close_x'], f['close_y'])
        action(fid, sid, 'close', 'button', s)
        wait(rf'\[shell.app.close.request\] target={target} task={task} surface={sid}', s, 30)
        wait(rf'\[abacus.closed\] target={target} task={task}', s, 60)
        retired = wait(rf'\[app.private.retired\] target={target} task={task} ', s, 90)
        wait(rf'\[shell.app.cleanup\] target={target} task={task} ', s, 60)
        text = t()
        for bad in ('KERNEL PANIC', 'panicked', '[abacus.fatal]'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, target=int(target), task=int(task), sid=sid, frame=fid, menu=[mx, my, mw, mh], steps=steps, retired=retired[0],
                      scope=f'QEMU {a.resolution}: title bar double-click maximise/restore, right-click window menu drawn by Display and its Float row, float and maximise buttons, close button ends the program')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
