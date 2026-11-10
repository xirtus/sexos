#!/usr/bin/env python3
"""Wallpaper and night light on a booted desktop: Super+B steps the wallpaper,
Super+Shift+N steps night light, and Display repaints with each. Screens are
saved for review, and the gate checks that the frame actually changed colour."""
import argparse,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
def ppm(path):
 data=path.read_bytes();parts=data.split(b'\n',3);w,h=map(int,parts[1].split());return w,h,parts[3]
def pixel(img,x,y):
 w,h,px=img;i=(y*w+x)*3;return tuple(px[i:i+3])
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'look',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.06)
 def press(*keys):
  for k in keys:send(True,[k])
  for k in reversed(keys):send(False,[k])
 def shot(name):time.sleep(1.5);f=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(f)});time.sleep(.4);return ppm(f)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  before=shot('00-theme')
  # Bottom-left corner: desktop background below any window.
  w,h,_=before;at=(4,h-4)
  s=len(t());press('meta_l','b');g['wait'](r'\[shell.look\] wallpaper=1 name=Dusk night=0 ok=1',s,30);g['wait'](r'\[display.look\] wallpaper=1 night=0',s,30)
  dusk=shot('01-wallpaper-dusk');assert pixel(dusk,*at)!=pixel(before,*at),(pixel(before,*at),pixel(dusk,*at))
  for level in (1,2,3):
   s=len(t());press('meta_l','shift','n');g['wait'](rf'\[display.look\] wallpaper=1 night={level}',s,30)
  warm=shot('02-night-light-high');r0,g0,b0=pixel(dusk,*at);r1,g1,b1=pixel(warm,*at)
  assert abs(r1-r0)<=2 and b1<b0,((r0,g0,b0),(r1,g1,b1))
  s=len(t());press('meta_l','shift','n');g['wait'](r'\[display.look\] wallpaper=1 night=0',s,30)
  back=shot('03-night-light-off');assert pixel(back,*at)==pixel(dusk,*at)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'theme':pixel(before,*at),'dusk':pixel(dusk,*at),'warm':pixel(warm,*at)})
 finally:g['stop']()
if __name__=='__main__':main()
