#!/usr/bin/env python3
"""egui on SexOS (crates/sex-egui): Tassel, an ordinary Rust std program, runs egui.

`run tassel probe` shows a fixed panel: a click on its button clicks it, a click in its
text field then typed keys fill the field, a click toggles its checkbox and wheel steps
scroll its list, each read back from the program's serial record. `run tassel` then runs
egui's own demo app, unmodified, which must paint its windows (thousands of triangles)
and take a click. Screenshots of both are kept."""
import argparse,json,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_gate_geometry import maximize
from check_disk_persistence import ppm_rgb
ROOT=Path(__file__).resolve().parents[1]
def client_rect(tile,width,height):
 """The client of the actual full-viewport frame; never include its chrome."""
 x,y,w,h=tile;side=(w-width)//2;top=h-height-side
 assert (side,top,w-width,h-height)==(8,34,16,42),(tile,width,height)
 return x+side,y+top,width,height

def colours(data,rect):
 """Distinct colours inside only the published client rectangle."""
 width,height,pixels=ppm_rgb(data,min_width=1,min_height=1)
 x,y,w,h=rect;seen=set()
 assert w>0 and h>0 and x>=0 and y>=0 and x+w<=width and y+h<=height,rect
 for row in range(y,y+h):
  base=(row*width)*3;line=pixels[base+x*3:base+(x+w)*3]
  seen.update(line[k:k+3] for k in range(0,len(line),3))
 return seen

def capture_pixels(q,out,name,rect,minimum,published,timeout=30):
 """Qualify fresh scanout, not merely an application's completed CPU paint.

 The publication must belong to this run's surface and full viewport. Poll
 pacing is not evidence: only a fresh screenshot satisfying the same colour
 predicate can pass. Keep every unsuccessful capture for diagnosis.
 """
 deadline=time.monotonic()+timeout;attempts=[]
 while time.monotonic()<deadline:
  path=out/f'{name}-capture-{len(attempts)+1:03d}.ppm'
  q.cmd('screendump',{'filename':str(path)})
  seen=colours(path.read_bytes(),rect);ready=bool(published())
  attempts.append(dict(file=path.name,colours=len(seen),published=ready))
  if ready and len(seen)>minimum:
   shutil.copyfile(path,out/f'{name}.ppm')
   (out/f'{name}-captures.json').write_text(json.dumps(dict(ok=True,client=rect,minimum_exclusive=minimum,attempts=attempts),indent=2)+'\n')
   return seen
  time.sleep(.05)
 (out/f'{name}-captures.json').write_text(json.dumps(dict(ok=False,client=rect,minimum_exclusive=minimum,attempts=attempts),indent=2)+'\n')
 raise AssertionError(f'{name} never published fresh client pixels with >{minimum} colours before deadline; attempts={attempts}')
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
  # Both the wheel probe and full demo need a reviewable viewport. Preserve
  # the demo's triangle oracle by waiting for its actual maximized paint.
  tile,before=maximize(g,surface)
  cursor=before;deadline=time.monotonic()+120
  while True:
   frame=g['wait'](r'\[egui.frame\] n=\d+ w=(\d+) h=(\d+) meshes=(\d+) triangles=(\d+) pixels=(\d+) ms=(\d+)',cursor,max(1,deadline-time.monotonic()))
   cursor+=frame.end()
   if (int(frame[1]),int(frame[2]))==(tile[2]-16,tile[3]-42):break
   assert time.monotonic()<deadline,'client never painted its maximized viewport'
  assert tile[0]>=0 and tile[1]>=0 and tile[0]+tile[2]<=rw and tile[1]+tile[3]<=rh,tile
  return target,task,surface,tile,frame,before
 def shot(name,tile,frame,target,surface,before,minimum):
  width,height=int(frame[1]),int(frame[2]);rect=client_rect(tile,width,height)
  pattern=rf'\[display.app.pixels\] target={target} surface={surface} w={width} h={height}(?:\r?\n|$)'
  return capture_pixels(q,a.out,name,rect,minimum,lambda:re.search(pattern,t()[before:]))
 def close(target,task,surface):
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  target,task,surface,tile,frame,before=start('probe')
  ox,oy,_,_=client_rect(tile,int(frame[1]),int(frame[2]))
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
  probe=shot('tassel-probe',tile,frame,target,surface,before,8)
  assert len(probe)>8,'the probe panel looks blank'
  close(target,task,surface)
  target,task,surface,tile,frame,before=start('trace')
  assert int(frame[4])>1000,f'the demo drew too little: {frame[0]}'
  demo=shot('tassel-demo',tile,frame,target,surface,before,40)
  colour_count=len(demo);assert colour_count>40,f'the demo looks blank ({colour_count} colours)'
  # A click on the demo's background still takes input without trouble.
  click(tile[0]+tile[2]//2,tile[1]+tile[3]//2);time.sleep(2)
  close(target,task,surface)
  text=t();assert 'KERNEL PANIC' not in text and 'panicked' not in text
  print({'ok':True,'demo_first_frame':frame[0],'colours':colour_count})
 finally:g['stop']()
if __name__=='__main__':main()
