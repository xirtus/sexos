#!/usr/bin/env python3
"""A program gets a heap from its own row's window: Vec, Box, BTreeMap and String work,
it can fill the window and is refused (not faulted) past it, every run starts with no
memory and only zeroed pages, rows run side by side, and Close takes the pages back."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
ROWS=(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text().count('app: App::Program, ordinal:')-1
WINDOW=eval(re.search(r'pub const WINDOW_BYTES: usize = ([\d* ]+);',(ROOT/'crates/sex-pdx/src/program_store.rs').read_text())[1])
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--runs',type=int,default=4);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/heaptest').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('heaptest',elf)])
 g=boot(a.iso,disk,a.out,'heap',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started():
  s=len(t());run('run heaptest')
  m=g['wait'](r'\[heaptest\.(ok|fail)\] target=(\d+) task=(\d+)([^\n]*)',s,180)
  assert m[1]=='ok',f'heap test failed: {m[0]}'
  target,task=m[2],int(m[3]);fields=dict(kv.split('=')for kv in m[4].split())
  g['wait'](rf'\[heaptest.ready\] target={target} task={task}',s,60);g['wait'](rf'\[display.text.committed\] target={target} ',s,60)
  grants=re.findall(rf'\[app.program.memory\] target={target} task={task} granted=(\d+) held=(\d+) base=(0x[0-9a-f]+) window_pages=(\d+)',t()[s:])
  assert grants,'no grant evidence'
  assert int(grants[0][1])==16 and len({b for *_,b,_ in grants})==1,'grants are not one span from the window start'
  peak=int(fields['peak']);assert 4*1024*1024<peak<=WINDOW,f'peak {peak}'
  assert int(fields['full_mib'])>=1,'the heap never reached the window'
  return dict(target=int(target),task=task,peak=peak,full=int(fields['full_mib']),held=int(grants[-1][1]))
 def close(run):
  s=len(t());q.key('f11');g['wait'](rf'\[heaptest.closed\] target={run["target"]} task={run["task"]}',s,60)
  m=g['wait'](rf'\[app.private.retired\] target={run["target"]} task={run["task"]} pages=(\d+)',s,60)
  assert int(m[1])>=run['held'],'retire did not take the grown pages';time.sleep(1)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  runs=[]
  for _ in range(a.runs):
   r=started();runs.append(r);path=a.out/f'heaptest-{r["task"]}.ppm';q.cmd('screendump',{'filename':str(path)});close(r)
  assert len({r['task']for r in runs})==len(runs),'a run reused an old task identity'
  assert len({r['target']for r in runs})<len(runs),'no row was ever reused'
  # Every row at once: each has its own window on its own key or turn.
  live=[started() for _ in range(ROWS)]
  assert len({r['target']for r in live})==ROWS,'programs were not on separate rows'
  q.cmd('screendump',{'filename':str(a.out/'side-by-side.ppm')});time.sleep(1)
  for r in reversed(live):
   g['focus'](SURFACE[r['target']]);close(r)
  # A row used before starts the next run empty and zeroed (checked by the program).
  close(started())
  text=t();assert 'KERNEL PANIC'not in text and '[heaptest.fatal]'not in text and '[heaptest.fail]'not in text
  print({'ok':True,'runs':runs,'side_by_side':live})
 finally:g['stop']()
if __name__=='__main__':main()
