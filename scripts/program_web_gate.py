#!/usr/bin/env python3
"""Web pages in a program: `run web` parses, styles and lays out its welcome page with
sex-html and draws it with Geist (sex-web) into its pixel surface. The screen must match
the same page rendered on the host by `cargo run -p sex-web --example page`, pixel for
pixel, before and after scrolling with Down and after Right focuses a link."""
import argparse,re,shutil,subprocess,time
from pathlib import Path
from PIL import Image
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
W,H=410,330
PAGE=ROOT/'apps/web/src/welcome.html'
BAR_TEXT='Welcome to Web — about:welcome'
def reference(out,scroll,focus):
 path=out/f'reference-{scroll}-{focus}.ppm'
 subprocess.run(['cargo','run','-q','-p','sex-web','--example','page','--',str(PAGE),str(path),str(W),str(H),str(scroll),str(focus),BAR_TEXT],cwd=ROOT,check=True)
 return Image.open(path).convert('RGB')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/web').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('web',elf)])
 g=boot(a.iso,disk,a.out,'web',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def compare(name,scroll,focus,tile):
  ref=reference(a.out,scroll,focus)
  shot=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  x,y,w,h=tile;img=Image.open(shot).convert('RGB');bx,by=x+8,y+36
  vw,vh=min(W,w-16),min(H,h-44)
  checked=bad=worst=0
  for py in range(0,vh,3):
   for px in range(0,vw,5):
    got,want=img.getpixel((bx+px,by+py)),ref.getpixel((px,py))
    d=max(abs(i-j)for i,j in zip(got,want));checked+=1;worst=max(worst,d);bad+=d>2
  img.crop((bx,by,bx+vw,by+vh)).save(a.out/f'{name}.png')
  assert checked>5000 and bad==0,f'{name}: {bad} of {checked} pixels differ (worst {worst})'
  return checked,worst
 def shown(s):
  m=g['wait'](rf'\[web.shown\] w={W} h={H} scroll=(\d+) focus=(-?\d+) items=(\d+) ticks=(\d+)',s,120);time.sleep(3)
  return int(m[1]),int(m[2]),int(m[4])
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());run('run web')
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,240);target,task=m[1],m[2]
  loaded=g['wait'](r'\[web.loaded\] .*items=(\d+) links=(\d+) height=(\d+).*layout_ticks=(\d+)',s,60)
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w={W} h={H}',s,60);time.sleep(3)
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
  first=compare('web',0,-1,tile)
  s2=len(t());g['focus'](surface);time.sleep(.3);q.key('down')
  scroll,focus,ticks=shown(s2);assert scroll==40 and focus==-1,(scroll,focus)
  scrolled=compare('web-scrolled',scroll,focus,tile)
  s3=len(t());q.key('right')
  scroll,focus,ticks2=shown(s3);assert focus==0,focus
  focused=compare('web-focused',scroll,focus,tile)
  s4=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s4,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s4,90)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'items':int(loaded[1]),'links':int(loaded[2]),'height':int(loaded[3]),'layout_ticks':int(loaded[4]),'paint_ticks':[ticks,ticks2],'checked':[first,scrolled,focused]})
 finally:g['stop']()
if __name__=='__main__':main()
