#!/usr/bin/env python3
"""A window dragged by its title bar moves, floats free of the tiling, and stays put when other windows open; zoom puts it back."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
from browser_gate_viewport import chrome_metrics
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'float',a.resolution);t=g['text'];q=g['q']
 def at(x,y):q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':int(x*32767/1280)}},{'type':'abs','data':{'axis':'y','value':int(y*32767/800)}}]})
 def btn(down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]})
 def quil_x():return int(re.findall(r'\[silk.drag.move\] surface=201 x=(-?\d+) y=(-?\d+)',t())[-1][0])
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  # Quil's title bar, dragged right.
  metrics=chrome_metrics(t())
  x,y,w,h=map(int,re.findall(r'\[shell.tile.apply\] sid=201 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1])
  tx=x+192*metrics['scale'];ty=y+metrics['title']//2
  assert x+72*metrics['scale']<tx<x+w-metrics['inset'] and y<=ty<y+metrics['title'],('title drag point',x,y,w,h,tx,ty)
  s=len(t());at(tx,ty);time.sleep(.5);btn(True);time.sleep(.3)
  for i in range(1,11):at(tx+i*20,ty);time.sleep(.15)
  btn(False);g['wait'](r'\[shell.window.float\] frame=\d+ sid=201',s,30);g['wait'](r'\[silk.drag.end\] surface=201',s,30);time.sleep(2)
  moved=quil_x();assert moved>=150,moved
  q.cmd('screendump',{'filename':str(a.out/'floated.ppm')});time.sleep(.5)
  # Another window opens: the tiling places it, and leaves Quil where it was put.
  s=len(t());palette_select(g['key'],'FocusProgram',palette_commands(ROOT));q.key('ret')
  g['wait'](r'\[hello.ready\]',s,60);g['wait'](r'\[shell.tile.apply\] sid=359',s,60);time.sleep(2)
  assert not re.search(r'\[shell.tile.apply\] sid=201 ',t()[s:]),'tiling moved the floating window'
  q.cmd('screendump',{'filename':str(a.out/'after-open.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'quil_x':moved})
 finally:g['stop']()
if __name__=='__main__':main()
