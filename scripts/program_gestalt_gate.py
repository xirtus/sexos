#!/usr/bin/env python3
"""Gestalt, the vector editor (VectorCraft's engine, built for x86_64-sex-std.json):

* `run gestalt` writes welcome.svg into its folder, opens it (four objects on an 800 x 600 pt
  artboard) and draws it; maximized, it fits the drawing again, and the screendump shows its
  red, blue and yellow.
* M picks the Rectangle tool; a drag on the canvas draws a fifth object, selected.
* Ctrl+Z takes it away and Ctrl+Shift+Z puts it back.
* V, Ctrl+A selects all five; Ctrl+G groups them into one object.
* Ctrl+S writes the SVG; Ctrl+E writes a PNG beside it; the window closes.
* A second `run gestalt` reads the saved SVG back: one object (the group).

No kernel panic, page fault or std abort anywhere in the record."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
from program_gate_geometry import colours,current_tile,maximize
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
STRIP=40

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/gestalt').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('gestalt',elf)])
 g=boot(a.iso,disk,a.out,'gestalt',a.resolution);t=g['text'];q=g['q']
 rw,rh=map(int,a.resolution.split('x'))
 def type_line(line):
  for k in line:
   q.key({' ':'spc','.':'dot'}.get(k,k));time.sleep(.05)
 def opened(objects):
  s=len(t());g['focus'](153);time.sleep(.5);type_line('run gestalt');q.key('ret')
  o=g['wait'](rf'\[gestalt\.open\] target=(\d+) task=(\d+) name=welcome\.svg objects={objects} artboard=800x600\n',s,900)
  g['wait'](r'\[gestalt\.render\] width=\d+ height=\d+ zoom=[\d.]+ objects=\d+ ms=\d+\n',s,900)
  g['wait'](rf'\[gestalt\.ready\] target={o[1]} task={o[2]}\n',s,300)
  g['focus'](SURFACE[int(o[1])]);time.sleep(.8)
  return o
 def press(*keys):
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]});time.sleep(.4)
 def close(o):
  s=len(t());q.key('f11');g['wait'](r'\[gestalt\.closed\]\n',s,120)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  o=opened(4)
  # Maximized, the drawing fits the bigger window again.
  _,s=maximize(g,SURFACE[int(o[1])])
  g['wait'](r'\[gestalt\.render\] width=(\d{3,}) height=(\d{3,}) zoom=[\d.]+ objects=4 ms=\d+\n',s,300)
  time.sleep(1.5);shot=a.out/'gestalt-opened.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  tile=current_tile(t(),SURFACE[int(o[1])])
  seen=colours(shot.read_bytes(),tile)
  for name,rgb in(('red',b'\xe8\x45\x3c'),('blue',b'\x1a\x73\xe8'),('yellow',b'\xf9\xab\x00')):
   assert rgb in seen,f'the welcome drawing\'s {name} does not show ({len(seen)} colours)'
  def at(x,y):q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round((tile[0]+8+x)*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round((tile[1]+36+y)*32767/(rh-1))}}]});time.sleep(.15)
  def btn(down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.15)
  # Rectangle tool, then a drag near the canvas's top left.
  s=len(t());q.key('m');g['wait'](r'\[gestalt\.tool\] id=rectangle\n',s,60)
  s=len(t());at(STRIP+30,20);btn(True)
  for i in range(1,6):at(STRIP+30+i*16,20+i*10)
  btn(False)
  m=g['wait'](r'\[gestalt\.gesture\] tool=rectangle objects=5 selected=1 box=-?\d+,-?\d+,(\d+),(\d+)\n',s,120)
  assert int(m[1])>0 and int(m[2])>0,m[0]
  g['chord']('ctrl','z',r'\[gestalt\.command\] edit\.undo objects=4 selected=\d+\n')
  s=len(t());press('ctrl','shift','z');g['wait'](r'\[gestalt\.command\] edit\.redo objects=5 selected=\d+\n',s,120)
  s=len(t());q.key('v');g['wait'](r'\[gestalt\.tool\] id=selection\n',s,60)
  g['chord']('ctrl','a',r'\[gestalt\.command\] select\.all objects=5 selected=5\n')
  g['chord']('ctrl','g',r'\[gestalt\.command\] object\.group objects=1 selected=1\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'gestalt-edited.ppm')})
  m=g['chord']('ctrl','s',r'\[gestalt\.saved\] name=welcome\.svg bytes=(\d+) objects=1\n')
  assert int(m[1])>300,m[0]
  e=g['chord']('ctrl','e',r'\[gestalt\.exported\] name=welcome\.png format=png bytes=(\d+)\n')
  assert int(e[1])>1000,e[0]
  close(o)
  o=opened(1)
  close(o)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'saved_bytes':int(m[1]),'png_bytes':int(e[1])})
 finally:g['stop']()
if __name__=='__main__':main()
