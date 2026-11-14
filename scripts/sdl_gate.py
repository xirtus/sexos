#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""An SDL2 program runs on SexOS: apps/sdltest (C), linked with SDL2 2.30.9 and the
SexOS backends in crates/sex-sdl.

The gate types `run sdltest` into Spindle, as a person would, and checks four things:

* video: the program fills four coloured quarters and a white middle with
  SDL_FillRect and SDL_UpdateWindowSurface. Its report must name the expected
  pixels, and a QEMU screendump must show those colours, on a 7 px grid, in the part of
  the window that Shell's tiling leaves visible.
* audio: it queues one second of a 440 Hz tone with SDL_OpenAudio and
  SDL_QueueAudio. The backend's record line must say the frames reached the sound
  ring. The sound card's output, recorded to a WAV file through QEMU's Intel HD
  Audio, must hold a tone of about one second at 440 Hz.
* timer: SDL_Delay(100) must take 100 to 200 ms by SDL_GetTicks.
* input: a K key pressed in the window must arrive as SDL_KEYDOWN (scancode 14,
  SDL_SCANCODE_K), then SDL_TEXTINPUT "k", then SDL_KEYUP. The program must then
  quit with status 0 and its window must close."""
import argparse,os,re,shutil,time
from pathlib import Path
from PIL import Image
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','900')
ROOT=Path(__file__).resolve().parents[1]

PLAIN={' ':'spc','.':'dot','-':'minus'}
W,H=320,240
def expected(px,py):
 """The colour sdltest draws at (px, py), or None within 3 px of an edge between colours."""
 if abs(px-W//2)<=23 and abs(py-H//2)<=23:
  return 0xffffff if abs(px-W//2)<=17 and abs(py-H//2)<=17 else None
 if abs(px-W//2)<=3 or abs(py-H//2)<=3:return None
 return [[0xe02020,0x20c040],[0x2040e0,0xf0d020]][py>=H//2][px>=W//2]

def near(a,b,tol=8):return all(abs(((a>>s)&255)-((b>>s)&255))<=tol for s in(0,8,16))

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/sdltest').read_bytes();assert elf[:4]==b'\x7fELF'
 if b'This build has no C compiler output (musl headers were missing).' in elf:
  raise SystemExit('sdltest is the no-compiler fallback: install musl headers or set MUSL_INCLUDE, then rebuild before running this gate')
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('sdltest',elf)])
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=a0,path={wav},out.frequency=48000,out.channels=2 -device intel-hda -device hda-output,audiodev=a0'
 from quil_autosave_gate import boot
 from sound_gate import segments
 g=boot(a.iso,disk,out,'sdl',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
  for ch in line:
   s=len(t());q.key(PLAIN.get(ch,ch))
   g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=\d+\n',s,30)
  q.key('ret')
 try:
  g['wait'](r'\[bell.sound.ready\] card=hda stream=\d+ rate=48000 channels=2',0,300)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(1)
  s=len(t());type_line('run sdltest')
  report=[]
  def line(pattern,timeout=120):
   m=g['wait'](r'\[std\.out\] \[sdltest\] '+pattern+r'\n',s,timeout);report.append(m[0].strip());return m
  line(r'init version=2\.30\.9 video=sexos audio=sexos',300)
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  target,task,surface=int(launch[1]),int(launch[2]),int(launch[3])
  line(r'drew w=320 h=240 format=SDL_PIXELFORMAT_RGB888 tl=e02020 tr=20c040 bl=2040e0 br=f0d020 mid=ffffff')
  timer=line(r'timer delay100=(\d+)');assert 100<=int(timer[1])<=200,f'SDL_Delay(100) took {timer[1]} ms'
  line(r'audio open freq=48000 channels=2 samples=\d+ queued=192000')
  drained=line(r'audio drained left=0 ms=(\d+)',60)
  audio=g['wait'](r'\[sdl.audio\] rate=48000 frames=(\d+) nonsilent=(\d+)\n',s,60);report.append(audio[0].strip())
  # A sine is 0 near each crossing, so a few frames of the tone are silent.
  assert int(audio[2])>=47000,f'only {audio[2]} frames of the tone reached the ring'
  line(r'waiting for key')
  # The window as Display shows it: the program's pixels start 8 px in and 34 px down,
  # and Shell's tiling may give it a pixel area smaller than 320x240, which cuts it off.
  time.sleep(1.5);shot=out/'sdltest.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  tile=re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());assert tile,'no tile for the window'
  area=re.findall(rf'\[shell.program.resize\] pd={target} task={task} w=(\d+) h=(\d+)',t());assert area,'no pixel area for the window'
  x,y,_,_=map(int,tile[-1]);aw,ah=map(int,area[-1]);vw,vh=min(W,aw),min(H,ah)
  assert vw>=200 and vh>=130,f'pixel area too small to check: {aw}x{ah}'
  img=Image.open(shot).convert('RGB');bx,by=x+8,y+34;checked=0;seen={}
  for py in range(3,vh-3,7):
   for px in range(3,vw-3,7):
    want=expected(px,py)
    if want is None:continue
    r,gr,b=img.getpixel((bx+px,by+py));got=(r<<16)|(gr<<8)|b;checked+=1
    assert near(got,want),f'pixel ({px},{py}) of the window is {got:06x}, not {want:06x}'
    seen[f'{want:06x}']=seen.get(f'{want:06x}',0)+1
  assert len(seen)==5 and checked>400,f'checked {checked} pixels, colours {seen}'
  # The key.
  s=len(t());g['focus'](surface);time.sleep(.5);q.key('k')
  line(r'key down scancode=14 name=K sym=K',60);line(r'text k',30);line(r'key up scancode=14',30);line(r'done',30)
  g['wait'](rf'\[std\.exit\] target={target} task={task} status=0\n',s,60)
  # A program that took its window for pixels closes it when it ends.
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text and 'panicked' not in text
  q.cmd('quit');g['proc'].wait(30)
  heard=[(round(st,2),round(ln,2),round(hz)) for st,ln,hz in segments(wav)]
  tone=[x for x in heard if 0.8<=x[1]<=1.3 and abs(x[2]-440)<=9]
  assert tone,f'no one-second 440 Hz tone in the card output: {heard}'
  for r in report:print(r)
  print({'ok':True,'drain_ms':int(drained[1]),'frames':int(audio[1]),'nonsilent':int(audio[2]),'tone':tone[0],'pixels_checked':checked,'colours':seen,'area':(aw,ah)})
 finally:g['stop']()
if __name__=='__main__':main()
