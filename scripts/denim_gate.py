#!/usr/bin/env python3
"""Compressed memory (Denim, J5 D2): a program's cold heap pages go out to Denim and come
back on touch with their data intact. Needs an ISO built with SEXOS_DENIM_EAGER=1, so
Denim takes cold pages without waiting for memory pressure.

Heaptest keeps 8 MiB of mixed pages (zero, same-filled, text, noise; key K), then idles.
Denim must take at least half of them out; reading them back (key V) faults them in, and
every byte must match. Close must still retire the run cleanly with pages out, and
nothing may fault or panic."""
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
  # Idle while Denim's scans find the kept pages cold and take them.
  outs=0
  for _ in range(90):
   r=re.findall(r'\[denim\.round\] outs=(\d+) ins=(\d+) pages_out=(\d+) slab_bytes=(\d+) store_pages=(\d+) saved_pages=(-?\d+)',t()[s:])
   if r and int(r[-1][2])>=1024:outs=int(r[-1][2]);break
   time.sleep(2)
  assert outs>=1024,f'Denim took only {r[-1] if r else None} pages out'
  out_round=r[-1]
  g['focus'](sid);time.sleep(.5);s=len(t());q.key('v')
  v=g['wait'](rf'\[heaptest\.verify\] target={target} task={task} pages=2048 bad=(\d+) ticks=(\d+)',s,240)
  assert v[1]=='0',f'{v[1]} kept pages came back wrong'
  ins=re.findall(r'\[denim\.in\] target=\d+ page=0x[0-9a-f]+ fault=1 ins=(\d+)',t()[s:])
  assert ins and int(ins[-1])>=512,f'pages did not come back through faults: {ins[-1:]}'
  # Close with pages still out: the run retires and every frame comes back.
  g['wait'](r'\[denim\.round\] ',s,60)
  g['focus'](sid);time.sleep(.3);s=len(t());q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
  for bad in ('KERNEL PANIC','panicked at','[denim.lost]','[denim.damaged]','[denim.panic]'):assert bad not in t(),bad
  assert not re.search(r'fault.kill \[paging.fault.terminal\] pd=\d+',t()),'a task was killed by a fault'
  print({'ok':True,'target':target,'out':out_round,'verify_ticks':int(v[2]),'faults_in':int(ins[-1])})
 finally:g['stop']()
if __name__=='__main__':main()
