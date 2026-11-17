#!/usr/bin/env python3
"""Compressed memory (Denim, J5 D3): a program whose heap is over twice the machine's RAM
still completes. Needs an ordinary ISO (not eager): Denim acts only on memory pressure.

Heaptest's D key fills 2.5x the memory free at its start, a mebibyte at a time (mostly
ordinary heap: zeros, filled runs and text, an eighth noise), waiting politely while memory
is critical; then it reads every page back. Every byte must match, the heap must reach at
least twice the guest's RAM, Denim must have taken pages out and brought them back, and
nothing may panic, be lost, or be ended for running out of memory.

  SEXOS_GATE_MEMORY=1G  python3 scripts/denim_double_gate.py --iso ... --source-disk ... --out ...
  SEXOS_GATE_MEMORY=12G python3 scripts/denim_double_gate.py ..."""
import argparse,os,re,shutil,time
os.environ.setdefault('SEXOS_GATE_MEMORY','1G')
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def mib(size):
 n=float(size[:-1]);return int(n*1024 if size[-1] in 'Gg' else n)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--minutes',type=float,default=75);a=p.parse_args()
 ram=mib(os.environ['SEXOS_GATE_MEMORY']);os.environ.setdefault('SEXOS_GATE_BOUND',str(int(a.minutes*60)+1200))
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('heaptest',(ROOT/'iso_root/programs/heaptest').read_bytes())])
 g=boot(a.iso,disk,a.out,'denim-double',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[denim\.ready\] ok=1 rows=\d+ eager=0 ',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,180);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in 'run heaptest':q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
  m=g['wait'](r'\[heaptest\.ok\] target=(\d+) task=(\d+)',s,240);target,task=m[1],m[2]
  g['wait'](rf'\[display.text.committed\] target={target} ',s,60);time.sleep(1)
  sid=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  g['focus'](sid);time.sleep(.5);s=len(t());q.key('d')
  st=g['wait'](rf'\[heaptest\.double\.start\] target={target} task={task} free_pages=(\d+) goal_mib=(\d+)',s,60)
  goal=int(st[2]);assert goal>=2*ram,f'goal {goal} MiB is under twice the RAM ({ram} MiB)'
  began=time.monotonic()
  while True:
   d=re.search(rf'\[heaptest\.double\] target={target} task={task} mib=(\d+) free_pages=\d+ bad=(\d+) fill_ticks=(\d+) verify_ticks=(\d+) waited=(\d+)',t()[s:])
   if d:break
   for bad in ('KERNEL PANIC','panicked at','[heaptest.fail]','[shell.oom.end]','[denim.lost','[denim.damaged]','[denim.panic]'):assert bad not in t()[s:],bad
   assert time.monotonic()-began<a.minutes*60,'heaptest did not finish: '+(re.findall(r'\[heaptest\.double\.progress\][^\n]*',t()[s:])[-1:] or ['no progress'])[0]
   time.sleep(5)
  assert d[2]=='0',f'{d[2]} pages came back wrong'
  assert int(d[1])>=2*ram,f'heap reached {d[1]} MiB, under twice the RAM'
  rounds=re.findall(r'\[denim\.round\] outs=(\d+) ins=(\d+) pages_out=(\d+) slab_bytes=(\d+) store_pages=(\d+) [^\n]* faults=(\d+) recalled=(\d+) returned=(\d+)',t()[s:])
  assert rounds,'Denim never took a page out'
  last=rounds[-1];peak=max(int(r[2]) for r in rounds)
  assert int(last[0])>0 and int(last[5])>0,last
  # Close: the run retires with its pages out, and Denim gives its store back.
  g['focus'](sid);time.sleep(.3);s2=len(t());q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s2,600)
  for bad in ('KERNEL PANIC','panicked at','[denim.damaged]','[denim.panic]','[shell.oom.end]'):assert bad not in t(),bad
  store=max(int(r[4]) for r in rounds)
  print({'ok':True,'ram_mib':ram,'heap_mib':int(d[1]),'times_ram':round(int(d[1])/ram,2),'fill_s':int(d[3])/1000,'verify_s':int(d[4])/1000,'waited_s':int(d[5])/1000,
   'outs':int(last[0]),'ins':int(last[1]),'faults':int(last[5]),'peak_pages_out':peak,'peak_store_pages':store,'ratio':round(peak/max(store,1),2),'returned':int(last[7])})
 finally:g['stop']()
if __name__=='__main__':main()
