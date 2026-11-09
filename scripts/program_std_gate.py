#!/usr/bin/env python3
"""An ordinary Rust program runs on SexOS: `run stdhello alpha beta` is built against
the real Rust standard library (x86_64-sex-std.json over crates/sex-std) and checks
println!, arguments, HashMap, std::fs (write, append, read, read_dir, metadata, remove,
no way above its folder), threads with Arc/Mutex, mpsc, Instant and sleep, SystemTime.
Every check must pass, its output must reach the window and the serial record, it must
wait for the window to close after main returns, and run again."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/stdhello').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('stdhello',elf)])
 g=boot(a.iso,disk,a.out,'stdhello',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started():
  s=len(t());run('run stdhello alpha beta')
  g['wait'](r'\[std\.out\] arguments: \["alpha", "beta"\]\n',s,600)
  m=g['wait'](r'\[stdhello\.(ok|fail)\] passed=(\d+)/(\d+)\n',s,600)
  if m[1]!='ok':
   time.sleep(1);raise AssertionError('std checks failed: '+''.join(re.findall(r'\[std\.out\] FAIL[^\n]*\n',t()[s:])))
  assert int(m[3])>=14,f'not every check ran: {m[0]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s,60)
  return dict(target=int(e[1]),task=int(e[2]),passed=int(m[2]))
 def close(r):
  s=len(t());g['focus'](SURFACE[r['target']]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={r["target"]} task={r["task"]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  first=started();g['wait'](rf'\[display.text.committed\] target={first["target"]} ',0,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'stdhello.ppm')})
  close(first)
  second=started();close(second)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':[first,second]})
 finally:g['stop']()
if __name__=='__main__':main()
