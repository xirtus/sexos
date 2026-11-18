#!/usr/bin/env python3
"""The desktop weather card (slice S5c) on a booted desktop with a network.

Settings' first-start setup picks Europe/Berlin as the time zone. On an empty
workspace, Super+Shift+W shows the cards; the Shell starts `steamy` hidden, with
the network right (`[shell.weather] state=starting|running`), tells it the zone
(`[shell.weather.place] zone=5 ok=1`), and the helper asks Open-Meteo about
Berlin (`[steamy.ask] zone=5 lat=5252 lon=1340`). Its report reaches the Shell
from that run only (`[shell.weather.report] ... ok=1`) and Display draws it.

The gate runs where the public Internet may or may not be reachable from the
guest, so it takes either answer the helper can give and says which it got:
a forecast ("Berlin 7°" / "Cloudy, 9° / −2°", checked for shape), or the card
saying Open-Meteo is out of reach. Either way the whole path ran. Super+Shift+W
again hides the cards and the Shell ends the helper."""
import argparse, json, os, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
DRAW = r'\[display.widgets.draw\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) time="[^"]*" date="[^"]*" cpu="[^"]*" free="[^"]*" sky="([^"]*)" temps="([^"]*)"'
BERLIN = 5


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(x for x in out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    install(disk, [('tatami', (ROOT / 'iso_root/programs/tatami').read_bytes())])
    os.environ.setdefault('SEXOS_GATE_NIC', '-netdev user,id=n0 -device e1000e,netdev=n0')
    g = boot(a.iso, disk, out, 'weather', a.resolution); t = g['text']; q = g['q']; wait = g['wait']
    steps = []

    def step(pattern, start, timeout=30):
        m = wait(pattern, start, timeout); steps.append(m[0]); return m

    def chord(*keys):
        q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': True, 'key': {'type': 'qcode', 'data': k}}} for k in keys]})
        time.sleep(.1)
        q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': False, 'key': {'type': 'qcode', 'data': k}}} for k in reversed(keys)]})
        time.sleep(.4)

    def typed(text):
        for k in text: q.key('spc' if k == ' ' else k); time.sleep(.05)

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 120); time.sleep(2)
        # Settings' first start: keep the layout, pick Berlin, no password, never idle.
        g['focus'](153); time.sleep(.5); s = len(t())
        typed('run tatami'); q.key('ret')
        step(r'\[settings.setup\] step=start', s, 120); time.sleep(1)
        q.key('ret'); step(r'\[settings.setup\] step=Layout ok=1', s, 60)
        typed('berlin'); time.sleep(.3); q.key('ret'); step(r'\[settings.setup\] step=Zone ok=1', s, 60)
        q.key('ret'); step(r'\[settings.setup\] step=Password ok=1', s, 60)
        q.key('down'); time.sleep(.2); q.key('down'); time.sleep(.2); q.key('ret'); step(r'\[settings.setup\] step=Idle ok=1', s, 60)
        step(rf'\[shell.clock.zone\] zone={BERLIN} ok=1', s, 60)

        s = len(t()); chord('meta_l', '2')
        step(r'\[shell.wm.action\] action=Workspace\(1\) done=1', s)
        s = len(t()); chord('meta_l', 'shift', 'w')
        step(r'\[shell.widgets\] on=1 ok=1', s)
        step(r'\[display.widgets\] cards=7', s)
        step(rf'\[shell.weather\] state=starting zone={BERLIN}', s, 30)
        run = step(r'\[shell.weather\] state=running target=(\d+) task=(\d+)', s, 60)
        step(r'\[steamy.ready\] target=' + run[1] + ' ', s, 60)
        step(rf'\[shell.weather.place\] zone={BERLIN} ok=1', s, 30)
        step(rf'\[steamy.ask\] zone={BERLIN} lat=5252 lon=1340', s, 30)
        rep = step(rf'\[steamy.report\] zone={BERLIN} report=(Ok\(Weather \{{[^}}]*\}}\)|Unavailable\((\d)\)) ms=\d+ sent=1', s, 120)
        step(rf'\[shell.weather.report\] pd={run[1]} zone={BERLIN} report=Some\(.*\) ok=1', s, 30)
        step(rf'\[display.widgets.weather\] zone={BERLIN} ', s, 30)
        if rep[1].startswith('Ok'):
            got = 'forecast'
            card = step(DRAW.replace(r'sky="([^"]*)"', r'sky="(Berlin −?\d+°)"'), s, 30)
            if not re.fullmatch(r'[A-Z][a-z ,]+, −?\d+° / −?\d+°', card[6]): raise RuntimeError('forecast line: ' + card[6])
        else:
            got = 'unavailable'
            if rep[2] not in ('2', '3'): raise RuntimeError('odd reason: ' + rep[0])
            card = step(DRAW.replace(r'temps="([^"]*)"', r'temps="(Open-Meteo out of reach|No forecast just now)"'), s, 30)
            if card[5] != 'Weather': raise RuntimeError('weather line: ' + card[5])

        # Off again: the cards go and the Shell ends the helper.
        s = len(t()); chord('meta_l', 'shift', 'w')
        step(r'\[shell.widgets\] on=0 ok=1', s)
        step(rf'\[shell.weather\] state=stopped target={run[1]} task={run[2]}', s, 30)
        step(rf'\[app.private.retired\] target={run[1]} task={run[2]} ', s, 60)
        text = t()
        for bad in ('KERNEL PANIC', 'panicked'):
            if bad in text: raise RuntimeError('guest reported ' + bad)
        result = dict(ok=True, got=got, report=rep[1], card=[card[5], card[6]], helper=[int(run[1]), int(run[2])], steps=steps,
                      scope=f'QEMU {a.resolution} with user networking: with the zone set to Berlin, Super+Shift+W starts the '
                            'hidden weather helper with the network, the Shell tells it the zone, its Open-Meteo report '
                            f'({got}) reaches Display, and the cards going end the helper')
        (out / 'result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n'); print(json.dumps(result, ensure_ascii=False))
    except Exception as e:
        tail = [l for l in t().splitlines() if re.search(r'weather|steamy|widgets|settings\.setup|clock\.zone|PANIC|panic|refused', l)][-80:]
        print('--- weather: serial lines ---'); print('\n'.join(l[:220] for l in tail))
        (out / 'result.json').write_text(json.dumps(dict(ok=False, error=str(e), steps=steps)) + '\n'); raise
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
