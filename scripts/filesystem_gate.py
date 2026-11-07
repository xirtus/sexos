#!/usr/bin/env python3
"""Real files on a real-size disk: directories, contents, and they survive a reboot and a power cut."""
import argparse,os,re,shutil,signal,time
from pathlib import Path
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
GIB=1024*1024*1024
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal'}
def session(a,disk,tag):
 g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
 g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
 g['focus'](153);time.sleep(.5)
 def line(text):
  for k in text:
   q.key(KEYS.get(k,k));time.sleep(.03)
  q.key('ret')
 def run(text,want=None,timeout=60):
  """Send one command and wait for its final result."""
  s=len(t());line(text)
  if want:return g['wait'](want,s,timeout)
  time.sleep(.3);return None
 def cat(path):
  s=len(t());line(f'cat {path}');g['wait'](r'\[spindle.fs.result\] kind=7 status=\d+ value=\d+',s,60);time.sleep(1)
  return re.findall(r'\[spindle.fs.cat\] ([^\n]*)',t()[s:])
 def ls(path=''):
  s=len(t());line(f'ls {path}'.strip());g['wait'](r'\[spindle.fs.result\] kind=2 ',s,60);time.sleep(1)
  return re.findall(r'\[spindle.fs.line\] ([^\n]*)',t()[s:])
 return g,t,q,run,cat,ls
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(GIB)
 OK=r'\[spindle.fs.line\] ok'
 # First boot: format, directories and files.
 g,t,q,run,cat,ls=session(a,disk,'fs1')
 try:
  run('df',r'\[spindle.fs.line\] \d+ MiB free of \d+ MiB')
  assert '[files.fs.formatted]' in t(),'no format on a new disk'
  total=int(re.search(r'MiB free of (\d+) MiB',t())[1]);assert total>=400,total
  run('mkdir notes',OK);run('mkdir notes/work',OK)
  run('write notes/a.txt hello from a file',r'\[spindle.fs.result\] kind=8 status=0 value=18')
  run('append notes/a.txt second line',r'\[spindle.fs.result\] kind=8 status=0 value=12')
  assert cat('notes/a.txt')==['hello from a file','second line'],t()[-400:]
  run('cd notes');time.sleep(1);run('write work/b.txt deep',r'\[spindle.fs.result\] kind=8 status=0')
  assert sorted(ls())==['a.txt  30','work/'],ls()
  run('mv a.txt work/c.txt',OK);assert cat('work/c.txt')==['hello from a file','second line']
  s=len(t());run('rm work',r'\[spindle.fs.line\] directory not empty')
  run('cat missing.txt',r'\[spindle.fs.line\] not found')
  run('cd ..');time.sleep(1);run('rm notes/work/b.txt',OK)
  assert 'KERNEL PANIC'not in t()
 finally:g['stop']()
 # Second boot: all there, found by mounting, not formatting.
 g,t,q,run,cat,ls=session(a,disk,'fs2')
 try:
  assert cat('notes/work/c.txt')==['hello from a file','second line']
  assert '[files.fs.mounted]' in t() and '[files.fs.formatted]' not in t(),'mount failed'
  assert ls('notes/work')==['c.txt  30']
  # Commit after commit, then pull the plug with no warning.
  done=0
  for i in range(30):
   run(f'write notes/n{i}.txt value {i}',r'\[spindle.fs.result\] kind=8 status=0');done=i+1
  run('write notes/last.txt in flight')
  time.sleep(.05)
  proc=g.get('proc')
 finally:
  # An instant stop, as a power cut would be.
  os.system(f"pkill -9 -f {str(disk)!r}")
  try:g['stop']()
  except Exception:pass
 # Third boot: the disk must mount, and everything committed must be there.
 g,t,q,run,cat,ls=session(a,disk,'fs3')
 try:
  assert '[files.fs.formatted]' not in t()
  names=ls('notes')
  for i in range(done):assert f'n{i}.txt  {len(f"value {i}")+1}' in names,(i,names)
  assert cat(f'notes/n{done-1}.txt')==[f'value {done-1}']
  run('write notes/after.txt still works',r'\[spindle.fs.result\] kind=8 status=0')
  assert cat('notes/after.txt')==['still works']
  assert 'KERNEL PANIC'not in t()
  print({'ok':True,'total_mib':total,'committed_before_cut':done})
 finally:g['stop']()
if __name__=='__main__':main()
