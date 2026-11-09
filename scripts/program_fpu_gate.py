#!/usr/bin/env python3
"""Programs use SSE and keep their own registers: several FPU test runs at once each
hold distinct XMM0-15 and MXCSR values across yields and preemption, get exact float
results, start with the reset MXCSR on a reused row, and the kernel stays soft-float."""
import argparse,re,shutil,subprocess,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def has_sse(path):
 text=subprocess.run(['objdump','-d','--no-show-raw-insn',str(path)],capture_output=True,text=True,check=True).stdout
 return bool(re.search(r'\s(movaps|movups|movdqu|movsd|addsd|mulsd|divsd|cvtsi2sd|xorps|pxor)\s',text))
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--copies',type=int,default=3);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=ROOT/'iso_root/programs/fputest';assert elf.read_bytes()[:4]==b'\x7fELF'
 assert has_sse(elf),'fputest was not built with SSE'
 assert not has_sse(ROOT/'iso_root/sexos-kernel'),'the kernel uses SSE'
 install(disk,[('fputest',elf.read_bytes())])
 g=boot(a.iso,disk,a.out,'fpu',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  assert '[fpu.ready] sse=1' in t(),'kernel did not enable SSE'
  s=len(t())
  for i in range(a.copies):
   s1=len(t());run('run fputest');g['wait'](r'\[shell.program.launch\] target=\d+ task=\d+ surface=\d+',s1,120);time.sleep(2)
  done=[]
  while len(done)<a.copies:
   m=g['wait'](r'\[fputest\.(ok|fail)\] target=(\d+) task=(\d+) ([^\n]*)',s,300);s2=t().index(m[0],s)+len(m[0])
   assert m[1]=='ok',f'FPU test failed: {m[0]}';done.append((int(m[2]),int(m[3]),m[4]));s=s2
  assert len({d[0] for d in done})==a.copies,'copies did not run on separate rows'
  launches=[(int(x),int(y)) for x,y in re.findall(r'\[shell.program.launch\] target=(\d+) task=(\d+) ',t())]
  # Close one and run again on the reused row: it must start with the reset MXCSR.
  for target,task,_ in done:
   surface=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
   s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
  s=len(t());run('run fputest');m=g['wait'](r'\[fputest\.(ok|fail)\] target=(\d+) task=(\d+) ([^\n]*)',s,300)
  assert m[1]=='ok',f'reused row: {m[0]}';assert int(m[2]) in {d[0] for d in done},'no row was reused'
  text=t();assert 'KERNEL PANIC' not in text and '[fputest.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'runs':done+[(int(m[2]),int(m[3]),m[4])]})
 finally:g['stop']()
if __name__=='__main__':main()
