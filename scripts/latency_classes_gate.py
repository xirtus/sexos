#!/usr/bin/env python3
"""Interactive tasks (input, display, desktop) run ahead of other work on their core,
and a normal task running user code gives way when one wakes. Runs `bench spin` (a
program that computes without a system call for a while) and then the bench itself as
load, moves the pointer all the while, and reads wake-to-run latency per class from the
kernel's [sched.latency] counters. On one core, where the spinner shares the core with
the desktop, interactive wakes must preempt it and wait less than normal ones."""
import argparse,json,os,re,shutil,threading,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
LINE=r'\[sched\.latency\] class=(\w+) wakes=(\d+) avg_cycles=(\d+) max_cycles=(\d+) preemptions=(\d+) hist=\[([\d, ]+)\]\n'
def last(text):
 out={}
 for c,n,a,m,p,h in re.findall(LINE,text):out[c]=(int(n),int(n)*int(a),int(m),int(p),[int(x)for x in h.split(',')])
 return out
def percentile(hist,share):
 # Upper bound of the bucket holding that share of wakes: 1024 << i cycles.
 need=share*sum(hist);seen=0
 for i,c in enumerate(hist):
  seen+=c
  if seen>=need:return 1024<<i
 return 1024<<len(hist)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('bench',(ROOT/'iso_root/programs/bench').read_bytes())])
 g=boot(a.iso,disk,a.out,'latency',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  done=threading.Event();moves=[0]
  def wiggle():
   d=24
   while not done.is_set():
    q.cmd('input-send-event',{'events':[{'type':'rel','data':{'axis':'x','value':d}},{'type':'rel','data':{'axis':'y','value':d}}]})
    d=-d;moves[0]+=1;time.sleep(.03)
  def window(line,end):
   g['focus'](153);time.sleep(.5)
   for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
   s=len(t());before=last(t());moves[0]=0;done.clear()
   q.key('ret');w=threading.Thread(target=wiggle,daemon=True);w.start()
   m=g['wait'](end,s,900);done.set();w.join()
   # The kernel reports its counters every 2048 ticks.
   time.sleep(3)
   after=last(t());res={'moves':moves[0]}
   for c in('interactive','normal'):
    assert c in after,f'no {c} latency counters'
    n0,s0,_,p0,h0=before.get(c,(0,0,0,0,[0]*24));n1,s1,mx,p1,h1=after[c]
    assert n1-n0>=32,f'too few {c} wakes in the window: {n1-n0}'
    h=[b-a for a,b in zip(h0,h1)]
    res[c]={'wakes':n1-n0,'avg_cycles':(s1-s0)//(n1-n0),'p50_below':percentile(h,.5),'p90_below':percentile(h,.9),'p99_below':percentile(h,.99),'max_cycles':mx}
   res['preemptions']=after['interactive'][3]-before.get('interactive',(0,0,0,0))[3]
   return m,res
  m,spin=window('run bench spin',r'\[bench\.spin\.done\][^\n]*\n')
  s=len(t());q.key('f11');g['wait'](r'\[app.private.retired\] target=',s,120);time.sleep(1)
  m,bench=window('run bench',r'\[bench\.(done|fail)\][^\n]*\n');assert m[1]=='done',m[0]
  res={'spin':spin,'bench':bench};(a.out/'latency.json').write_text(json.dumps(res))
  print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),**res})
  if os.environ.get('SEXOS_GATE_CORES','1')=='1':
   i,n=spin['interactive'],spin['normal']
   assert spin['preemptions']>0,f'the spinner never gave way: {spin}'
   assert i['p90_below']<n['p90_below'],f'interactive tasks waited as long as normal ones: {spin}'
 finally:g['stop']()
if __name__=='__main__':main()
