#!/usr/bin/env python3
"""Responsiveness: time from a key press to the pixels that show it, idle and with four
programs computing flat out (`run bench spin`). Each key typed into Spindle is timed
from the host's key event to Display committing Spindle's new text to the screen
([display.text.committed] for Spindle's surface). Reports p50/p95/p99 in ms for both
phases and fails if the loaded p99 is above --max-p99-ms. Under QEMU TCG the numbers
are far above real hardware; they are recorded to track regressions. The plan's target
is p99 < 16 ms on hardware."""
import argparse,json,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','1800')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
COMMIT=r'\[display\.text\.committed\] target=12 '
def pct(xs,p):
 xs=sorted(xs);return xs[min(len(xs)-1,int(round(p*(len(xs)-1))))]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--keys',type=int,default=40);p.add_argument('--busy',type=int,default=4);p.add_argument('--max-p99-ms',type=float,default=400.0);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('bench',(ROOT/'iso_root/programs/bench').read_bytes())])
 g=boot(a.iso,disk,a.out,'latency',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def measure(n):
  g['focus'](153);time.sleep(1)
  out=[]
  for i in range(n):
   s=len(t());k='abcdefghijklmnopqrstuvwxyz'[i%26]
   t0=time.monotonic();q.key(k)
   while not re.search(COMMIT,t()[s:]):
    if time.monotonic()-t0>10:raise TimeoutError(f'key {i} never reached the screen')
    time.sleep(.0005)
   out.append((time.monotonic()-t0)*1000);time.sleep(.08)
  # Clear the line again so the next command starts clean.
  for _ in range(n):q.key('backspace');time.sleep(.02)
  return out
 def summary(xs):return {'keys':len(xs),'p50_ms':round(pct(xs,.5),1),'p95_ms':round(pct(xs,.95),1),'p99_ms':round(pct(xs,.99),1),'max_ms':round(max(xs),1)}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  idle=measure(a.keys)
  s=len(t())
  for i in range(a.busy):
   # One at a time: each must be running before the next is asked for.
   type_line('run bench spin 600')
   deadline=time.time()+240
   while len(re.findall(r'\[bench\.spin\.start\]',t()[s:]))<i+1:
    assert time.time()<deadline,f'busy program {i+1} never started';time.sleep(1)
  time.sleep(3)
  loaded=measure(a.keys)
  still=len(re.findall(r'\[bench\.spin\.done\]',t()[s:]))
  res={'idle':summary(idle),'loaded':summary(loaded),'busy_programs':a.busy,'spins_finished_during':still,'cores':os.environ.get('SEXOS_GATE_CORES','1')}
  (a.out/'latency.json').write_text(json.dumps(res))
  assert still==0,'the load ended before the loaded keys were timed'
  assert res['loaded']['p99_ms']<=a.max_p99_ms,f"loaded p99 {res['loaded']['p99_ms']} ms over {a.max_p99_ms}"
  text=t();assert 'KERNEL PANIC' not in text
  print({'ok':True,**res})
 finally:g['stop']()
if __name__=='__main__':main()
