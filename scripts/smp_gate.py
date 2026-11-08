#!/usr/bin/env python3
"""Every core comes up with its own setup, and the desktop works with all of them present."""
import argparse,os,shutil,time
from pathlib import Path
CORES=os.environ.setdefault('SEXOS_GATE_CORES','4')
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'smp',a.resolution);t=g['text'];q=g['q']
 try:
  m=g['wait'](rf'\[smp.online\] cores=(\d+) present=(\d+)',0,60);assert m[1]==CORES==m[2],f'online {m[1]} of {m[2]}, wanted {CORES}'
  cores={int(c) for c in __import__('re').findall(r'\[smp.core.online\] core=(\d+)',t())};assert cores==set(range(1,int(CORES))),cores
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret');
  # Closing a program unmaps its pages, which must reach every core.
  m=g['wait'](r'\[hello.ready\] target=(\d+) task=(\d+)',s,60);s=len(t());time.sleep(2);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,60)
  g['focus'](153);time.sleep(.5)
  for k in 'records':q.key(k);time.sleep(.05)
  s=len(t());q.key('ret');shots=int(g['wait'](r'tlb_shootdowns=(\d+)',s,60)[1])
  # One batched request now covers a whole run's pages; before, one went per page.
  assert shots>=1,f'{shots} shootdowns for a retired program'
  print({'ok':True,'cores':int(CORES),'tlb_shootdowns':shots})
 finally:g['stop']()
if __name__=='__main__':main()
