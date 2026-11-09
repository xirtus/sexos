#!/usr/bin/env python3
"""A program runs on several cores: `run threads` draws a picture on one task and again
in bands across workers on other cores, and both match. Workers that spin forever end
when the program is closed, and the row's memory is retired only after they have; a
worker that panics ends the whole program; the row runs again after each."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/threads').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('threads',elf)])
 g=boot(a.iso,disk,a.out,'threads',a.resolution);t=g['text'];q=g['q']
 cores=int(os.environ.get('SEXOS_GATE_CORES','1'))
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started():
  s=len(t());run('run threads')
  m=g['wait'](r'\[threads\.(ok|fail)\] target=(\d+) task=(\d+)([^\n]*)\n',s,600)
  assert m[1]=='ok',f'threads failed: {m[0]}'
  target,task=int(m[2]),int(m[3]);g['wait'](rf'\[threads\.ready\] target={target} task={task}\n',s,60)
  f=dict(kv.split('=')for kv in m[4].split())
  placed={int(c)for c in re.findall(rf'\[program\.worker\.start\] target={target} run={task} worker=\d+ core=(\d+)\n',t()[s:])}
  assert placed,'no worker started'
  return dict(target=target,task=task,cores=int(f['cores']),speedup=int(f['speedup_x100'])/100,worker_cores=sorted(placed),hash=f['hash'])
 def ended(r,s):
  g['wait'](rf'\[app\.private\.retired\] target={r["target"]} task={r["task"]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  first=started()
  if cores>=4:
   assert first['cores']>=2,f'workers kept to one core: {first}'
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'threads.ppm')})
  # Workers that never finish: Close ends the program, every worker, then the row.
  s=len(t());g['focus'](SURFACE[first['target']]);time.sleep(.5);q.key('s')
  m=g['wait'](rf'\[threads\.spinning\] target={first["target"]} task={first["task"]} workers=(\d+)\n',s,60);assert int(m[1])>=1
  time.sleep(2);s=len(t());q.key('f11')
  g['wait'](rf'\[threads\.closed\] target={first["target"]} task={first["task"]}\n',s,60)
  g['wait'](rf'\[program\.worker\.end\] target={first["target"]} run={first["task"]} ',s,60)
  ended(first,s)
  # A worker that panics ends the whole program.
  second=started();assert second['hash']==first['hash'],'pictures differ between runs'
  s=len(t());g['focus'](SURFACE[second['target']]);time.sleep(.5);q.key('p')
  g['wait'](rf'\[threads\.panic\] target={second["target"]} task={second["task"]}\n',s,60)
  g['wait'](rf'\[program\.worker\.fault\] target={second["target"]} ',s,60)
  ended(second,s)
  third=started()
  s=len(t());g['focus'](SURFACE[third['target']]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[threads\.closed\] target={third["target"]} task={third["task"]}\n',s,60);ended(third,s)
  text=t();assert 'KERNEL PANIC' not in text and '[threads.fail]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':[first,second,third]})
 finally:g['stop']()
if __name__=='__main__':main()
