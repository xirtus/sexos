#!/usr/bin/env python3
"""Files repairs a damaged SexOS disk at start.

First boot: a file is written and kept. Then, on the host, the newest commit's root node
is damaged, as a bad sector or a torn write would. `sexfs-fsck` must call the disk
damaged. Second boot: Files finds it will not mount, repairs it (salvaging from the
commit before), and the file reads back. Afterwards `sexfs-fsck --deep` finds the disk
clean."""
import argparse,os,shutil,struct,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
BASE=131072*512
BLOCK=4096
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def newest_root(disk):
 with open(disk,'rb') as f:
  best=None
  for slot in (0,1):
   f.seek(BASE+slot*BLOCK);b=f.read(BLOCK)
   if struct.unpack_from('<Q',b,0)[0]!=0x3142_5346_5845_53:continue
   gen,root=struct.unpack_from('<QQ',b,16)
   if best is None or gen>best[0]:best=(gen,root)
 return best
def fsck(tool,disk,*extra):
 r=subprocess.run([str(tool),*extra,str(disk)],capture_output=True,text=True);print(r.stdout.strip());return r.returncode
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 subprocess.run(['cargo','build','-q','--release','--manifest-path',str(ROOT/'tools/sexfs-fsck/Cargo.toml')],check=True)
 tool=ROOT/'tools/sexfs-fsck/target/release/sexfs-fsck'
 from quil_autosave_gate import boot
 def session(tag,body):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def keys(text):
   for k in text:
    shift=k.isupper() or k in ':_'
    code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
    if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
    else:q.key(code)
    time.sleep(.06)
  def expect(text,pattern,timeout=60):
   s=len(t());g['focus'](153);time.sleep(.5);keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
  try:
   g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
   body(g,t,expect)
   assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  finally:g['stop']()
 def first(g,t,expect):
  expect('mkdir keep',r'\[spindle.fs.result\] kind=3 status=0')
  expect('write keep/note.txt survives a damaged disk',r'\[spindle.fs.result\] kind=8 status=0')
  expect('cat keep/note.txt',r'\[spindle.fs.cat\] survives a damaged disk')
  # One more commit after the note: damaging the newest commit then costs only this.
  expect('mkdir later',r'\[spindle.fs.result\] kind=3 status=0')
  time.sleep(3)
 session('repair1',first)
 assert fsck(tool,disk,'--deep')==0,'the disk was damaged before the test damaged it'
 gen,root=newest_root(disk)
 with disk.open('r+b') as f:
  f.seek(BASE+root*BLOCK+200);b=bytearray(f.read(16))
  for i in range(len(b)):b[i]^=0xA5
  f.seek(BASE+root*BLOCK+200);f.write(b)
 print(f'damaged generation {gen} root node {root}')
 assert fsck(tool,disk)==1,'sexfs-fsck did not notice the damage'
 def second(g,t,expect):
  expect('cat keep/note.txt',r'\[spindle.fs.cat\] survives a damaged disk',180)
  m=g['wait'](r'\[files.fs.repaired\] generation=(\d+) bad_nodes=(\d+) ',0,10)
  assert int(m[2])>=1,m[0]
  expect('write keep/after.txt written after repair',r'\[spindle.fs.result\] kind=8 status=0')
  expect('cat keep/after.txt',r'\[spindle.fs.cat\] written after repair')
  time.sleep(3)
 session('repair2',second)
 assert fsck(tool,disk,'--deep')==0,'the repaired disk is not clean'
 print({'ok':True,'damaged_generation':gen})
if __name__=='__main__':main()
