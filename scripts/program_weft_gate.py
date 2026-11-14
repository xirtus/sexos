#!/usr/bin/env python3
"""Weft on SexOS (crates/weft): Abacus, the calculator, is a no_std program built on it.

`run abacus` paints its window with Weft. Typed digits go to its expression line, a
click on the keypad's + key adds to it and Enter works it out; keypad clicks alone
work out 7 × 6; a division by zero shows an error; Ctrl+Shift+P opens the command
palette, where typing finds Radians and Enter runs it. Each step is read back from the
program's serial record, and a screenshot is kept."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_egui_gate import colours
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/abacus').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('abacus',elf)])
 rw,rh=map(int,a.resolution.split('x'))
 g=boot(a.iso,disk,a.out,'weft',a.resolution);t=g['text'];q=g['q']
 def keys(names):
  for k in names:q.key(k);time.sleep(.08)
 def run(line):
  g['focus'](153);time.sleep(.5)
  keys(['spc' if k==' ' else k for k in line]);q.key('ret')
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def button(name,down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':name}}]});time.sleep(.2)
 def click(x,y):move(x,y);button('left',True);button('left',False)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());run('run abacus')
  m=g['wait'](r'\[abacus.ready\] target=(\d+) task=(\d+) keys=(\d+)',s,300);target,task=m[1],m[2];assert int(m[3])==30,m[0]
  g['wait'](r'\[abacus.frame\] n=1\n',s,300)
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(3)
  # Keys in abacus::KEYS order: C ⌫ ( ) % / sin cos tan √ ^ / 7 8 9 ÷ π / 4 5 6 × ln / 1 2 3 − log / 0 . ans + =
  labels=['C','bs','(',')','%','sin','cos','tan','sqrt','^','7','8','9','div','pi','4','5','6','mul','ln','1','2','3','minus','log','0','.','ans','+','=']
  pos={labels[int(n)]:(int(x),int(y)) for n,x,y in re.findall(r'\[abacus.key\] n=(\d+) x=(-?\d+) y=(-?\d+)',t()[s:])}
  assert len(pos)==30,pos
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
  ox,oy=tile[0]+8,tile[1]+36
  def key(label):x,y=pos[label];click(ox+x,oy+y)
  # Typed digits, a clicked +, typed digits, Enter.
  g['focus'](surface);time.sleep(.3)
  s=len(t());keys(['1','2']);key('+');keys(['3','0']);q.key('ret')
  g['wait'](r'\[abacus.result\] expr=12\+30 value=42\n',s,60)
  # The keypad alone.
  s=len(t())
  for k in ['7','mul','6','=']:key(k)
  g['wait'](r'\[abacus.result\] expr=7\*6 value=42\n',s,60)
  s=len(t());keys(['1','slash','0','ret'])
  g['wait'](r"\[abacus.error\] what=Can't divide by zero\n",s,60)
  # The command palette.
  s=len(t())
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,('ctrl','shift','p')),(False,('p','shift','ctrl')))for x in ks]});time.sleep(.5)
  keys(['r','a','d','i']);q.key('ret')
  g['wait'](r'\[abacus.command\] id=abacus.radians\n',s,60)
  # Extras use real palette commands and keyboard input in the same Weft window.
  def chord(*codes):
   q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,codes),(False,tuple(reversed(codes))))for x in ks]})
  def text(value):
   symbols={'+':('shift','equal'),'*':('shift','8'),'(':('shift','9'),')':('shift','0')}
   for c in value:
    if c in symbols:chord(*symbols[c])
    elif c.isupper():chord('shift',c.lower())
    else:q.key({' ':'spc','-':'minus','.':'dot','/':'slash'}.get(c,c))
    time.sleep(.08)
  def command(query,identity):
   start=len(t());chord('ctrl','shift','p');time.sleep(.3);text(query);q.key('ret')
   g['wait'](rf'\[abacus.command\] id={identity}\n',start,60);time.sleep(.2)
  def expression(value,result):
   start=len(t());chord('ctrl','a');text(value);q.key('ret')
   g['wait'](rf'\[abacus.result\] expr={re.escape(value)} value={re.escape(result)}\n',start,60)
  command('8-bit Words','abacus.bits8');expression('255+1','0')
  command('64-bit Words','abacus.bits64');command('Hexadecimal Output','abacus.hex')
  expression('18446744073709551615','0xFFFFFFFFFFFFFFFF')
  q.cmd('screendump',{'filename':str(a.out/'abacus-programmer.ppm')})
  command('Unit Conversion','abacus.convert');expression('32 F to C','0 C')
  command('Recall Last Calculation','abacus.recall-last')
  start=len(t());q.key('ret');g['wait'](r'\[abacus.result\] expr=32 F to C value=0 C\n',start,60)
  expression('1 MiB to KiB','1024 KiB')
  start=len(t());text('1 m to kg');q.key('ret');g['wait'](r'\[abacus.error\] what=Units: Dimension\n',start,60)
  q.cmd('screendump',{'filename':str(a.out/'abacus-converter.ppm')})
  command('Scientific Mode','abacus.scientific');expression('cbrt(-27)','-3')
  time.sleep(1)
  path=a.out/'abacus.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  n=len(colours(path.read_bytes(),tile));assert n>8,f'the window looks blank ({n} colours)'
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  text=t();assert 'KERNEL PANIC' not in text and 'abacus.fatal' not in text
  print({'ok':True,'colours':n})
 finally:g['stop']()
if __name__=='__main__':main()
