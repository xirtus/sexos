#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Real Moan UI -> Files save picker -> FFmpeg WAV/FLAC/ShineMP3. Extract actual
SexFS files read-only and independently ffprobe/decode them; also cancel a long
WAV export through Close and retain only a partial, never a false success."""
import argparse
import array
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from install_program import install
from quil_autosave_gate import boot
ROOT = Path(__file__).resolve().parents[1]


def inspect(path, expected_frames):
    probe = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries',
        'stream=codec_name,sample_rate,channels,duration:format=duration', '-of', 'json', str(path)], text=True))
    stream, = probe['streams']
    ext = path.suffix[1:]
    assert stream['codec_name'] == dict(wav='pcm_s16le', flac='flac', mp3='mp3')[ext], probe
    assert int(stream['sample_rate']) == 48000 and stream['channels'] == 2, probe
    decoded = subprocess.check_output(['ffmpeg', '-v', 'error', '-i', str(path), '-f', 's16le', '-acodec', 'pcm_s16le', '-'])
    if ext != 'mp3':
        assert len(decoded) == expected_frames * 4, (ext, len(decoded), expected_frames)
        samples = array.array('h', decoded)
        if sys.byteorder != 'little': samples.byteswap()
        for i in range(expected_frames):
            value = (4000 if i % 120 < 60 else -4000) if i < 96000 else 0
            assert samples[i*2] == value and samples[i*2+1] == value, (ext, i, samples[i*2:i*2+2], value)
    else:
        assert abs(len(decoded)//4 - expected_frames) < 2304, ('MP3 sample count', len(decoded)//4)
        samples = array.array('h', decoded)
        if sys.byteorder != 'little': samples.byteswap()
        for channel in range(2):
            mid = samples[channel::2][12000:60000]
            crossings = sum(a <= 0 < b for a,b in zip(mid,mid[1:]))
            assert abs(crossings - 400) < 8, ('MP3 pitch', channel, crossings)
            energy = sum(v*v for v in mid)/len(mid)
            assert 2500**2 < energy < 5500**2, ('MP3 level', channel, energy)
            assert max(abs(v) for v in samples[channel::2][144000:180000]) < 32, 'MP3 silence tail is not silent'
    return dict(codec=stream['codec_name'], decoded_frames=len(decoded)//4, bytes=path.stat().st_size,
        sha256=hashlib.sha256(path.read_bytes()).hexdigest(), decoded_sha256=hashlib.sha256(decoded).hexdigest())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('iso','source-disk','out'): p.add_argument('--'+name, required=True, type=Path)
    p.add_argument('--resolution', default='1280x800')
    a=p.parse_args(); out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    assert not any(out.iterdir()), 'fresh output directory required'
    disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
    with disk.open('r+b') as f:f.truncate(1024**3)
    install(disk,[(name,(ROOT/'iso_root/programs'/name).read_bytes()) for name in ('moan','files')])
    extractor=out/'sexfs-extract'
    subprocess.run(['rustc','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
    os.environ.setdefault('SEXOS_GATE_BOUND','1200')
    g=boot(a.iso.resolve(),disk,out,'moan-export',a.resolution);q=g['q'];text=g['text'];rw,rh=map(int,a.resolution.split('x'))
    def keys(value):
        for c in value:
            q.key({' ':'spc','/':'slash','.':'dot','-':'minus'}.get(c,c));time.sleep(.06)
    def chord(*names):
        q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':name}}}
            for down,part in ((True,names),(False,tuple(reversed(names)))) for name in part]})
    def say(value):
        g['focus'](153);time.sleep(.3);start=len(text());keys(value);q.key('ret');return start
    try:
        g['wait'](r'\[app.private.retired\] target=41 ',0,180)
        start=say('files')
        f=g['wait'](r'\[shell.program.launch\] target=\d+ task=\d+ surface=(\d+)',start,120);files_surface=int(f[1])
        g['wait'](r'\[files.app.ready\] target=\d+ task=\d+ root=/ portal=1',start,120)
        start=say('run moan')
        m=g['wait'](r'\[moan.ready\] target=(\d+) task=(\d+)',start,300);target,task=m[1],m[2]
        surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',start,60)[1])
        g['wait'](r'\[moan.control\] name=range_end ',start,60);time.sleep(1)
        def control(name):
            positions=re.findall(rf'\[moan.control\] name={name} x=(-?\d+) y=(-?\d+)',text()[start:]);assert positions,name
            x,y=map(int,positions[-1]);tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text())[-1]));x+=tile[0]+8;y+=tile[1]+36
            q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
            for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.15)
        def choose(filename):
            g['wait'](r'\[files.app.question\] program=moan mode=Save',cursor,60)
            g['focus'](files_surface);time.sleep(.3);chord('ctrl','s');time.sleep(.3)
            # The existing Files filename editor is not a Weft field: Ctrl+A
            # does not select its suggestion. Clear the short public hint.
            for _ in range(16):q.key('backspace');time.sleep(.04)
            keys(filename);q.key('ret')
            g['wait'](rf'\[files.pick.granted\] row=\d+ path=/{re.escape(filename)}',cursor,60)
            g['focus'](surface)
        g['focus'](surface);time.sleep(.3);control('Tone')
        g['wait'](r'\[moan.project\] tracks=1 frames=96000 ',start,60)
        results={}
        for digit,ext in [('1','wav'),('2','flac'),('3','mp3')]:
            cursor=len(text());g['focus'](surface);chord('ctrl','shift',digit);time.sleep(.3);chord('ctrl','shift','e')
            choose('export.'+ext)
            made=g['wait'](rf'\[moan.export\] format={ext} frames=192000 bytes=(\d+) synced=1',cursor,300)
            results[ext]=dict(expected_bytes=int(made[1]),expected_frames=192000)
        # Long silent gap makes Close deterministic even on fast codecs/hosts.
        cursor=len(text());control('position');chord('ctrl','a');keys('3600');control('Tone')
        g['wait'](r'\[moan.project\] tracks=2 frames=172896000 ',cursor,60)
        cursor=len(text());chord('ctrl','shift','1');chord('ctrl','shift','e');choose('partial.wav')
        g['wait'](r'\[moan.export.started\] format=wav frames=172992000',cursor,60)
        q.key('f11')
        g['wait'](r'\[moan.export.cancelled\] partial_retained=1',cursor,120)
        assert not re.search(r'\[moan.export\] format=wav ',text()[cursor:]), 'cancelled export reported success'
        # A dirty project shows the standard Cancel/Discard prompt. Explicitly
        # choose Discard, rather than bypassing the unsaved-project guard.
        time.sleep(.4);q.key('tab');q.key('ret')
        g['wait'](rf'\[app.private.retired\] target={target} task={task} ',cursor,120)
        q.cmd('screendump',{'filename':str(out/'after-export.ppm')})
        g['stop']();g=None
        for ext,info in results.items():
            path=out/('export.'+ext);subprocess.run([str(extractor),str(disk),'/export.'+ext,str(path)],check=True)
            assert path.stat().st_size==info['expected_bytes']
            info.update(inspect(path,info['expected_frames']))
        partial=out/'partial.wav';subprocess.run([str(extractor),str(disk),'/partial.wav',str(partial)],check=True)
        assert partial.stat().st_size<172992000*4,partial.stat().st_size
        result=dict(ok=True,formats=results,close_cancelled=True,partial_bytes=partial.stat().st_size)
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result)
    finally:
        if g:g['stop']()


if __name__=='__main__':main()
