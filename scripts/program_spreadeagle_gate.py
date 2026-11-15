#!/usr/bin/env python3
"""Spreadeagle, the spreadsheet (GridCraft's engine, built for x86_64-sex-std.json):

* `run spreadeagle` writes welcome.xlsx (hand-written SpreadsheetML, apps/spreadeagle/make_welcome.py)
  into its folder and opens it: two sheets, the Budget column A as written.
* Right shows column B with the cached formula results recalculated by the engine
  (B5 =SUM(B2:B4) = 1650, B6 =B5*12 = 19800).
* Typing 40 into B2 and Enter recalculates: B5 490, B6 5880.
* Ctrl+S writes the xlsx; the window closes.
* A second `run spreadeagle` reads the saved file back: B2 is 40 and the formulas still give
  490 and 5880, so GridCraft's writer and reader round-trip on SexOS.
* Shift+arrows select A1:B4; Ctrl+Shift+D sorts it by column B, Z to A, under its header
  (300, 150, 40) and the totals still read 490 and 5880.
* Ctrl+Shift+C adds a column chart of the selection at D1, drawn over the grid.
* Ctrl+Shift+E on A3 (Fun) filters the region to that row (the others read `~`);
  Ctrl+Shift+L turns the filter off and every row shows again.
* Ctrl+Shift+F on B2 freezes the header row and the label column; Ctrl+S saves.
* A third `run spreadeagle` finds the chart and the frozen panes kept; Ctrl+Page Down
  shows the second sheet.

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
 def opened(col_a=COL_A):
  s=len(t());run('run spreadeagle')
  o=g['wait'](r'\[spreadeagle\.open\] target=(\d+) task=(\d+) name=welcome\.xlsx sheets=Budget,Notes\n',s,900)
  column('A',col_a,s,600)
  g['focus'](SURFACE[int(o[1])]);time.sleep(.8)
  return o
 def ctrl_shift(k,pattern):
  s=len(t());keys=('ctrl','shift',k)
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]})
  return g['wait'](pattern,s,120)
 def shift(k):
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':k}],'hold-time':80});time.sleep(.3)
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
  # Sort a selection, chart it, filter, freeze.
  s=len(t());g['chord']('ctrl','home',rf'\[spreadeagle\.column\] sheet=Budget col=A text={COL_A}\n')
  for k in('down','down','down','right'):shift(k)
  ctrl_shift('d',r'\[spreadeagle\.column\] sheet=Budget col=B text=Cost\|300\|150\|40\|490\|5880\|\|\n')
  m2=ctrl_shift('c',r'\[spreadeagle\.chart\] at=D1 width=(\d+) height=(\d+)\n')
  assert int(m2[1])>200 and int(m2[2])>150,m2[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart.ppm')})
  q.key('esc');time.sleep(.3)
  s=len(t());q.key('left');column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|',s)
  q.key('up');time.sleep(.3)
  ctrl_shift('e',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|~\|Fun\|~\|~\|~\|\|\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter.ppm')})
  ctrl_shift('l',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|Food\|Fun\|Rent\|Total\|Per year\|\|\n')
  q.key('up');time.sleep(.3);q.key('right');time.sleep(.5)
  ctrl_shift('f',r'\[spreadeagle\.command\] view\.freezePanes\n')
  g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=\d+\n')
  close(o)
  s=len(t());o=opened(r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|')
  g['wait'](r'\[spreadeagle\.view\] charts=1 frozen=1,1\n',s,60)
  s=len(t());g['chord']('ctrl','pgdn',r'\[spreadeagle\.column\] sheet=Notes col=A text=Welcome to Spreadeagle, the SexOS spreadsheet\.\|')
  close(o)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'saved_bytes':int(m[1])})
 finally:g['stop']()
if __name__=='__main__':main()
