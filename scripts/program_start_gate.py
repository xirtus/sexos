#!/usr/bin/env python3
"""Program start time: from Enter on `run NAME` in Spindle to the program saying it is
ready ([NAME.ready]), and from Spindle taking the command to ready, for a few programs of different sizes, several runs each (the
first run of a program reads it from disk; later ones may reuse a row). Records the
host-side milliseconds under QEMU, with each program's size, in start-times.json, and
fails only if a start takes longer than --max-ms. The plan's hardware target is under
100 ms for Jot."""
import argparse,json,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','1800')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
PROGRAMS=['fortune','monitor','shapes','jot']
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--runs',type=int,default=3);p.add_argument('--max-ms',type=float,default=30000.0);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elfs={n:(ROOT/'iso_root/programs'/n).read_bytes()for n in PROGRAMS};install(disk,list(elfs.items()))
 g=boot(a.iso,disk,a.out,'start',a.resolution);t=g['text'];q=g['q']
 res={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for name in PROGRAMS:
   times=[];starts=[]
   for _ in range(a.runs):
    g['focus'](153);time.sleep(.5)
    for k in f'run {name}':q.key('spc' if k==' ' else k);time.sleep(.05)
    s=len(t());t0=time.monotonic();q.key('ret')
    # Two clocks: from the Enter key event, and from Spindle taking the command
    # ([spindle.enter]), which leaves out the keyboard path (timed by latency_gate.py).
    entered=None;deadline=t0+120
    while True:
     txt=t()[s:]
     if entered is None and '[spindle.enter]' in txt:entered=time.monotonic()
     m=re.search(rf'\[{name}\.ready\] target=(\d+) task=(\d+)',txt)
     if m:break
     assert time.monotonic()<deadline,f'{name} never became ready'
     time.sleep(.001)
    done=time.monotonic();times.append(round((done-t0)*1000));starts.append(round((done-(entered or t0))*1000))
    time.sleep(1.5);surface=int(re.findall(rf'\[shell.program.launch\] target={m[1]} task={m[2]} surface=(\d+)',t())[-1])
    s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90);time.sleep(.5)
   res[name]={'bytes':len(elfs[name]),'from_key_ms':times,'from_command_ms':starts,'best_ms':min(starts)}
  (a.out/'start-times.json').write_text(json.dumps(res))
  worst=max(max(r['from_command_ms'])for r in res.values());assert worst<=a.max_ms,f'a start took {worst} ms'
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),**res})
 finally:g['stop']()
if __name__=='__main__':main()
