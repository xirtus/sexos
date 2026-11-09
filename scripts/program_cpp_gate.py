#!/usr/bin/env python3
"""C++ runs in a SexOS program: `run cpp` links libc++ (scripts/build_libcxx.sh) and
checks static constructors, containers, strings, streams, virtual calls,
std::function, std::thread with std::mutex, and thread_local. Every check must pass,
the window must show the report, and the program must close and run again."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/cpp').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('cpp',elf)])
 g=boot(a.iso,disk,a.out,'cpp',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started():
  s=len(t());run('run cpp')
  m=g['wait'](r'\[cpp\.(ok|fail)\] target=(\d+) task=(\d+) passed=(\d+)/(\d+)\n',s,600)
  if m[1]!='ok':
   time.sleep(1);raise AssertionError('C++ checks failed: '+''.join(re.findall(r'\[cpp\.log\] [^\n]*\n',t()[s:])))
  assert m[4]==m[5] and int(m[5])>=10,f'not every check ran: {m[0]}'
  return dict(target=int(m[2]),task=int(m[3]),passed=int(m[4]))
 def close(r):
  s=len(t());g['focus'](SURFACE[r['target']]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[cpp\.closed\] target={r["target"]} task={r["task"]}\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={r["target"]} task={r["task"]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  first=started();g['wait'](rf'\[display.text.committed\] target={first["target"]} ',0,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'cpp.ppm')})
  close(first)
  second=started();close(second)
  text=t();assert 'KERNEL PANIC' not in text and '[cpp.fatal]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':[first,second]})
 finally:g['stop']()
if __name__=='__main__':main()
