#!/usr/bin/env python3
"""System monitor: `run monitor` lists the services and programs that use the CPU with
their share of one core, from the kernel's read-only statistics (syscall 87). While
`run bench spin` computes without a system call, Monitor must name Bench at the top of
its list at close to a whole core, and the window must show it."""
import argparse,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SAMPLE=r'\[monitor\.sample\] rows=(\d+) busy_tenths=(\d+) top_pd=(\d+) top_tenths=(\d+) top_task=(\d+)\n'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--min-tenths',type=int,default=700);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('monitor','bench')])
 g=boot(a.iso,disk,a.out,'monitor',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());run('run monitor')
  g['wait'](r'\[monitor\.ready\] target=(\d+) task=(\d+)',s,180)
  idle=g['wait'](SAMPLE,s,120)
  s=len(t());run('run bench spin')
  spin=g['wait'](r'\[bench\.spin\.start\] target=(\d+) task=(\d+)',s,180);bench=int(spin[1])
  seen=[];deadline=time.time()+240;end=None
  while time.time()<deadline:
   for m in re.finditer(SAMPLE,t()[s:]):
    if int(m[3])==bench:seen.append(int(m[4]))
   end=re.search(r'\[bench\.spin\.done\]',t()[s:])
   if (seen and max(seen)>=a.min_tenths) or end:break
   time.sleep(2)
  shot=a.out/'monitor.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  try:
   from PIL import Image;Image.open(shot).save(a.out/'monitor.png')
  except Exception:pass
  assert seen,f'Monitor never listed Bench (domain {bench}) at the top'
  assert max(seen)>=a.min_tenths,f'Bench peaked at {max(seen)/10:.1f}% of a core, want {a.min_tenths/10:.0f}%'
  text=t();assert 'KERNEL PANIC' not in text and '[monitor.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'bench_pd':bench,'bench_peak_percent':max(seen)/10,'samples_on_top':len(seen),'idle_busy_percent':int(idle[2])/10,'cores':os.environ.get('SEXOS_GATE_CORES','1')})
 finally:g['stop']()
if __name__=='__main__':main()
