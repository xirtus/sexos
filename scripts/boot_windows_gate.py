#!/usr/bin/env python3
"""A real machine's first desktop (x17, 2026-11-24): boots once with CPUID's hypervisor
bit off, which is how the Shell tells a real PC from a VM, and checks that

- the kernel's timer took its rate from a real clock (`[timer.source]` names one and
  counts more than zero per ms), not the fixed fallback that made the x17's 1 ms tick
  ~417 ms;
- the Shell put Linen, Quil and Kaleidoscope away (`[shell.boot.windows] tidy=1
  minimized=0b111`) and gave Spindle the focus;
- Spindle still takes typing afterwards.

In a VM (every other gate) the Shell keeps the reference layout: `tidy=0
reason=virtual_machine`; the gate checks that too, from a second, ordinary boot."""
import argparse,os,re,shutil,sys
from pathlib import Path
from quil_autosave_gate import boot

def run(iso,disk,out,tag,cpu):
 os.environ['SEXOS_GATE_CPU']=cpu
 g=boot(iso,disk,out,tag,'1280x800')
 try:
  timer=g['wait'](r'\[timer\.source\] source=(\w+) per_ms=(\d+)',0,60)
  windows=g['wait'](r'\[shell\.boot\.windows\] tidy=(\d)[^\n]*',0,120)
  return g,timer,windows
 except BaseException:
  g['stop']();raise

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path)
 p.add_argument('--out',required=True,type=Path);a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
 base=os.environ.get('SEXOS_GATE_CPU','max,+pku,+rdseed')
 disk=a.out/'metal.img';shutil.copyfile(a.source_disk,disk)
 g,timer,windows=run(a.iso,disk,a.out,'metal',base+',hypervisor=off')
 try:
  print(f'[boot.windows.gate] metal timer={timer[0]!r} windows={windows[0]!r}',flush=True)
  assert timer[1]!='none' and int(timer[2])>0,f'timer had no clock: {timer[0]}'
  assert windows[1]=='1',f'Shell kept the boot windows on a real machine: {windows[0]}'
  assert 'minimized=0b111' in windows[0],f'not every boot window was put away: {windows[0]}'
  assert 'spindle_focus=1' in windows[0],f'Spindle did not get the focus: {windows[0]}'
  # The person types into Spindle straight away.
  s=g['size']();g['q'].key_sequence(['h','w'])
  g['wait'](r'\[spindle\.(text|line\.edit)',s,60)
 finally:
  g['stop']()
 disk=a.out/'vm.img';shutil.copyfile(a.source_disk,disk)
 g,timer,windows=run(a.iso,disk,a.out,'vm',base)
 g['stop']()
 print(f'[boot.windows.gate] vm timer={timer[0]!r} windows={windows[0]!r}',flush=True)
 assert 'tidy=0 reason=virtual_machine' in windows[0],f'Shell changed the VM layout: {windows[0]}'
 print('[boot.windows.gate] ok=1',flush=True)

if __name__=='__main__':
 sys.exit(main())
