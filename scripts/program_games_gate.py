#!/usr/bin/env python3
"""Games: `run solitaire` and `run chess` start Web on its built-in game pages, which are
JavaScript on Canvas 2D played from the keyboard. Solitaire must match the host render
(sex-web's page example) pixel for pixel when dealt and after drawing a card; Chess when
set up and after picking a pawn, and the computer must answer a move within the page's
script time limit."""
import argparse,re,shutil,subprocess,time
from pathlib import Path
from PIL import Image
from install_program import install
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
 progs=[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in ('solitaire','chess')]
 assert all(e[:4]==b'\x7fELF' for _,e in progs)
 install(disk,progs)
 sol_bar='Solitaire — about:solitaire';chess_bar='Chess — about:chess'
 refs={'solitaire':reference(a.out,GAMES/'solitaire.html',sol_bar,'solitaire'),
  'solitaire-drawn':reference(a.out,GAMES/'solitaire.html',sol_bar,'solitaire-drawn','keyArrowUp,keyEnter'),
  'chess':reference(a.out,GAMES/'chess.html',chess_bar,'chess'),
  'chess-picked':reference(a.out,GAMES/'chess.html',chess_bar,'chess-picked','keyEnter')}
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
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(3)
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
  return target,task,surface,tile,int(loaded[1])
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  target,task,surface,tile,sol_ticks=start('solitaire')
  checked=[compare('solitaire',tile)]
  g['focus'](surface);time.sleep(.3)
  s=len(t());q.key('up');g['wait'](r'\[web.shown\]',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[web.shown\]',s,60);time.sleep(2)
  checked.append(compare('solitaire-drawn',tile))
  close(target,task)
  target,task,surface,tile,chess_ticks=start('chess')
  checked.append(compare('chess',tile))
  g['focus'](surface);time.sleep(.3)
  s=len(t());q.key('ret');g['wait'](r'\[web.shown\]',s,60);time.sleep(2)
  checked.append(compare('chess-picked',tile))
  # e2-e4, then the computer's reply.
  for k in ('up','up'):s=len(t());q.key(k);g['wait'](r'\[web.shown\]',s,60)
  s=len(t());q.key('ret')
  reply=g['wait'](r'\[web.console\] level=0 chess move (\S+) depth (\d+) nodes (\d+) ms (\d+)\n',s,120)
  text=t()
  assert '[web.script] trapped' not in text,'the computer ran past the script time limit'
  close(target,task)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'script_ticks':{'solitaire':sol_ticks,'chess':chess_ticks},'reply':reply[1],'depth':int(reply[2]),'nodes':int(reply[3]),'think_ms':int(reply[4]),'checked':checked})
 finally:g['stop']()
if __name__=='__main__':main()
