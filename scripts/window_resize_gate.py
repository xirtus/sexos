#!/usr/bin/env python3
"""A window dragged by its corner changes size on screen and floats: the other windows tile the whole area again, and the tiling leaves it be when more windows open."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'resize',a.resolution);t=g['text'];q=g['q']
 def at(x,y):q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':int(x*32767/1280)}},{'type':'abs','data':{'axis':'y','value':int(y*32767/800)}}]})
 def btn(down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]})
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  x,y,w,h=map(int,re.findall(r'\[shell.tile.apply\] sid=201 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1])
  # Quil's bottom-right corner, dragged up and left.
  cx,cy=x+w-2,y+h-2
  s=len(t());at(cx,cy);time.sleep(.5);btn(True);time.sleep(.3)
  for i in range(1,11):at(cx-i*12,cy-i*8);time.sleep(.15)
  btn(False);g['wait'](r'\[silk.resize.end\] sid=201',s,30);time.sleep(2)
  got=re.findall(r'\[silk.resize.apply\] sid=201 edge=BottomRight x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t()[s:])
  assert got,'no resize applied'
  nx,ny,nw,nh=map(int,got[-1])
  assert (nx,ny)==(x,y) and w-130<=nw<=w-110 and h-90<=nh<=h-70,(x,y,w,h,got[-1])
  assert re.search(r'\[shell.window.float\]|\[silk.resize.hit\] frame=\d+ surface=201',t()[s:])
  # The tiled windows take the whole area again, the resized one stays put above them.
  assert re.search(r'\[shell.window.retile\] frame=\d+ sid=201',t()[s:]),'no retile after resize'
  r=t()[s:];assert not re.search(r'\[shell.tile.apply\] sid=201 ',r),'tiling moved the resized window'
  others=[(int(m[0]),int(m[1])) for m in re.findall(r'\[shell.tile.apply\] sid=\d+ x=(-?\d+) y=-?\d+ w=(\d+)',r)]
  # E1's independent layout goldens: 8px outside, 6px between tiles.
  # Resizing must refill the work area while preserving those existing gaps.
  assert others and min(ox for ox,_ in others)==8,others
  assert max(ox+ow for ox,ow in others)==1280-8,others
  ordered=sorted(set(others))
  assert all(right[0]-(left[0]+left[1])>=6 for left,right in zip(ordered,ordered[1:])),others
  q.cmd('screendump',{'filename':str(a.out/'resized.ppm')});time.sleep(.5)
  # Another window opens: the tiling leaves the resized window as it is.
  s=len(t());palette_select(g['key'],'FocusProgram',palette_commands(ROOT));q.key('ret')
  g['wait'](r'\[hello.ready\]',s,60);g['wait'](r'\[shell.tile.apply\] sid=359',s,60);time.sleep(2)
  assert not re.search(r'\[shell.tile.apply\] sid=201 ',t()[s:]),'tiling resized the window back'
  q.cmd('screendump',{'filename':str(a.out/'after-open.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'before':(w,h),'after':(nw,nh)})
 finally:g['stop']()
if __name__=='__main__':main()
