#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""An SDL2 game runs on SexOS: apps/driftspark (C), linked with SDL2 2.30.9 and the
SexOS backends in crates/sex-sdl.

The gate types commands into Spindle, as a person would, and checks:

* demo: `run driftspark --demo 11 900` plays 900 frames with scripted input. Its
  score line and the checksum of its last frame must be the expected ones, because
  the game is deterministic (integer simulation, one step a frame). It must exit 0.
* sound: the game synthesises its effects in the SDL audio callback. The backend's
  record line must count non-silent frames, and the sound card's output (QEMU's Intel
  HD Audio, recorded to a WAV file) must hold several separate sounds.
* window: `run driftspark` shows the title screen. A QEMU screendump of the window
  must show the title's amber letters on the game's dark field.
* keys: Space launches a game and a Right key moves the craft right, by the game's
  report and on screen (the craft's cyan pixels). P pauses and resumes, and Esc quits
  with status 0 and closes the window."""
import argparse,os,re,shutil,time
from pathlib import Path
from PIL import Image
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','900')
ROOT=Path(__file__).resolve().parents[1]

PLAIN={' ':'spc','.':'dot','-':'minus'}
W,H,HUD=320,240,12
SEED,FRAMES=11,900
# What `driftspark --demo 11 900` ends with; the game computes it the same on any machine.
DEMO=r'demo seed=11 frames=900 score=417 level=3 lives=2 sparks=21'
CHECKSUM='7cee900e'

def near(a,b,tol=10):return all(abs(((a>>s)&255)-((b>>s)&255))<=tol for s in(0,8,16))

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/driftspark').read_bytes();assert elf[:4]==b'\x7fELF'
 if b'This build has no C compiler output (musl headers were missing).' in elf:
  raise SystemExit('driftspark is the no-compiler fallback: install musl headers or set MUSL_INCLUDE, then rebuild before running this gate')
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('driftspark',elf)])
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=a0,path={wav},out.frequency=48000,out.channels=2 -device intel-hda -device hda-output,audiodev=a0'
 from quil_autosave_gate import boot
 from sound_gate import segments
 g=boot(a.iso,disk,out,'driftspark',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
  for ch in line:
   s=len(t());q.key(PLAIN.get(ch,ch))
   g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=\d+\n',s,30)
  q.key('ret')
 report=[]
 def line(pattern,s,timeout=120):
  m=g['wait'](r'\[std\.out\] \[driftspark\] '+pattern+r'\n',s,timeout);report.append(m[0].strip());return m
 def launched(s):
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60);return int(m[1]),int(m[2]),int(m[3])
 def window(surface,target,task,name):
  """The window's pixels as Display shows them, from a screendump: (image, origin, visible size)."""
  shot=out/f'{name}.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  tile=re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());assert tile,'no tile for the window'
  area=re.findall(rf'\[shell.program.resize\] pd={target} task={task} w=(\d+) h=(\d+)',t());assert area,'no pixel area for the window'
  x,y,_,_=map(int,tile[-1]);aw,ah=map(int,area[-1])
  # Shell's tiling may give the window a pixel area smaller than 320x240, which cuts it off.
  assert aw>=260 and ah>=130,f'pixel area too small to check: {aw}x{ah}'
  return Image.open(shot).convert('RGB'),(x+8,y+34),(aw,ah)
 def pixels(img,org,area):
  for py in range(min(H,area[1])):
   for px in range(min(W,area[0])):
    r,gr,b=img.getpixel((org[0]+px,org[1]+py));yield px,py,(r<<16)|(gr<<8)|b
 try:
  g['wait'](r'\[bell.sound.ready\] card=hda stream=\d+ rate=48000 channels=2',0,300)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(1)

  # The demo: scripted play, a fixed score and a fixed last frame.
  s=len(t());type_line(f'run driftspark --demo {SEED} {FRAMES}')
  line(r'ready mode=demo w=320 h=240 sound=on',s,300)
  target,task,surface=launched(s)
  g['wait'](r'\[sdl.present\] n=4 w=320 h=240',s,60)
  line(re.escape(DEMO),s,300)
  line(r'demo checksum='+CHECKSUM,s,30)
  g['wait'](rf'\[std\.exit\] target={target} task={task} status=0\n',s,60)
  audio=g['wait'](r'\[sdl.audio\] rate=48000 frames=(\d+) nonsilent=(\d+)\n',s,60);report.append(audio[0].strip())
  assert int(audio[2])>=4800,f'only {audio[2]} non-silent frames of game sound reached the ring'
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120);time.sleep(1)

  # The game itself: the title screen.
  s=len(t());type_line('run driftspark')
  line(r'ready mode=play w=320 h=240 sound=on',s,300)
  target,task,surface=launched(s)
  time.sleep(2.5);img,org,area=window(surface,target,task,'title')
  amber=dark=0
  for px,py,c in pixels(img,org,area):
   if 40<=py<=80 and near(c,0xffc040):amber+=1
   if near(c,0x070a18,6):dark+=1
  assert amber>=600,f'the title shows {amber} amber pixels'
  assert dark>=min(W,area[0])*min(H,area[1])*3//4,f'only {dark} pixels of the window are the dark field'

  # Launch, and push the craft right.
  s=len(t());g['focus'](surface);time.sleep(.5);q.key('spc');line(r'start',s,30);time.sleep(.5)
  s=len(t());q.key('right')
  before=line(r'steer dir=right x=(\d+) y=(\d+)',s,30)
  after=line(r'drift x=(\d+) y=(\d+)',s,30)
  x0,y0,x1,y1=int(before[1]),int(before[2]),int(after[1]),int(after[2])
  assert x1-x0>=12 and abs(y1-y0)<=2,f'the craft went from ({x0},{y0}) to ({x1},{y1})'
  # The screen agrees: the craft's cyan hull is right of where the push began. Display
  # shows frames a little late, so let the drift settle first.
  time.sleep(1.5);img,org,area=window(surface,target,task,'moved')
  hull=[(px,py) for px,py,c in pixels(img,org,area) if py>=HUD and (near(c,0x40e0ff,24) or near(c,0x206878,16))]
  assert len(hull)>=8,f'no craft on screen ({len(hull)} hull pixels)'
  cx=sum(x for x,_ in hull)/len(hull);cy=sum(y for _,y in hull)/len(hull)
  assert cx>=x1 and abs(cy-y0)<=6,f'the craft is drawn at ({cx:.0f},{cy:.0f}), pushed from ({x0},{y0})'

  # Pause, resume, quit.
  s=len(t());q.key('p');line(r'pause on',s,30);q.key('p');line(r'pause off',s,30)
  s=len(t());q.key('esc');quit=line(r'quit score=(\d+) best=\d+',s,30)
  g['wait'](rf'\[std\.exit\] target={target} task={task} status=0\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text and 'panicked' not in text
  q.cmd('quit');g['proc'].wait(30)
  heard=[(round(st,2),round(ln,2),round(hz)) for st,ln,hz in segments(wav)]
  assert len(heard)>=4,f'the card played {len(heard)} separate sounds: {heard}'
  for r in report:print(r)
  print({'ok':True,'demo_score':417,'checksum':CHECKSUM,'nonsilent':int(audio[2]),'sounds':len(heard),'title_amber':amber,'moved':(x0,x1),'drawn_x':round(cx),'area':area})
 finally:g['stop']()
if __name__=='__main__':main()
