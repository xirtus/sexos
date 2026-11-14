#!/usr/bin/env python3
"""Random syscalls and pdx calls from a program stop nothing: `run fuzz CALLS SEED`
makes CALLS seeded calls (every syscall number but those that end or reshape the caller,
and pdx calls to any slot with any operation and arguments). The kernel must not stop,
no service may fault, the fuzzer itself must finish and report, and the desktop must
still answer afterwards (Spindle runs a command, a program starts and closes).
Protocol seeds (`run fuzz CALLS SEED proto`) send every operation number sex-pdx names
to the services this program holds, with mutated arguments. A long run for the nightly
soak: --calls 1000000 --seeds 1 --proto-seeds 2."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--calls',type=int,default=20000);p.add_argument('--seeds',default='1,2,3');p.add_argument('--proto-seeds',default='4');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=ROOT/'iso_root/programs/fuzz';assert elf.read_bytes()[:4]==b'\x7fELF'
 install(disk,[('fuzz',elf.read_bytes())])
 g=boot(a.iso,disk,a.out,'fuzz',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 runs=[]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  plan=[(x,False)for x in a.seeds.split(',')if x]+[(x,True)for x in a.proto_seeds.split(',')if x]
  for seed,proto in plan:
   s=len(t());run(f'run fuzz {a.calls} {seed}'+(' proto'if proto else''))
   m=g['wait'](rf'\[fuzz.start\] target=(\d+) task=(\d+) calls={a.calls} seed={seed}',s,120)
   d=g['wait'](rf'\[fuzz.done\] target={m[1]} task={m[2]} calls=(\d+) ipc=(\d+) ok=(\d+) seed={seed}',s,max(900,a.calls//20000*120))
   run_text=t()[s:];answers=len(re.findall(r'\[fuzz\.answer\] value=',run_text))
   ops=re.search(r'\[fuzz\.ops\] covered=(\d+) of=(\d+) other_answers=(\d+)',run_text)
   runs.append(dict(seed=int(seed),proto=proto,calls=int(d[1]),ipc=int(d[2]),ok=int(d[3]),answer_kinds=answers,ops_covered=f'{ops[1]}/{ops[2]}'if ops else None))
   if proto:assert ops and int(ops[1])*10>=int(ops[2])*9,f'protocol run covered too few operations: {ops and ops.group(0)}'
   # Close it the normal way: Shell and the services still answer.
   time.sleep(1);surface=int(re.findall(rf'\[shell.program.launch\] target={m[1]} task={m[2]} surface=(\d+)',t())[-1])
   s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
  text=t()
  services=[l for l in re.findall(r'fault.kill \[[^\]]+\] pd=(\d+)',text) if int(l)<=15]
  assert not services,f'services faulted: {services}'
  assert '[fuzz.fatal]' not in text and 'KERNEL PANIC' not in text,'the fuzzer or the kernel stopped'
  # The desktop still works: Spindle answers a command.
  s=len(t());run('crashes');g['wait'](r'\[spindle.crashes\] count=\d+',s,60)
  print({'ok':True,'runs':runs})
 finally:g['stop']()
if __name__=='__main__':main()
