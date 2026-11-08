#!/usr/bin/env python3
"""A program gets a heap from its own row's arena: Vec, Box, BTreeMap and String work,
it grows past the old 8 MiB window, every row can hold 12 MiB at once, pressing A
fills the machine's memory and is refused (not faulted) at the end, every run starts
with no memory and only zeroed pages, and Close gives every page back."""
import argparse,os,re,shutil,time
# Filling a 512 MiB machine's memory under emulation takes longer than most gates.
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
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
 def free_now():
  m=re.findall(r'free_pages=(\d+)',t());return int(m[-1])if m else None
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started():
  s=len(t());run('run heaptest')
  m=g['wait'](r'\[heaptest\.(ok|fail)\] target=(\d+) task=(\d+)([^\n]*)\n',s,180)
  assert m[1]=='ok',f'heap test failed: {m[0]}'
  target,task=m[2],int(m[3]);fields=dict(kv.split('=')for kv in m[4].split())
  g['wait'](rf'\[heaptest.ready\] target={target} task={task}',s,60);g['wait'](rf'\[display.text.committed\] target={target} ',s,60)
  grants=re.findall(rf'\[app.program.memory\] target={target} task={task} granted=(\d+) held=(\d+) base=(0x[0-9a-f]+) window_pages=(\d+)',t()[s:])
  assert grants,'no grant evidence'
  assert re.search(rf'\[app\.program\.loaded\] target={target} task={task} [^\n]* stack_pages=1024\n',t()[s:]),'the 4 MiB stack heaptest asks for was not given'
  assert int(grants[0][1])==16 and len({b for *_,b,_ in grants})==1,'grants are not one span from the window start'
  peak=int(fields['peak']);assert 12*1024*1024<peak<=WINDOW,f'peak {peak}'
  assert int(fields['full_mib'])>=12,f"filled only {fields['full_mib']} MiB"
  return dict(target=int(target),task=task,peak=peak,full=int(fields['full_mib']),held=int(grants[-1][1]),free=int(re.findall(r'free_pages=(\d+)',t()[s:])[-1]))
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
  # One program takes all the memory programs may have, is refused at the end, and
  # gives every page back at Close: the next run finds as much free as before.
  big=started();s=len(t());g['focus'](SURFACE[big['target']]);time.sleep(.5);q.key('a')
  m=g['wait'](rf'\[heaptest\.(all|fail)\] target={big["target"]} task={big["task"]} ([^\n]*)\n',s,900)
  assert m[1]=='all',f'fill failed: {m[0]}'
  filled=int(dict(kv.split('=')for kv in m[2].split())['full_mib']);assert filled>=64,f'filled only {filled} MiB'
  big.update(filled=filled,free=int(re.findall(r'free_pages=(\d+)',t()[s:])[-1]),held=int(re.findall(rf'target={big["target"]} task={big["task"]} granted=\d+ held=(\d+)',t())[-1]),window=int(re.findall(rf'target={big["target"]} task={big["task"]} granted=\d+ held=\d+ base=0x[0-9a-f]+ window_pages=(\d+)',t())[-1]))
  # Filled means the machine ran out, or the program reached its arena (all but its
  # image, stack and guard page).
  assert big['free']<1024 or big['held']>=big['window']-2048,f"memory left over after filling: {big}"
  close(big)
  after=started();close(after)
  assert after['free']+after['held']>=big['free']+big['held']-64,f"pages leaked: {big} then {after}"
  # A deep call fits the 4 MiB stack; running past it hits the guard page and ends
  # only that run, never the machine.
  deep=started();s=len(t());g['focus'](SURFACE[deep['target']]);time.sleep(.5);q.key('s')
  g['wait'](rf'\[heaptest\.deep\] target={deep["target"]} task={deep["task"]} frames=20 ',s,120)
  s=len(t());q.key('o')
  g['wait'](rf'\[heaptest\.overflow\] target={deep["target"]} task={deep["task"]}\n',s,60)
  g['wait'](rf'fault\.kill \[paging\.fault\.terminal\] pd={deep["target"]} ',s,120)
  text=t();assert 'KERNEL PANIC'not in text and '[heaptest.fatal]'not in text and '[heaptest.fail]'not in text
  print({'ok':True,'runs':runs,'side_by_side':live,'all':big,'after':after})
 finally:g['stop']()
if __name__=='__main__':main()
