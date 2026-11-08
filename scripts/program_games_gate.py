#!/usr/bin/env python3
"""Games: `run solitaire`, `run chess` and `run maze` start Web on its built-in game pages, which are
JavaScript on Canvas 2D played from the keyboard. Solitaire must match the host render
(sex-web's page example) pixel for pixel when dealt and after drawing a card; Chess when
set up and after picking a pawn, and the computer must answer a move within the page's
script time limit. 3D Chess (three.js) must match the host render when set up and after picking a pawn,
and answer a move. Maze (WebGL) must match the host render when it starts and after
walking one cell, and its walk is timed."""
import argparse,re,shutil,subprocess,time
from pathlib import Path
from PIL import Image
from install_program import install
import os
# Loading three.js takes a while under emulation.
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
GAMES=ROOT/'apps/web/src/games'
W,H=410,330
def reference(out,page,bar,name,actions=''):
 path=out/f'reference-{name}.ppm'
 subprocess.run(['cargo','run','-q','--release','-p','sex-web','--example','page','--',str(page),str(path),str(W),str(H),'0','-1',bar,actions],cwd=ROOT,check=True)
 return Image.open(path).convert('RGB')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 progs=[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in ('solitaire','chess','maze','chess3d')]
 assert all(e[:4]==b'\x7fELF' for _,e in progs)
 install(disk,progs)
 sol_bar='Solitaire — about:solitaire';chess_bar='Chess — about:chess'
 refs={'solitaire':reference(a.out,GAMES/'solitaire.html',sol_bar,'solitaire'),
  'solitaire-drawn':reference(a.out,GAMES/'solitaire.html',sol_bar,'solitaire-drawn','keyArrowUp,keyEnter'),
  'chess':reference(a.out,GAMES/'chess.html',chess_bar,'chess'),
  'chess-picked':reference(a.out,GAMES/'chess.html',chess_bar,'chess-picked','keyEnter'),
  'maze':reference(a.out,GAMES/'maze.html','Maze — about:maze','maze'),
  'maze-walked':reference(a.out,GAMES/'maze.html','Maze — about:maze','maze-walked','keyArrowUp'),
  'chess3d':reference(a.out,GAMES/'chess3d.html','3D Chess — about:chess3d','chess3d'),
  'chess3d-picked':reference(a.out,GAMES/'chess3d.html','3D Chess — about:chess3d','chess3d-picked','keyEnter')}
 g=boot(a.iso,disk,a.out,'games',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def compare(name,tile):
  ref=refs[name]
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
 def start(name):
  s=len(t());run(f'run {name}')
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,300);target,task=m[1],m[2]
  loaded=g['wait'](r'\[web.loaded\] .*script_ticks=(\d+) scripted=(\d)',s,60);assert loaded[2]=='1',loaded[0]
  # The games draw their first frame on a timer tick after loading.
  g['wait'](r'\[web.loaded\][^\n]*\n(?:.*\n)*?\[web.shown\]',s,120)
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(3)
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
  return target,task,surface,tile,int(loaded[1])
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,300)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 # SEXOS_GAMES=chess3d,maze runs only those (all by default).
 want=os.environ.get('SEXOS_GAMES','solitaire,chess,maze,chess3d').split(',')
 checked=[];result={'ok':True}
 def solitaire():
  target,task,surface,tile,ticks=start('solitaire')
  checked.append(compare('solitaire',tile))
  g['focus'](surface);time.sleep(.3)
  s=len(t());q.key('up');g['wait'](r'\[web.shown\]',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[web.shown\]',s,60);time.sleep(2)
  checked.append(compare('solitaire-drawn',tile))
  close(target,task)
  result['solitaire']={'load_ticks':ticks}
 def chess():
  target,task,surface,tile,ticks=start('chess')
  checked.append(compare('chess',tile))
  g['focus'](surface);time.sleep(.3)
  s=len(t());q.key('ret');g['wait'](r'\[web.shown\]',s,60);time.sleep(2)
  checked.append(compare('chess-picked',tile))
  # e2-e4, then the computer's reply, drawn on the next tick.
  for k in ('up','up'):s=len(t());q.key(k);g['wait'](r'\[web.shown\]',s,60)
  s=len(t());q.key('ret')
  reply=g['wait'](r'\[web.console\] level=0 chess move (\S+) depth (\d+) nodes (\d+) ms (\d+)\n',s,120)
  g['wait'](r'chess move [^\n]*\n(?:.*\n)*?\[web.shown\]',s,120)
  close(target,task)
  result['chess']={'load_ticks':ticks,'reply':reply[1],'depth':int(reply[2]),'nodes':int(reply[3]),'think_ms':int(reply[4])}
 def maze():
  s=len(t());target,task,surface,tile,ticks=start('maze')
  g['wait'](r'\[web.console\] level=0 maze ready',s,120);time.sleep(2)
  checked.append(compare('maze',tile))
  g['focus'](surface);time.sleep(.3)
  s=len(t());began=time.time();q.key('up')
  g['wait'](r'\[web.console\] level=0 maze stopped cell (\d+) facing (\d+) frames (\d+)',s,300);walk_s=round(time.time()-began,1)
  g['wait'](r'\[web.shown\]',s,60);time.sleep(2)
  checked.append(compare('maze-walked',tile))
  close(target,task)
  result['maze']={'load_ticks':ticks,'walk_seconds':walk_s}
 def chess3d():
  # three.js and the shared chess engine, built into Web.
  s=len(t());target,task,surface,tile,ticks=start('chess3d')
  g['wait'](r'\[web.console\] level=0 chess3d drawn',s,600);time.sleep(5)
  shaders=[int(m) for m in re.findall(r'chess3d shader ms (\d+)',t()[s:])]
  checked.append(compare('chess3d',tile))
  g['focus'](surface);time.sleep(.3)
  s=len(t());began=time.time();q.key('ret');g['wait'](r'chess3d drawn[^\n]*\n(?:.*\n)*?\[web.shown\]',s,300);pick_s=round(time.time()-began,1);time.sleep(2)
  checked.append(compare('chess3d-picked',tile))
  for k in ('up','up'):s=len(t());q.key(k);g['wait'](r'chess3d drawn[^\n]*\n(?:.*\n)*?\[web.shown\]',s,300)
  s=len(t());began=time.time();q.key('ret')
  reply=g['wait'](r'\[web.console\] level=0 chess move (\S+) depth (\d+) nodes (\d+) ms (\d+)\n',s,300)
  g['wait'](r'chess move [^\n]*\n(?:.*\n)*?\[web.console\] level=0 chess3d drawn[^\n]*\n(?:.*\n)*?\[web.shown\]',s,300);reply_s=round(time.time()-began,1)
  close(target,task)
  result['chess3d']={'load_ticks':ticks,'shader_ms':shaders,'pick_seconds':pick_s,'reply':reply[1],'move_and_reply_seconds':reply_s}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  for name,play in (('solitaire',solitaire),('chess',chess),('maze',maze),('chess3d',chess3d)):
   if name in want:play()
  text=t();assert '[web.script] trapped' not in text and 'InternalError: interrupted' not in text,'a game ran past the script time limit'
  assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  result['checked']=checked
  print(result)
 finally:g['stop']()
if __name__=='__main__':main()
