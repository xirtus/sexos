#!/usr/bin/env python3
"""Ending a program from the system monitor (Mesh): choose its line, press Delete, and
the Shell ends it (system_stats::OP_END_PROGRAM). A system service's line is refused."""
import argparse,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
# The system monitor (Mesh, formerly Monitor); it logs [monitor.*] either way.
MESH='mesh' if (ROOT/'iso_root/programs/mesh').exists() else 'monitor'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('fortune',MESH)])
 g=boot(a.iso,disk,a.out,'end',a.resolution);t=g['text'];q=g['q']
 def run(line,tag):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret');m=g['wait'](rf'\[{tag}\.ready\] target=(\d+) task=(\d+)',s,180);time.sleep(1.5)
  return int(m[1]),int(m[2]),int(re.findall(rf'\[shell.program.launch\] target={m[1]} task={m[2]} surface=(\d+)',t())[-1])
 def choose(target):
  for _ in range(60):
   s=len(t());q.key('down');m=g['wait'](r'\[monitor\.choose\] target=(\d+)\n',s,30)
   if int(m[1])==target:return
  raise AssertionError(f'never reached line {target}')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  ft,ftask,_=run('run fortune','fortune')
  mt,mtask,msid=run('run '+MESH,'monitor')
  g['wait'](r'\[monitor\.sample\] ',0,60);time.sleep(1.5)
  g['focus'](msid);time.sleep(.5)
  # A service (Files, domain 11) is refused.
  choose(11);s=len(t());q.key('delete')
  g['wait'](r'\[shell\.program\.end\] caller=\d+ target=11 status=4\n',s,30)
  choose(ft);s=len(t());q.key('delete')
  g['wait'](rf'\[shell\.program\.end\] caller={mt} target={ft} status=0\n',s,30)
  g['wait'](rf'\[app.private.retired\] target={ft} task={ftask} ',s,90)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'ended':ft,'by':mt})
 finally:g['stop']()
if __name__=='__main__':main()
