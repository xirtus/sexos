#!/usr/bin/env python3
"""The disk program store holds more than one directory sector of programs, and a store program takes arguments: `run jot @todo.txt` runs Jot from the second directory sector, given that one file."""
import argparse,shutil,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
from install_program import install,PER_SECTOR
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','@':None}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 # More programs than one sector holds; Jot is the last, in the second sector.
 fillers=[(f'filler{i}',bytes([i+1])*700) for i in range(PER_SECTOR+3)]
 install(disk,fillers+[('jot',(ROOT/'iso_root/programs/jot').read_bytes())])
 g=boot(a.iso,disk,a.out,'store',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   else:q.key(KEYS.get(k) or k)
   time.sleep(.06)
 def expect(text,pattern,timeout=60):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout),s
 def close(m):
  s=len(t());g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  expect('write todo.txt buy milk',r'\[spindle.fs.result\] kind=8 status=0')
  # A bare name from the second directory sector.
  _,s=expect('run jot',r'\[jot.ready\]')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[jot.loaded\] bytes=0 ',s,60);g['wait'](rf'\[files.program.bound\] target={m[1]} task={m[2]} root=/data/jot',s,60);time.sleep(1);close(m)
  # The same store program with an argument: it opens the file it was given.
  _,s=expect('run jot @todo.txt',r'\[jot.ready\]')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[jot.loaded\] bytes=9\b',s,60);g['wait'](rf'\[files.program.bound\] target={m[1]} task={m[2]} root=/data/jot',s,60);time.sleep(1);close(m)
  # A name in neither /bin nor the store is still refused.
  expect('run nosuch @todo.txt',r'\[shell.program.denied\]')
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'store_programs':len(fillers)+1})
 finally:g['stop']()
if __name__=='__main__':main()
