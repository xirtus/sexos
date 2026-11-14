#!/usr/bin/env python3
"""The calculator, a program built on sex-app: typed expressions with precedence, errors
said in words (unbalanced brackets, division by zero), the scientific keys in degrees,
the keypad clicked with the mouse, an operator carrying on from the answer, and a past
answer recalled from the history with Up and Enter."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
# Characters to QEMU key names; a tuple is a chord with Shift.
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal','^':('shift','6'),'+':('shift','equal'),'*':('shift','8'),
      '(':('shift','9'),')':('shift','0'),'!':('shift','1'),'%':('shift','5'),'@':('shift','2')}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('sex-calc',(ROOT/'iso_root/programs/sex-calc').read_bytes())])
 rw,rh=map(int,a.resolution.split('x'))
 g=boot(a.iso,disk,a.out,'calc',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def line(text,pattern,timeout=60):
  s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def chord(k):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':k}],'hold-time':80});time.sleep(.2)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def click(x,y):
  move(x,y)
  for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys('run sex-calc');q.key('ret')
  ready=g['wait'](r'\[calc.ready\] target=(\d+) task=(\d+)',s,120);target,task=ready[1],ready[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);sid=int(launch[1])
  g['wait'](rf'\[display.app.pixels\] target={target} ',s,60);time.sleep(3)
  # The whole desktop for the calculator (Super+M), so every key has room.
  g['focus'](sid);time.sleep(.5);s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
  g['wait'](r'\[shell.wm.action\] action=ToggleMonocle done=1',s,30);g['wait'](rf'\[shell.program.resize\] pd={target} task={task} w=\d+ h=\d+ status=0',s,30);time.sleep(2)
  g['focus'](sid);time.sleep(.5)
  results=[]
  for expr,value in [('2+3*4','14'),('2^3^2','512'),('-2^2','-4'),('(1+2)(3+4)','21'),('0.1+0.2','0.3')]:
   # Delete first: a leading minus straight after an answer would carry on from it.
   q.key('delete');time.sleep(.2)
   line(expr,rf'\[calc.result\] expr={re.escape(expr)} value={re.escape(value)}\n');results.append(expr)
  # Errors are said, and the expression stays to be put right; Delete clears it.
  line('(1+2',r'\[calc.error\] expr=\(1\+2 error=Unbalanced\n');q.key('delete');time.sleep(.3)
  line('1/0',r'\[calc.error\] expr=1/0 error=DivideByZero\n');q.key('delete');time.sleep(.3)
  s=len(t());chord('s');g['wait'](r'\[calc.mode\] sci=1 angle=rad\n',s,30)
  s=len(t());chord('d');g['wait'](r'\[calc.mode\] sci=1 angle=deg\n',s,30)
  line('sin(30)+2^-1',r'\[calc.result\] expr=sin\(30\)\+2\^-1 value=1\n')
  line('5!',r'\[calc.result\] expr=5! value=120\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'calc-scientific.ppm')});time.sleep(.5)
  s=len(t());chord('s');g['wait'](r'\[calc.mode\] sci=0 angle=deg\n',s,30);time.sleep(1)
  # The basic pad, clicked: where Layout puts each key for the window's pixel area.
  tile=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,tile[-1])
  size=re.findall(rf'\[shell.program.resize\] pd={target} task={task} w=(\d+) h=(\d+) status=0',t());aw,ah=map(int,size[-1])
  # pixel_region: the area sits inside the border and under the title bar.
  inset=(w-aw)//2;ox,oy=x+inset,y+h-ah-inset
  aw,ah=max(aw,320),max(ah,240)
  hist=min(max(aw*3//10,150),280);pad_w=aw-hist-16;top=92;row_h=(ah-top-8)//6;col_w=pad_w//5
  def key(label,r,c):
   s=len(t());click(ox+8+c*col_w+col_w//2,oy+top+r*row_h+row_h//2);g['wait'](rf'\[calc.key\] label={re.escape(label)}\n',s,30)
  for label,r,c in [('C',1,0),('7',2,0),('*',2,4),('6',3,2)]:key(label,r,c)
  s=len(t());key('=',5,3);g['wait'](r'\[calc.result\] expr=7\*6 value=42\n',s,30)
  # An operator straight after an answer carries on from it.
  line('+8',r'\[calc.result\] expr=ans\+8 value=50\n')
  # Up twice picks the answer before last; Enter puts it in.
  s=len(t());q.key('up');time.sleep(.2);q.key('up');time.sleep(.2);q.key('ret');g['wait'](r'\[calc.recall\] index=1 value=42\n',s,30)
  line('/2',r'\[calc.result\] expr=42/2 value=21\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'calc.ppm')});time.sleep(.5)
  s=len(t());q.key('f11');g['wait'](rf'\[calc.closed\] target={target} task={task}',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
  text=t();assert 'KERNEL PANIC' not in text and '[calc.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'typed':results,'tile':(x,y,w,h)})
 finally:g['stop']()
if __name__=='__main__':main()
