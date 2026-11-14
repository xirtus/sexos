#!/usr/bin/env python3
"""Spreadeagle, the spreadsheet (GridCraft's engine, built for x86_64-sex-std.json):

* `run spreadeagle` writes welcome.xlsx (hand-written SpreadsheetML, apps/spreadeagle/make_welcome.py)
  into its folder and opens it: two sheets, the Budget column A as written.
* Right shows column B with the cached formula results recalculated by the engine
  (B5 =SUM(B2:B4) = 1650, B6 =B5*12 = 19800).
* Typing 40 into B2 and Enter recalculates: B5 490, B6 5880.
* Ctrl+S writes the xlsx; the window closes.
* A second `run spreadeagle` reads the saved file back: B2 is 40 and the formulas still give
  490 and 5880, so GridCraft's writer and reader round-trip on SexOS; Ctrl+Page Down shows
  the second sheet.

No kernel panic, page fault or std abort anywhere in the record."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
COL_A=r'Item\|Rent\|Food\|Fun\|Total\|Per year\|\|'

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/spreadeagle').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('spreadeagle',elf)])
 g=boot(a.iso,disk,a.out,'spreadeagle',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  for k in line:
   if k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key({' ':'spc','.':'dot'}.get(k,k))
   time.sleep(.05)
 def run(line):
  g['focus'](153);time.sleep(.5);type_line(line);q.key('ret')
 def column(col,text,start,timeout=300):
  return g['wait'](rf'\[spreadeagle\.column\] sheet=Budget col={col} text={text}\n',start,timeout)
 def opened():
  s=len(t());run('run spreadeagle')
  o=g['wait'](r'\[spreadeagle\.open\] target=(\d+) task=(\d+) name=welcome\.xlsx sheets=Budget,Notes\n',s,900)
  column('A',COL_A,s,600)
  g['focus'](SURFACE[int(o[1])]);time.sleep(.8)
  return o
 def close(o):
  s=len(t());q.key('f11');g['wait'](r'\[spreadeagle\.closed\]\n',s,120)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  o=opened()
  s=len(t());q.key('right');column('B',r'Cost\|1200\|300\|150\|1650\|19800\|\|',s)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-opened.ppm')})
  s=len(t());q.key('down');type_line('40');q.key('ret')
  g['wait'](r'\[spreadeagle\.set\] cell=B2 input=40\n',s,120)
  column('B',r'Cost\|40\|300\|150\|490\|5880\|\|',s)
  m=g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=(\d+)\n')
  assert int(m[1])>2000,f'saved file is too small: {m[0]}'
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-edited.ppm')})
  close(o)
  o=opened()
  s=len(t());q.key('right');column('B',r'Cost\|40\|300\|150\|490\|5880\|\|',s)
  s=len(t());g['chord']('ctrl','pgdn',r'\[spreadeagle\.column\] sheet=Notes col=A text=Welcome to Spreadeagle, the SexOS spreadsheet\.\|')
  close(o)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'saved_bytes':int(m[1])})
 finally:g['stop']()
if __name__=='__main__':main()
