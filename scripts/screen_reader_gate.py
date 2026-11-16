#!/usr/bin/env python3
"""Whisper, the screen reader, on a booted desktop (slice S7).

Super+Alt+R turns the reader on: Shell shows the notice, starts the whisper-voice
program (hidden, no window) and speaks "Screen reader on". Then focus moves between
two windows, Abacus (a Weft program) and the Spindle terminal, and each time Shell
says the focused window's title ([shell.reader.say] text=TITLE) and the voice speaks
it ([whisper.voice.spoke] with samples and a peak above zero). Abacus asked whether a
reader is on and sends its accessibility tree; pressing its 7 key focuses that
button, and Shell says "7 button". Super+Alt+R again turns the reader off and the
voice ends.

QEMU has no sound card here: the voice still makes the speech (the samples and
peak are counted as it is made), it only has no ring to play it into."""
import argparse
import json
import os
import re
import shutil
import time
from pathlib import Path

os.environ.setdefault('SEXOS_GATE_BOUND', '900')
from install_program import install
from quil_autosave_gate import boot
from program_gate_geometry import maximize

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a = p.parse_args()
    out = a.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()): raise RuntimeError('fresh output required')
    disk = out / 'nvme.img'; shutil.copyfile(a.source_disk, disk)
    programs = []
    for name in ('whisper-voice', 'abacus'):
        elf = (ROOT / 'iso_root/programs' / name).read_bytes(); assert elf[:4] == b'\x7fELF', name
        programs.append((name, elf))
    install(disk, programs)
    rw, rh = map(int, a.resolution.split('x'))
    g = boot(a.iso.resolve(strict=True), disk, out, 'reader', a.resolution)
    t = g['text']; q = g['q']; wait = g['wait']
    result = dict(ok=False)

    def keys(names):
        for k in names: q.key(k); time.sleep(.08)

    def chord(key):
        # Super+Alt+key, as a person presses it.
        for code, down in (('meta_l', True), ('alt', True), (key, True), (key, False), ('alt', False), ('meta_l', False)):
            q.cmd('input-send-event', {'events': [{'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': code}}}]})
            time.sleep(.06)

    def move(x, y):
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
                                              {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]}); time.sleep(.3)

    def button(down): q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]}); time.sleep(.2)

    def said(text, start, timeout=60):
        """Shell said exactly `text`, and the voice spoke it: returns (samples, peak)."""
        m = wait(r'\[shell.reader.say\] text=' + re.escape(text) + r'\n', start, timeout)
        at = start + m.end()
        v = wait(r'\[whisper.voice.text\] ' + re.escape(text) + r'\n', at, timeout)
        at += v.end()
        s = wait(r'\[whisper.voice.spoke\] samples=(\d+) ms=(\d+) peak=(\d+) stopped=(\d)', at, timeout)
        samples, peak = int(s[1]), int(s[3])
        if samples <= 0 or peak <= 0: raise RuntimeError(f'the voice made no sound for {text!r}: {s[0]}')
        return dict(text=text, samples=samples, ms=int(s[2]), peak=peak, stopped=int(s[4]))

    try:
        wait(r'\[app.private.retired\] target=41 ', 0, 180)
        g['focus'](153); time.sleep(.5)

        # The reader on: the notice, the voice program, its first words.
        s = len(t()); chord('r')
        wait(r'\[shell.access\] change=Reader\(true\) status=0x0 notice=\d', s)
        wait(r'\[shell.reader\] on=1', s)
        v = wait(r'\[shell.reader.voice\] state=running target=(\d+) task=(\d+)', s, 180)
        voice = (int(v[1]), int(v[2]))
        wait(r'\[whisper.voice.ready\] rate=(\d+)', s, 120)
        heard = [said('Screen reader on', s, 120)]

        # Abacus starts and takes focus: its title is spoken.
        g['focus'](153); time.sleep(.5)
        s = len(t()); keys(['spc' if k == ' ' else k for k in 'run abacus']); q.key('ret')
        m = wait(r'\[abacus.ready\] target=(\d+) task=(\d+)', s, 300); target, task = int(m[1]), int(m[2])
        surface = int(wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)', s, 60)[1])
        heard.append(said('Abacus', s, 120))
        # Abacus asked whether a reader is on, heard yes, and sent its tree.
        wait(rf'\[shell.reader.asked\] pd={target} task={task} ', s, 60)
        wait(rf'\[shell.reader.tree\] pd={target} applied=\w+ nodes=([1-9]\d*) ', s, 60)

        # Focus to Spindle and back: each title is spoken.
        s = len(t()); g['focus'](153)
        heard.append(said('Spindle', s))
        s = len(t()); g['focus'](surface)
        heard.append(said('Abacus', s))

        # A Weft button: pressing Abacus's 7 focuses it, and Shell names it.
        time.sleep(2)
        tile, before = maximize(g, surface)
        lay = wait(r'\[abacus.layout\] w=(\d+) h=(\d+)\n', before, 60); at = before + t()[before:].index(lay[0])
        wait(r'\[abacus.key\] n=29 x=(-?\d+) y=(-?\d+)\n', at, 60); time.sleep(1)
        pos = {int(n): (int(x), int(y)) for n, x, y in re.findall(r'\[abacus.key\] n=(\d+) x=(-?\d+) y=(-?\d+)', t()[at:])}
        x, y = pos[10]  # abacus::KEYS row 3: 7 8 9 ÷ π
        g['focus'](surface); time.sleep(.5)
        s = len(t()); move(tile[0] + 8 + x, tile[1] + 36 + y); button(True)
        heard.append(said('7 button', s))
        button(False)

        # The reader off: the voice ends.
        s = len(t()); chord('r')
        wait(r'\[shell.access\] change=Reader\(false\) status=0x0 notice=\d', s)
        wait(r'\[shell.reader\] on=0', s)
        wait(r'\[whisper.voice.done\]|\[app.private.retired\] target=' + str(voice[0]) + ' ', s, 120)
        s = len(t()); g['focus'](153); time.sleep(2)
        if '[shell.reader.say]' in t()[s:]: raise RuntimeError('the reader spoke while off')
        text = t()
        if 'KERNEL PANIC' in text or 'abacus.fatal' in text: raise RuntimeError('fault')
        result = dict(ok=True, voice=dict(target=voice[0], task=voice[1]), abacus=dict(target=target, task=task, surface=surface), heard=heard,
                      scope='QEMU: Super+Alt+R starts the whisper-voice program, focus titles (Abacus, Spindle) and a Weft button (7) are spoken with nonzero samples and peak; off ends the voice')
        print(json.dumps(result))
    except Exception as e:
        result = dict(ok=False, error=str(e))
        raise
    finally:
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        g['stop']()


if __name__ == '__main__':
    main()
