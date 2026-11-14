#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Moan engine -> program sound ring -> Bell -> virtio-snd -> recorded stereo.
The guest edits/splits two tracks, pans them apart and mixes 400/600 Hz tones.
This is an engine proof; no claim about the pending Weft UI or microphone UX.
"""
import argparse
import array
import json
import os
from pathlib import Path
import shutil
import sys
import time
import wave
from install_program import install

ROOT = Path(__file__).resolve().parents[1]


def inspect(path):
    with wave.open(str(path), 'rb') as wav:
        assert wav.getnchannels() == 2 and wav.getsampwidth() == 2
        rate = wav.getframerate()
        assert rate == 48000
        pcm = array.array('h', wav.readframes(wav.getnframes()))
    if sys.byteorder != 'little':
        pcm.byteswap()
    result = []
    for channel, expected in enumerate((400, 600)):
        samples = pcm[channel::2]
        loud = [i for i, value in enumerate(samples) if abs(value) > 1000]
        assert loud, f'channel {channel} is silent'
        duration = (loud[-1] - loud[0] + 1) / rate
        assert 1.9 < duration < 2.1, (channel, duration)
        # Exclude fades and card start/stop transients. Check channel identity,
        # amplitude and continuity in the middle of the recorded mix.
        middle = samples[loud[0] + rate // 4:loud[0] + rate * 7 // 4]
        assert len(middle) == rate * 3 // 2
        assert sum(abs(x) < 1000 for x in middle) < rate // 100, 'dropout'
        crossings = sum(a <= 0 < b for a, b in zip(middle, middle[1:]))
        hz = crossings / (len(middle) / rate)
        assert abs(hz - expected) < expected * .02, (channel, hz)
        peak = max(abs(v) for v in middle)
        assert 5700 <= peak <= 6300, (channel, peak)
        result.append(dict(channel=channel, hz=hz, seconds=duration, peak=peak))
    return result


def inspect_effects(path):
    with wave.open(str(path), 'rb') as wav:
        assert wav.getnchannels() == 2 and wav.getsampwidth() == 2
        rate = wav.getframerate()
        assert rate == 48000
        pcm = array.array('h', wav.readframes(wav.getnframes()))
    if sys.byteorder != 'little':
        pcm.byteswap()
    result = []
    for channel, expected in enumerate((400, 600)):
        samples = pcm[channel::2]
        first = next(i for i, value in enumerate(samples) if abs(value) > 1000)
        middle = samples[first + rate // 4:first + rate * 3 // 4]
        assert len(middle) == rate // 2
        crossings = sum(a <= 0 < b for a, b in zip(middle, middle[1:]))
        hz = crossings * 2
        assert abs(hz - expected) < expected * .03, (channel, hz)
        peak = max(abs(v) for v in middle)
        assert 2200 < peak < 5000, (channel, peak)
        # Source is silent after frame 48000. A real delayed tail must remain,
        # then decay; reading only the guest's success receipt is insufficient.
        tail = samples[first + rate:first + rate + rate // 5]
        late = samples[first + rate * 9 // 5:first + rate * 2]
        assert len(tail) == rate // 5 and len(late) == rate // 5
        early_energy = sum(v*v for v in tail)
        late_energy = sum(v*v for v in late)
        assert early_energy > rate * 100, (channel, 'missing tail')
        assert late_energy < early_energy // 10, (channel, early_energy, late_energy)
        result.append(dict(channel=channel, hz=hz, peak=peak, tail_energy=early_energy, late_energy=late_energy))
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--iso', required=True, type=Path)
    p.add_argument('--source-disk', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    p.add_argument('--effects', action='store_true')
    a = p.parse_args()
    out = a.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    disk = out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    install(disk, [('moan-probe', (ROOT / 'iso_root/programs/moan-probe').read_bytes())])
    wav = out / 'moan.wav'
    wav.unlink(missing_ok=True)
    os.environ['SEXOS_GATE_DEVICES'] = (
        f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 '
        '-device virtio-sound-pci,audiodev=snd0')
    from quil_autosave_gate import boot
    g = boot(a.iso, disk, out, 'moan', a.resolution)
    try:
        g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2', 0, 120)
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 300)
        g['focus'](153)
        time.sleep(.5)
        start = len(g['text']())
        for k in 'run moan-probe' + (' fx' if a.effects else ''):
            g['q'].key({' ': 'spc', '-': 'minus'}.get(k, k))
            time.sleep(.05)
        g['q'].key('ret')
        g['wait'](r'\[moan.probe.ok\] frames=96000 played=96000\n', start, 120)
        assert '[moan.probe.fail]' not in g['text']()
        time.sleep(.5)
        g['q'].cmd('quit')
        g['proc'].wait(30)
        result = dict(ok=True, effects=a.effects, channels=(inspect_effects(wav) if a.effects else inspect(wav)))
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(result)
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
