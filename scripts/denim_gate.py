#!/usr/bin/env python3
"""Compressed memory (Denim, J5 D2): a program's cold heap pages go out to Denim and come
back on touch with their data intact. Needs an ISO built with SEXOS_DENIM_EAGER=1, so
Denim takes cold pages without waiting for memory pressure.

Heaptest keeps 8 MiB of mixed pages (zero, same-filled, text, noise; key K), then idles.
Denim must take at least half of them out; reading them back (key V) faults them in, and
every byte must match. Close must still retire the run cleanly with pages out, and
nothing may fault or panic.

D3: the ISO is also a crash-probe build (SEXOS_CRASH_PROBE=1). A second heaptest run keeps
its pages and lets them go out; then `crash denim` in Spindle ends Denim as a fault would.
Denim starts again, its first call ends the run whose pages were lost, the Shell names it
in a System notice, and the run retires; the new Denim then works as before."""
import argparse,os,re,shutil,time
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('heaptest',(ROOT/'iso_root/programs/heaptest').read_bytes())])
 g=boot(a.iso,disk,a.out,'denim',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[denim\.ready\] ok=1 rows=\d+ eager=1 ',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in 'run heaptest':q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
  m=g['wait'](r'\[heaptest\.ok\] target=(\d+) task=(\d+)',s,180);target,task=m[1],m[2]
  g['wait'](rf'\[display.text.committed\] target={target} ',s,60);time.sleep(1)
  sid=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  g['focus'](sid);time.sleep(.5);s=len(t());q.key('k')
  g['wait'](rf'\[heaptest\.kept\] target={target} task={task} pages=2048',s,120)
  # Count only rounds after the keep: on a slow machine Denim already takes fresh,
  # unwritten heap pages during it, and they fault back as the keep writes them.
  s=len(t())
  # Idle while Denim's scans find the kept pages cold and take them.
  outs=0
  for _ in range(90):
   r=re.findall(r'\[denim\.round\] outs=(\d+) ins=(\d+) pages_out=(\d+) slab_bytes=(\d+) store_pages=(\d+) saved_pages=(-?\d+)',t()[s:])
   if r and int(r[-1][2])>=1024:outs=int(r[-1][2]);break
   time.sleep(2)
  assert outs>=1024,f'Denim took only {r[-1] if r else None} pages out'
  out_round=r[-1]
  # D4: Mesh shows the packed pages, the store they take and the ratio, and heaptest's share.
  mesh='mesh' if (ROOT/'iso_root/programs/mesh').exists() else 'monitor'
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in 'run '+mesh:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
  mp=g['wait'](r'\[monitor\.packed\] packed_pages=(\d+) store_pages=(\d+) ratio_tenths=(\d+)',s,120)
  assert int(mp[1])>=1024 and int(mp[2])>0,mp[0]
  g['wait'](rf'\[monitor\.row\.packed\] pd={target} packed_pages=[1-9]',s,60)
  mt=re.findall(r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',t()[s:])[-1]
  g['focus'](int(mt[2]));time.sleep(.3);s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={mt[0]} task={mt[1]} ',s,90)
  g['focus'](sid);time.sleep(.5);s=len(t());q.key('v')
  v=g['wait'](rf'\[heaptest\.verify\] target={target} task={task} pages=2048 bad=(\d+) ticks=(\d+)',s,240)
  assert v[1]=='0',f'{v[1]} kept pages came back wrong'
  # Faults counted by Denim's rounds, from before verify to after it. ([denim.in] lines
  # come only at powers of two of all ins, so the count may cross none during verify.)
  base=int(re.findall(r'\[denim\.round\] .* faults=(\d+)',t()[:s])[-1])
  faults=0
  for _ in range(60):
   r=re.findall(r'\[denim\.round\] .* faults=(\d+)',t()[s:])
   if r and int(r[-1])-base>=512:break
   time.sleep(1)
  faults=int(r[-1])-base if r else 0
  assert faults>=512,f'only {faults} pages came back through faults'
  # Close with pages still out: the run retires and every frame comes back.
  g['focus'](sid);time.sleep(.3);s=len(t());q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
  for bad in ('KERNEL PANIC','panicked at','[denim.lost]','[denim.damaged]','[denim.panic]'):assert bad not in t(),bad
  assert not re.search(r'fault.kill \[paging.fault.terminal\] pd=\d+',t()),'a task was killed by a fault'
  # Denim starts again with pages out: the run that had them ends and the Shell says so.
  def spindle(line):
   g['focus'](153);time.sleep(.5);s=len(t())
   for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
   q.key('ret');return s
  s=spindle('run heaptest')
  m=g['wait'](r'\[heaptest\.ok\] target=(\d+) task=(\d+)',s,180);t2,k2=m[1],m[2]
  g['wait'](rf'\[display.text.committed\] target={t2} ',s,60);time.sleep(1)
  sid2=int(re.findall(rf'\[shell.program.launch\] target={t2} task={k2} surface=(\d+)',t())[-1])
  g['focus'](sid2);time.sleep(.5);s=len(t());q.key('k')
  g['wait'](rf'\[heaptest\.kept\] target={t2} task={k2} pages=2048',s,120)
  g['wait'](r'\[denim\.round\] [^\n]* runs=1\n',s,120)
  s=spindle('crash denim')
  g['wait'](r'\[crash.probe.service\] pd=51\n',s,60)
  g['wait'](r'\[service.restarted\] pd=51 [^\n]*admitted=1',s,60)
  r=g['wait'](r'\[denim\.ready\] ok=1 [^\n]*restarts=(\d+) ended=(\d+)',s,60)
  assert r[1]=='1' and r[2]=='1',r[0]
  g['wait'](rf'\[denim\.lost\.run\] target={t2} task={k2} pages_out=[1-9]',s,60)
  g['wait'](rf'\[shell\.denim\.lost\] target={t2} task={k2} ',s,60)
  g['wait'](rf'\[app.private.retired\] target={t2} task={k2} ',s,120)
  for bad in ('KERNEL PANIC','panicked at','[denim.damaged]','[denim.panic]'):assert bad not in t(),bad
  print({'ok':True,'target':target,'out':out_round,'mesh':{'packed':int(mp[1]),'store':int(mp[2]),'ratio':int(mp[3])/10},'verify_ticks':int(v[2]),'faults_in':faults,'restart':{'target':t2,'ended':int(r[2])}})
 finally:g['stop']()
if __name__=='__main__':main()
