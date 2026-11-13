#!/usr/bin/env python3
"""Out of memory: a program that takes all the memory programs may have and keeps it
(Heap test's A) drives pressure to critical. After a grace period the Shell ends the
program run holding the most memory, says so in a notification, memory pressure falls
back to normal, and the desktop still answers (Spindle runs a command, a new program
starts). No service is ended."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('heaptest','monitor')])
 g=boot(a.iso,disk,a.out,'oom',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());run('run heaptest')
  m=g['wait'](r'\[heaptest\.ok\] target=(\d+) task=(\d+)',s,180);target,task=m[1],m[2]
  g['wait'](rf'\[heaptest.ready\] target={target} task={task}',s,60)
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)[1]);time.sleep(2)
  s=len(t());g['focus'](surface);time.sleep(.5);q.key('a')
  filled=g['wait'](rf'\[heaptest\.(all|fail)\] target={target} task={task} ([^\n]*)\n',s,900)
  assert filled[1]=='all',filled[0]
  crit=g['wait'](r'\[shell\.oom\.critical\] tick=(\d+)',s,120)
  end=g['wait'](rf'\[shell\.oom\.end\] target={target} task={task} pages=(\d+) ok=1',s,120)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  g['wait'](r'\[memory\.pressure\] level=0 ',s,120)
  shot=a.out/'oom.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  try:
   from PIL import Image;Image.open(shot).save(a.out/'oom.png')
  except Exception:pass
  # The desktop still works: a command answers and a new program starts.
  s=len(t());run('crashes');g['wait'](r'\[spindle.crashes\] count=\d+',s,60)
  s=len(t());run('run monitor');g['wait'](r'\[monitor\.ready\] target=\d+',s,180)
  text=t()
  ended=re.findall(r'\[shell\.oom\.end\] target=(\d+)',text);assert ended==[target],f'ended {ended}, want only {target}'
  services=[l for l in re.findall(r'fault.kill \[[^\]]+\] pd=(\d+)',text) if int(l)<=15];assert not services,services
  assert 'KERNEL PANIC' not in text
  print({'ok':True,'target':int(target),'filled':filled[2],'pages_ended':int(end[1]),'mib_ended':int(end[1])*4//1024})
 finally:g['stop']()
if __name__=='__main__':main()
