#!/usr/bin/env python3
"""Anti-aliased drawing in a program: Shapes draws sex-canvas's demo scene (tiny-skia on
the program heap) into its pixel surface, and the screen matches the same scene rendered
on the host; an arrow key moves the front window and the redraw matches too."""
import argparse,re,shutil,subprocess,time
from pathlib import Path
from PIL import Image
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
W,H=400,320
def reference(out,dx,dy):
 path=out/f'reference-{dx}-{dy}.ppm'
 subprocess.run(['cargo','run','-q','-p','sex-canvas','--example','desktop','--',str(path),str(W),str(H),str(dx),str(dy)],cwd=ROOT,check=True)
 return Image.open(path).convert('RGB')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/shapes').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('shapes',elf)])
 refs={(0,0):reference(a.out,0,0),(12,0):reference(a.out,12,0)}
 g=boot(a.iso,disk,a.out,'shapes',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def compare(name,ref,tile):
  shot=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  x,y,w,h=tile;img=Image.open(shot).convert('RGB');bx,by=x+8,y+36
  checked=bad=worst=0
  for py in range(0,min(H,h-44),5):
   for px in range(0,min(W,w-16),7):
    if 636<=bx+px<660 and 398<=by+py<432:continue
    got,want=img.getpixel((bx+px,by+py)),ref.getpixel((px,py))
    d=max(abs(i-j)for i,j in zip(got,want));checked+=1;worst=max(worst,d);bad+=d>2
  img.crop((bx,by,bx+min(W,w-16),by+min(H,h-44))).save(a.out/f'{name}.png')
  assert checked>1000 and bad==0,f'{name}: {bad} of {checked} pixels differ (worst {worst})'
  return checked,worst
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());run('run shapes')
  m=g['wait'](r'\[shapes.ready\] target=(\d+) task=(\d+)',s,180);target,task=m[1],m[2]
  first=g['wait'](rf'\[shapes.shown\] w={W} h={H} dx=0 dy=0 ticks=(\d+)',s,60)
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w={W} h={H}',s,60);time.sleep(3)
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
  grants=re.findall(rf'\[app.program.memory\] target={target} task={task} granted=(\d+)',t()[s:]);assert grants,'Shapes drew without heap pages'
  still=compare('shapes',refs[(0,0)],tile)
  s2=len(t());g['focus'](surface);time.sleep(.3);q.key('right')
  g['wait'](rf'\[shapes.shown\] w={W} h={H} dx=12 dy=0 ',s2,60);time.sleep(3)
  moved=compare('shapes-moved',refs[(12,0)],tile)
  s3=len(t());q.key('f11');g['wait'](rf'\[shapes.closed\] target={target} task={task}',s3,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s3,90)
  text=t();assert 'KERNEL PANIC' not in text and '[shapes.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'first_draw_ticks':int(first[1]),'checked':[still,moved],'heap_grants':len(grants)})
 finally:g['stop']()
if __name__=='__main__':main()
