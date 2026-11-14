#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""ripgrep 15.2.0 (apps/rg) and fd 10.5.2 (apps/fd) run on SexOS.

Everything is typed into Spindle, exactly as a person would:

* `run rg --version`: ripgrep starts, and Files makes its folder /data/rg.
* Spindle's own `mkdir`, `write` and `append` put a small tree in that folder:
  t/README.md, t/src/main.rs, t/src/lib.rs, t/docs/notes.md, t/docs/todo.txt and an
  empty folder t/empty (std's Hermit read_dir fails on an empty folder; neither
  program may report it).
* rg: `-n` prints file:line:text; `-i` matches across case; `-c` counts per file;
  `-g *.md` keeps only Markdown; `-l` with no path searches the program's folder;
  a pattern with no match exits with status 1.
* fd, in its own folder /data/fd, given the tree with `@/data/rg/t` (a folder the
  person grants the run): a name pattern, `-e md`, `-t d` (folders, the empty one
  too), and fd with no arguments in its own empty folder.

Output must reach the serial record as `[std.out]` lines; each run must exit with
the right status and close."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
import os
os.environ.setdefault('SEXOS_GATE_BOUND','1800')
from quil_autosave_gate import boot
from lua_gate import qcodes,SURFACE
ROOT=Path(__file__).resolve().parents[1]

# fs.rs request kinds, as `[spindle.fs.result]` names them.
MKDIR,WRITE=3,8
TREE=[('mkdir','/data/rg/t'),('mkdir','/data/rg/t/src'),('mkdir','/data/rg/t/docs'),('mkdir','/data/rg/t/empty'),
      ('write','/data/rg/t/README.md Hello world'),('append','/data/rg/t/README.md second line says hello'),
      ('write','/data/rg/t/src/main.rs fn main() { greet(); }'),('write','/data/rg/t/src/lib.rs pub fn greet() { hello(); }'),
      ('write','/data/rg/t/docs/notes.md notes about HELLO'),('write','/data/rg/t/docs/todo.txt todo: port fd')]

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 programs=[]
 for name in ('rg','fd'):
  elf=(ROOT/'iso_root/programs'/name).read_bytes();assert elf[:4]==b'\x7fELF';programs.append((name,elf))
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,programs)
 g=boot(a.iso,disk,a.out,'rg-fd',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  assert len(line)<256,f'line too long for Spindle: {len(line)}'
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
  base=None
  for n,ch in enumerate(line):
   s=len(t())
   q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40})
   m=g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n',s,30)
   if base is None:base=int(m[1])-n
   assert int(m[1])==base+n,f'Spindle dropped a key before {line[:n+1]!r}'
  s=len(t());q.key('ret')
  g['wait'](rf'\[spindle\.input\.recv\] key=enter len={base+len(line)-1}\n',s,30)
 def shell(cmd,rest):
  s=len(t());type_line(f'{cmd} {rest}')
  kind=MKDIR if cmd=='mkdir' else WRITE
  g['wait'](rf'\[spindle\.fs\.result\] kind={kind} status=0 ',s,60)
 def run(prog,args,status=0):
  s=len(t());type_line(f'run {prog} {args}'.rstrip())
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=(\d+)\n',s,600)
  target,task=int(e[1]),int(e[2])
  # The serial record holds whole lines: everything before the exit is in.
  done=t()[s:s+e.start()]
  out=re.findall(r'\[std\.out\] ([^\n]*)\n',done)
  err=re.findall(r'\[std\.err\] ([^\n]*)\n',done)
  # A run that printed nothing opened no window and ends by itself; otherwise its
  # window stays until the person closes it.
  if out or err:
   s2=len(t());g['focus'](SURFACE[target]);time.sleep(.5);q.key('f11')
  else:s2=s
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s2,180)
  print(f'$ run {prog} {args}  (status {e[3]})',flush=True)
  for line in out:print('  '+line,flush=True)
  for line in err:print('  stderr: '+line,flush=True)
  assert int(e[3])==status,f'run {prog} {args}: status {e[3]}, want {status}'
  return out,err
 def same(got,want,what):
  assert got==want,f'{what}: got {got!r}, want {want!r}'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  out,_=run('rg','--version');assert out and out[0].startswith('ripgrep 15.2.0'),out
  for cmd,rest in TREE:shell(cmd,rest)
  out,err=run('rg','--sort path -n --no-heading hello t')
  same(out,['t/README.md:2:second line says hello','t/src/lib.rs:1:pub fn greet() { hello(); }'],'rg -n');same(err,[],'rg -n stderr')
  out,err=run('rg','--sort path -n --no-heading -i hello t')
  same(out,['t/README.md:1:Hello world','t/README.md:2:second line says hello','t/docs/notes.md:1:notes about HELLO','t/src/lib.rs:1:pub fn greet() { hello(); }'],'rg -i');same(err,[],'rg -i stderr')
  out,err=run('rg','--sort path -c -i hello t')
  same(out,['t/README.md:2','t/docs/notes.md:1','t/src/lib.rs:1'],'rg -c');same(err,[],'rg -c stderr')
  out,err=run('rg','--sort path -n --no-heading -i -g *.md hello t')
  same(out,['t/README.md:1:Hello world','t/README.md:2:second line says hello','t/docs/notes.md:1:notes about HELLO'],'rg -g');same(err,[],'rg -g stderr')
  out,err=run('rg','-l todo')
  same(out,['t/docs/todo.txt'],'rg -l in its folder');same(err,[],'rg -l stderr')
  out,err=run('rg','zebra t',status=1);same(out,[],'rg no match');same(err,[],'rg no match stderr')
  # fd: the walk runs on several threads, so compare what it found, in any order.
  out,err=run('fd','main @/data/rg/t')
  same(sorted(out),['@/data/rg/t/src/main.rs'],'fd PATTERN');same(err,[],'fd stderr')
  out,err=run('fd','-e md . @/data/rg/t')
  same(sorted(out),['@/data/rg/t/README.md','@/data/rg/t/docs/notes.md'],'fd -e');same(err,[],'fd -e stderr')
  out,err=run('fd','-t d . @/data/rg/t')
  same(sorted(o.rstrip('/') for o in out),['@/data/rg/t/docs','@/data/rg/t/empty','@/data/rg/t/src'],'fd -t d');same(err,[],'fd -t d stderr')
  out,err=run('fd','')
  same(out,[],'fd in its empty folder');same(err,[],'fd empty stderr')
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':11})
 finally:g['stop']()
if __name__=='__main__':main()
