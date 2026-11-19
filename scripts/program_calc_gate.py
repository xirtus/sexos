#!/usr/bin/env python3
"""Abacus, the calculator (apps/abacus, on Weft), with what the old sex-calc had:
typed expressions with precedence, errors said in words (unbalanced brackets, division
by zero), Ctrl+D between radians and degrees, scientific functions and factorials, the
keypad clicked with the mouse, the memory keys, an operator carrying on from the
answer, and a past answer picked from the history with Up and put in with Enter."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_gate_geometry import colours,maximize
ROOT=Path(__file__).resolve().parents[1]
# Characters to QEMU key names; a tuple is a chord with Shift.
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal','^':('shift','6'),'+':('shift','equal'),'*':('shift','8'),
      '(':('shift','9'),')':('shift','0'),'!':('shift','1'),'%':('shift','5')}
# The keypad in abacus::KEYS order, as the gate names the keys.
LABELS=['MC','MR','M+','M-','neg','C','bs','(',')','%','sin','cos','tan','sqrt','^','7','8','9','div','pi',
        '4','5','6','mul','ln','1','2','3','minus','log','0','.','ans','+','=']
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/abacus').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('abacus',elf)])
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
  g['focus'](153);time.sleep(.5);s=len(t());keys('run abacus');q.key('ret')
  ready=g['wait'](r'\[abacus.ready\] target=(\d+) task=(\d+) keys=(\d+)',s,300);target,task=ready[1],ready[2];assert int(ready[3])==len(LABELS),ready[0]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);sid=int(launch[1])
  g['wait'](r'\[abacus.frame\] n=1\n',s,300);time.sleep(3)
  # The keypad needs room: zoom the window, then read where the keys went.
  tile,before=maximize(g,sid)
  lay=g['wait'](r'\[abacus.layout\] w=(\d+) h=(\d+)\n',before,60);s=before+t()[before:].index(lay[0])
  g['wait'](rf'\[abacus.key\] n={len(LABELS)-1} x=(-?\d+) y=(-?\d+)\n',s,60);time.sleep(1)
  pos={LABELS[int(n)]:(int(x),int(y)) for n,x,y in re.findall(r'\[abacus.key\] n=(\d+) x=(-?\d+) y=(-?\d+)',t()[s:])}
  assert len(pos)==len(LABELS),pos
  ox,oy=tile[0]+8,tile[1]+36
  def key(label):x,y=pos[label];click(ox+x,oy+y)
  g['focus'](sid);time.sleep(.5)
  results=[]
  for expr,value in [('2+3*4','14'),('2^3^2','512'),('-2^2','-4'),('(1+2)(3+4)','21'),('0.1+0.2','0.3'),('7 mod 3','1')]:
   # Escape first: an operator straight after an answer would carry on from it.
   q.key('esc');time.sleep(.2)
   line(expr,rf'\[abacus.result\] expr={re.escape(expr)} value={re.escape(value)}\n');results.append(expr)
  # Errors are said, and the expression stays to be put right; Escape clears it.
  line('(1+2',r"\[abacus.error\] what=Brackets don't match\n");q.key('esc');time.sleep(.3)
  line('1/0',r"\[abacus.error\] what=Can't divide by zero\n");q.key('esc');time.sleep(.3)
  s=len(t());chord('d');g['wait'](r'\[abacus.mode\] angle=rad\n',s,30)
  s=len(t());chord('d');g['wait'](r'\[abacus.mode\] angle=deg\n',s,30)
  line('sin(30)+2^-1',r'\[abacus.result\] expr=sin\(30\)\+2\^-1 value=1\n')
  q.key('esc');time.sleep(.2)
  line('5!',r'\[abacus.result\] expr=5! value=120\n')
  # The keypad, clicked.
  for k in ['C','7','mul','6']:key(k)
  s=len(t());key('=');g['wait'](r'\[abacus.result\] expr=7\*6 value=42\n',s,30)
  # M+ with an empty line keeps the answer.
  s=len(t());key('M+');g['wait'](r'\[abacus.memory\] value=42\n',s,30)
  # An operator straight after an answer carries on from it.
  line('+8',r'\[abacus.result\] expr=ans\+8 value=50\n')
  # Up twice picks the answer before last; Enter puts it in.
  s=len(t());q.key('up');time.sleep(.2);q.key('up');time.sleep(.2);q.key('ret');g['wait'](r'\[abacus.recall\] index=1 value=42\n',s,30)
  line('/2',r'\[abacus.result\] expr=42/2 value=21\n')
  # MR puts the memory in; ± changes its sign.
  key('MR');key('neg')
  s=len(t());key('=');g['wait'](r'\[abacus.result\] expr=-\(42\) value=-42\n',s,30)
  time.sleep(1);path=a.out/'calc.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  n=len(colours(path.read_bytes(),tile));assert n>8,f'the window looks blank ({n} colours)'
  s=len(t());g['focus'](sid);time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  text=t();assert 'KERNEL PANIC' not in text and '[abacus.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'typed':results,'tile':tuple(tile),'colours':n})
 finally:g['stop']()
if __name__=='__main__':main()
