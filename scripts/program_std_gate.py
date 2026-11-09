#!/usr/bin/env python3
"""Ordinary programs run on SexOS, built for x86_64-sex-std.json:

* `run stdhello alpha beta`: Rust against the real standard library (crates/sex-std):
  println!, arguments, HashMap, std::fs (write, append, read, read_dir, metadata,
  remove, no way above its folder), threads with Arc/Mutex, mpsc, Instant, sleep.
* `run chello one two`: C `main(argc, argv)` on sex-libc's POSIX layer: stdio streams,
  fopen/fgets/fseek/ftell, open/lseek/read, stat, rename, opendir/readdir, remove,
  errno, qsort, malloc, atexit.
* `run pdf`: the PDF viewer (hayro) writes welcome.pdf into its folder, reads it back
  through std::fs, renders page 1 with ink on it, turns to page 2 on Right, and closes.

Every check must pass, output must reach the window and the serial record, the program
must wait for its window to close after main returns, and each must run twice."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
# Several programs, a PDF page rendered twice under emulation.
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
PROGRAMS=[
 ('stdhello','alpha beta',r'\[std\.out\] arguments: \["alpha", "beta"\]\n',r'\[stdhello\.(ok|fail)\] passed=(\d+)/(\d+)\n',14),
 ('chello','one two',r'\[std\.out\] arguments: \[one\] \[two\]\n',r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',17),
]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elfs=[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in [p[0] for p in PROGRAMS]+['pdf']]
 assert all(e[:4]==b'\x7fELF' for _,e in elfs)
 install(disk,elfs)
 g=boot(a.iso,disk,a.out,'std',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started(name,args,said,score,least):
  s=len(t());run(f'run {name} {args}')
  g['wait'](said,s,600)
  m=g['wait'](score,s,600)
  if m[1]!='ok':
   time.sleep(1);raise AssertionError(f'{name} checks failed: '+''.join(re.findall(r'\[std\.out\] FAIL[^\n]*\n',t()[s:])))
  assert int(m[3])>=least and m[2]==m[3],f'not every check ran: {m[0]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s,60)
  return dict(name=name,target=int(e[1]),task=int(e[2]),passed=int(m[2]))
 def close(r):
  s=len(t());g['focus'](SURFACE[r['target']]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={r["target"]} task={r["task"]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  runs=[]
  for prog in PROGRAMS:
   first=started(*prog);g['wait'](rf'\[display.text.committed\] target={first["target"]} ',0,60)
   time.sleep(1);q.cmd('screendump',{'filename':str(a.out/f'{prog[0]}.ppm')})
   close(first);second=started(*prog);close(second);runs+=[first,second]
  for n in range(2):
   s=len(t());run('run pdf')
   o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=welcome\.pdf pages=2\n',s,600)
   m=g['wait'](r'\[pdf\.page\] page=1 pages=2 width=(\d+) height=(\d+) ink=(\d+) ms=(\d+)\n',s,900)
   assert int(m[1])>=300 and int(m[3])>2000,f'page 1 looks empty: {m[0]}'
   time.sleep(1)
   if n==0:q.cmd('screendump',{'filename':str(a.out/'pdf-page1.ppm')})
   g['focus'](SURFACE[int(o[1])]);time.sleep(.5)
   s2=len(t());q.key('right')
   m=g['wait'](r'\[pdf\.page\] page=2 pages=2 width=\d+ height=\d+ ink=(\d+) ms=\d+\n',s2,900)
   assert int(m[1])>1000
   if n==0:time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-page2.ppm')})
   q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s2,60)
   g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s2,180)
   runs.append({'name':'pdf','ink':int(m[1])})
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':runs})
 finally:g['stop']()
if __name__=='__main__':main()
