#!/usr/bin/env python3
"""A program outside the built-in apps launches, closes and runs again, reusing rows under fresh tasks."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
from check_desktop_pixels import font_from_source,glyph
from check_disk_persistence import ppm_rgb
ROOT=Path(__file__).resolve().parents[1]
SOURCE=(ROOT/'apps/hello/src/main.rs').read_text()
TEXT=re.search(r'const TEXT: &\[u8\] = b"(.*?)";',SOURCE)[1].encode().decode('unicode_escape')
def text_ink(path,frame,font,advance=12,line=18):
 """Fully covered Geist Mono pixels of TEXT in `frame`, and how many aren't the text colour."""
 import geist_cells
 width,_,pixels=ppm_rgb(path.read_bytes(),min_width=1,min_height=1)
 x0,y0=frame[0]+8,frame[1]+36
 bottom=frame[1]+frame[3]-8
 missing=0;ink=0
 # Display wraps at the window's width, a character at a time.
 cols=max(1,(frame[2]-16)//advance);lines=[]
 for content in TEXT.split('\n'):lines+=[content[i:i+cols] for i in range(0,len(content),cols)] or ['']
 for row,content in enumerate(lines):
  for col,char in enumerate(content):
   for gx,gy in geist_cells.solid(char):
    x=x0+col*advance+gx;y=y0+row*line+gy
    if y>=bottom:continue
    # The pointer rests at the screen's centre and may cover a glyph.
    if 636<=x<660 and 398<=y<432:continue
    ink+=1
    missing+=pixels[(y*width+x)*3:(y*width+x)*3+3]!=b'\xe0\xf0\xff'
 return ink,missing
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--runs',type=int,default=6);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 font=font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())
 g=boot(a.iso,disk,a.out,'program',a.resolution);t=g['text'];q=g['q'];commands=palette_commands(ROOT)
 def launch():
  palette_select(g['key'],'FocusProgram',commands)
  s=len(t());q.key('ret');m=g['wait'](rf'\[hello.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes={len(TEXT)}',s,60)
  target,task,sid=m[1],int(m[2]),m[3]
  g['wait'](rf'\[display.text.committed\] target={target} ',s,60);time.sleep(2)
  tile=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())
  assert tile,'no tile for program';x,y,w,h=map(int,tile[-1])
  path=a.out/f'program-{task}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  ink,missing=text_ink(path,(x,y,w,h),font);assert ink>500 and missing==0,f'text ink {ink} missing {missing}'
  return dict(target=int(target),task=task,ink=ink)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  runs=[]
  for _ in range(a.runs):
   run=launch();runs.append(run)
   s=len(t());q.key('f11');g['wait'](rf'\[hello.closed\] target={run["target"]} task={run["task"]}',s,60)
   g['wait'](rf'\[app.private.retired\] target={run["target"]} task={run["task"]} ',s,60);time.sleep(1)
  # Side by side: three live at once, each still drawn, closed in any order.
  live=[]
  for _ in range(3):
   palette_select(g['key'],'FocusProgram',commands)
   s=len(t());q.key('ret');m=g['wait'](rf'\[hello.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes={len(TEXT)}',s,60)
   live.append((m[1],int(m[2]),m[3]));g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
  assert len({row[0]for row in live})==3,'programs were not on separate rows'
  path=a.out/'side-by-side.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(1)
  side=[]
  for target,task,sid in live:
   tile=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,tile[-1])
   ink,missing=text_ink(path,(x,y,w,h),font);assert ink>100 and missing==0,f'row {target}: text ink {ink} missing {missing}'
   side.append(dict(target=int(target),task=task,ink=ink))
  for target,task,sid in reversed(live):
   g['focus'](int(sid));s=len(t());q.key('f11');g['wait'](rf'\[hello.closed\] target={target} task={task}',s,60)
   g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,60);time.sleep(1)
  tasks=[run['task']for run in runs]
  assert len(set(tasks))==len(tasks),'a run reused an old task identity'
  assert len(set(run['target']for run in runs))<len(runs),'no row was ever reused'
  assert 'KERNEL PANIC'not in t()
  print({'ok':True,'runs':runs,'side_by_side':side})
 finally:g['stop']()
if __name__=='__main__':main()
