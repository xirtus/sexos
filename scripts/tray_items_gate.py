#!/usr/bin/env python3
"""Program tray items and notification replies (slice S2) on a booted desktop.

Abacus is run from Spindle as `abacus tray`: it adds an item to the
panel with a four-row menu. A click on the item opens the menu, which Display
draws; its "Ask me a sum" row reaches the program, which posts a notification
through Bell with a button and an inline reply field. The Shell shows it as a
card; a click on the field, `12-5` typed with QMP keys and Enter send the
reply to the program, which works it out (7). Closing the program takes its
item out of the panel. Every step waits for the Shell's `[shell.tray_items.*]`
receipts, with places taken from them so clicks land on what is drawn."""
import argparse, json, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
ICON = r'\[shell.tray_items.icon\] slot=(\d+) label== x=(\d+) y=(\d+) size=(\d+)'
MENU = r'\[shell.tray_items.menu\] open=1 pd={pd} id=0 rows=4 x=(\d+) y=(\d+) w=(\d+) h=(\d+) row_h=(\d+) pad=(\d+)'
CARD = (r'\[shell.tray_items.card\] shown=1 actions=1 reply=1 x=(\d+) y=(\d+) w=(\d+) h=(\d+) action0=(\d+),(\d+) '
        r'field=(\d+),(\d+) send=(\d+),(\d+) close=(\d+),(\d+) summary=Abacus')
FRAME = r'\[shell.decor.frame\] frame=(\d+) sid={sid} [^\n]*close=(-?\d+),(-?\d+) '


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
    g = boot(a.iso, disk, out, 'tray', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def move(x, y):
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
                                              {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]})
        time.sleep(.3)

    def button(down):
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})

    def click(x, y):
        move(x, y)
        button(True); time.sleep(.15); button(False); time.sleep(.2)

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        g['focus'](153); time.sleep(.5); s = len(t())
        for k in 'run abacus tray':
            q.key({' ': 'spc'}.get(k, k)); time.sleep(.08)
        q.key('ret')
        ready = wait(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 120); pd, task = ready[1], ready[2]
        sid = int(wait(rf'\[shell.program.launch\] target={pd} task={task} surface=(\d+)', s, 60)[1])
        step(rf'\[abacus.tray.ready\] target={pd} task={task} item=1 rows=4', s, 60)
        step(rf'\[shell.tray_items.item\] pd={pd} task={task} id=0 label== rows=4 tip=Abacus: the last answer ok=1', s, 60)
        icon = wait(ICON, s, 60); time.sleep(1)
        ix, iy, size = int(icon[2]), int(icon[3]), int(icon[4])
        step(r'\[display.tray_items\] section=icons count=1', s, 30)
        q.cmd('screendump', {'filename': str(out / 'tray-item.ppm')})

        # A click on the item opens its menu, drawn by Display.
        s = len(t()); click(ix + size // 2, iy + size // 2)
        menu = step(MENU.format(pd=pd), s)
        mx, my, mw, mh, row_h, pad = map(int, menu.groups())
        step(r'\[display.tray_items\] section=menu shown=1 rows=4 ', s)
        step(rf'\[display.tray_items.draw\] menu rows=4 panel={mw}x{mh}\+{mx}\+{my}', s)
        time.sleep(1); q.cmd('screendump', {'filename': str(out / 'tray-menu.ppm')})

        # Row 3, "Ask me a sum", reaches the program, which posts a notification with a reply field.
        s = len(t()); click(mx + pad + 20, my + pad + 3 * row_h + row_h // 2)
        step(rf'\[shell.tray_items.menu\] open=0 reason=chose pd={pd} id=0 row=3', s)
        step(rf'\[shell.tray_items.told\] pd={pd} task={task} event=Menu \{{ item: 0, row: 3 \}} ok=1', s)
        step(r'\[abacus.tray.menu\] item=0 row=3 label=Ask me a sum', s)
        step(r'\[display.tray_items\] section=menu shown=0 ', s)
        step(r'\[abacus.notify.ask\] tag=1 posted=1', s)
        step(rf'\[shell.tray_items.actions\] pd={pd} task={task} committed=1 ok=1', s)
        step(rf'\[shell.tray_items.paired\] id=\d+ pd={pd} tag=1 queued=0', s)
        card = step(CARD, s)
        fx, fy = int(card[7]), int(card[8])
        step(r'\[display.tray_items\] section=card shown=1 actions=1 reply=1', s)
        step(r'\[display.tray_items.draw\] card actions=1 reply=1 active=0 len=0 ', s)
        time.sleep(1); q.cmd('screendump', {'filename': str(out / 'card.ppm')})

        # A click on the field, then typing and Enter: the reply reaches the program.
        s = len(t()); click(fx, fy)
        step(rf'\[shell.tray_items.reply\] focus=1 pd={pd} tag=1', s)
        step(r'\[display.tray_items\] section=reply active=1 len=0', s)
        s = len(t())
        for k in ['1', '2', 'minus', '5']:
            q.key(k); time.sleep(.15)
        step(r'\[shell.tray_items.reply.typed\] code=0x6 len=4', s)
        step(r'\[display.tray_items\] section=reply active=1 len=4', s)
        time.sleep(1); q.cmd('screendump', {'filename': str(out / 'reply-typed.ppm')})
        s = len(t()); q.key('ret')
        step(rf'\[shell.tray_items.reply\] sent=1 pd={pd} tag=1 len=4 via=enter', s)
        step(rf'\[shell.tray_items.told\] pd={pd} task={task} reply tag=1 len=4 ok=1', s)
        reply = step(r'\[abacus.notify.reply\] tag=1 text=12-5 answer=7 error=0', s)
        step(r'\[shell.tray_items.card\] shown=0', s)
        step(r'\[display.tray_items\] section=card shown=0 ', s)

        # Closing the program takes its item out of the panel.
        g['focus'](sid); time.sleep(1)
        f = wait(FRAME.format(sid=sid), 0, 30)
        f = re.findall(FRAME.format(sid=sid), t())[-1]
        s = len(t()); click(int(f[1]), int(f[2]))
        wait(rf'\[abacus.closed\] target={pd} task={task}', s, 60)
        step(r'\[shell.tray_items.cleanup\] gone=1 items=0 cards=0', s, 60)
        step(r'\[display.tray_items\] section=icons count=0', s, 30)
        text = t()
        for bad in ('KERNEL PANIC', 'panicked', '[abacus.fatal]', '[shell.tray_items.reject]'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, target=int(pd), task=int(task), sid=sid, icon=[ix, iy, size], menu=[mx, my, mw, mh], field=[fx, fy],
                      reply=reply[0], steps=steps,
                      scope=f'QEMU {a.resolution}: a program tray item with a menu drawn by Display, a menu row reaching the program, '
                            'a notification card with an inline reply typed with keys and sent with Enter, and the item gone with the program')
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
    except Exception as e:
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
