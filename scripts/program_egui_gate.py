#!/usr/bin/env python3
"""egui on SexOS (crates/sex-egui): Tassel, an ordinary Rust std program, runs egui.

`run tassel probe` shows a fixed panel: a click on its button clicks it, a click in its
text field then typed keys fill the field, a click toggles its checkbox and wheel steps
scroll its list, each read back from the program's serial record. `run tassel` then runs
egui's own demo app, unmodified, which must paint its windows (thousands of triangles)
and take a click. Screenshots of both are kept."""
import argparse,re,shutil,time
from pathlib import Path
from PIL import Image
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/tassel').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('tassel',elf)])
 rw,rh=map(int,a.resolution.split('x'))
 g=boot(a.iso,disk,a.out,'egui',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def button(name,down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':name}}]});time.sleep(.2)
 def click(x,y):move(x,y);button('left',True);button('left',False)
 def start(args):
  s=len(t());run('run tassel'+(' '+args if args else ''))
  m=g['wait'](r'\[egui.ready\] target=(\d+) task=(\d+) ',s,300);target,task=m[1],m[2]
  frame=g['wait'](r'\[egui.frame\] n=1 w=(\d+) h=(\d+) meshes=(\d+) triangles=(\d+) pixels=(\d+) ms=(\d+)',s,300)
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(3)
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
  return target,task,surface,tile,frame
 def shot(name,tile):
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  x,y,w,h=tile;img=Image.open(path).convert('RGB').crop((x+8,y+36,x+w-8,y+h-8));img.save(a.out/f'{name}.png');return img
 def close(target,task,surface):
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  target,task,surface,tile,frame=start('probe')
  ox,oy=tile[0]+8,tile[1]+36
  s=len(t());click(ox+60,oy+24);g['wait'](r'\[tassel.clicked\] count=1\n',s,60)
  s=len(t());click(ox+60,oy+24);g['wait'](r'\[tassel.clicked\] count=2\n',s,60)
  s=len(t());click(ox+110,oy+64);time.sleep(.5)
  for k in 'sex':q.key(k);time.sleep(.1)
  g['wait'](r'\[tassel.text\] value=sex\n',s,60)
  s=len(t());q.key('backspace');g['wait'](r'\[tassel.text\] value=se\n',s,60)
  s=len(t());click(ox+20,oy+102);g['wait'](r'\[tassel.checked\] value=true\n',s,60)
  s=len(t());move(ox+100,oy+190)
  for _ in range(4):button('wheel-down',True);button('wheel-down',False)
  m=g['wait'](r'\[tassel.scrolled\] first=(\d+)\n',s,60);assert int(m[1])>0,m[0]
  probe=shot('tassel-probe',tile)
  assert len(set(probe.getdata()))>8,'the probe panel looks blank'
  close(target,task,surface)
  target,task,surface,tile,frame=start('')
  assert int(frame[4])>1000,f'the demo drew too little: {frame[0]}'
  demo=shot('tassel-demo',tile)
  colours=len(set(demo.getdata()));assert colours>40,f'the demo looks blank ({colours} colours)'
  # A click on the demo's background still takes input without trouble.
  click(tile[0]+tile[2]//2,tile[1]+tile[3]//2);time.sleep(2)
  close(target,task,surface)
  text=t();assert 'KERNEL PANIC' not in text and 'panicked' not in text
  print({'ok':True,'demo_first_frame':frame[0],'colours':colours})
 finally:g['stop']()
if __name__=='__main__':main()
