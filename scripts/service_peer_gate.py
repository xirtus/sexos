#!/usr/bin/env python3
"""Actual cap-scoped peer snapshots across an intentional same-task Shell restart.
Requires a sealed ISO built with SEXOS_CRASH_PROBE=1. No build occurs in this gate.
"""
import argparse, hashlib, json, os, re, shutil, subprocess, time
from pathlib import Path
from quil_autosave_gate import boot
ROOT = Path(__file__).resolve().parents[1]
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    p=argparse.ArgumentParser()
    for name in ('iso','source-disk','out'): p.add_argument('--'+name,required=True,type=Path)
    a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
    if any(a.out.iterdir()): raise RuntimeError('fresh evidence directory required')
    disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
    with disk.open('r+b') as f:f.truncate(1024**3)
    result={'ok':False,'iso_sha256':sha(a.iso),'cores':os.environ.get('SEXOS_GATE_CORES','1'),'source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'observations':[]}
    result['sources']={str(p.relative_to(ROOT)):sha(p) for p in [Path(__file__),ROOT/'kernel/src/service_restart.rs',ROOT/'kernel/src/crash_log.rs',ROOT/'crates/sex-pdx/src/crash_log.rs',ROOT/'apps/spindle/src/main.rs',ROOT/'scripts/quil_autosave_gate.py']}
    g=None
    try:
        g=boot(a.iso.resolve(),disk,a.out,'peer','1280x800')
        g['wait'](r'\[app.private.retired\] target=41 ',0,180)
        original=g['wait'](r'\[boot.service.restartable\] pd=3 task=(\d+) pages=\d+ entry=0x[0-9a-f]+\n',0,30)
        result['task_before']=int(original[1])
        def command(line,pattern,timeout=60):
            g['focus'](153);time.sleep(.2);start=len(g['text']())
            for char in line:g['q'].key({' ':'spc','-':'minus'}.get(char,char));time.sleep(.05)
            g['q'].key('ret');return g['wait'](pattern,start,timeout)
        def snapshot():
            m=command('service-state shell',r'\[spindle.service.state\] service=shell (?:epoch=(\d+) state=(\d+)|error=(\d+))\n')
            item={'epoch':int(m[1]) if m[1] else None,'state':int(m[2]) if m[2] else None,'error':int(m[3]) if m[3] else None}
            result['observations'].append(item);return item
        def live_snapshot(epoch):
            for _ in range(10):
                item=snapshot()
                if item['error']==5:time.sleep(.1);continue
                assert item=={'epoch':epoch,'state':1,'error':None},item
                return item
            raise RuntimeError('peer snapshot remained contended')
        first=live_snapshot(1)
        start=len(g['text']())
        command('crash shell',r'\[spindle.crash.probe\] target=3\n')
        restarted=g['wait'](r'\[service.restarted\] pd=3 task=(\d+) restarts=1 pages=(\d+) dropped_messages=(\d+) admitted=1\n',start,120)
        assert int(restarted[1])==int(original[1]),restarted[0]
        # The restarted service's own legacy count must also remain correct;
        # startup can run before another core drops the restart mutex.
        g['wait'](r'\[shell.restarted\] restarts=1 adopted=\d+ windows=\d+\n',start,120)
        last=live_snapshot(2)
        # Complete records with this tag are protocol, not ignorable noise.
        captured=g['text']();complete=captured[:captured.rfind('\n')+1]
        for line in complete.splitlines():
            if line.startswith('[spindle.service.state]'):
                assert re.fullmatch(r'\[spindle.service.state\] service=shell (?:epoch=[1-9]\d* state=[1-7]|error=[1-5])',line),line
        result.update(ok=True,service_pd=3,same_task=int(restarted[1]),epoch_before=1,epoch_after=2,legacy_self_count=1,scope='native_cap_scoped_peer_snapshot_and_same_task_restart_not_recording_authority')
    except BaseException as exc:result['error']=repr(exc);raise
    finally:
        if g:g['stop']()
        (a.out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,sort_keys=True))
if __name__=='__main__':main()
