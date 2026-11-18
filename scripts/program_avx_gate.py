#!/usr/bin/env python3
"""Programs use AVX2 and keep their own YMM registers: two AVX test runs on one core
each hold distinct 256-bit YMM0-14 values and an AVX2 running sum in YMM15 across at
least 1,000 yields each, the kernel enables x87+SSE+AVX through XSAVE (never PKRU), and
the kernel itself stays free of vector code."""
import argparse,os,re,shutil,subprocess,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def disasm(path):return subprocess.run(['objdump','-d','--no-show-raw-insn',str(path)],capture_output=True,text=True,check=True).stdout
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--copies',type=int,default=2);p.add_argument('--min-yields',type=int,default=1000);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=ROOT/'iso_root/programs/avxtest';assert elf.read_bytes()[:4]==b'\x7fELF'
 assert re.search(r'\svpaddq\s+%ymm',disasm(elf)),'avxtest holds no AVX2 code'
 assert not re.search(r'%[xyz]mm\d',disasm(ROOT/'iso_root/sexos-kernel')),'the kernel uses vector registers'
 install(disk,[('avxtest',elf.read_bytes())])
 # TCG's "max" CPU has AVX2 and XSAVE; one core, so every copy shares it.
 os.environ.setdefault('SEXOS_GATE_CORES','1')
 g=boot(a.iso,disk,a.out,'avx',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  m=re.search(r'\[fpu\.ready\] sse=1 xsave_state=(\S+) save=(\S+) bytes=(\d+)',t());assert m,'kernel did not report its FPU state'
  assert m[1].startswith('x87+sse+avx'),f'kernel did not enable AVX: {m[0]}'
  assert m[2]!='fxsave' and int(m[3])>=832,f'kernel does not save YMM state: {m[0]}'
  s=len(t())
  for i in range(a.copies):
   s1=len(t());run('run avxtest');g['wait'](r'\[shell.program.launch\] target=\d+ task=\d+ surface=\d+',s1,120);time.sleep(2)
  done=[]
  while len(done)<a.copies:
   r=g['wait'](r'\[avxtest\.(ok|fail|skip)\] target=(\d+) task=(\d+) ([^\n]*)',s,600);s=t().index(r[0],s)+len(r[0])
   assert r[1]=='ok',f'AVX test did not pass: {r[0]}'
   y=int(re.search(r'yields=(\d+)',r[4])[1]);assert y>=a.min_yields,f'only {y} yields: {r[0]}'
   done.append((int(r[2]),int(r[3]),r[4]))
  assert len({d[0] for d in done})==a.copies,'copies did not run on separate rows'
  text=t();assert 'KERNEL PANIC' not in text and '[avxtest.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'fpu':m[0],'runs':done})
 finally:g['stop']()
if __name__=='__main__':main()
