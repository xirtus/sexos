#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Intermediate actual workspace and live-mixer UI/audio proof; no restricted-role claim."""
import sys,os,json,re,hashlib,struct,shutil,time,math,array,subprocess,argparse
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'scripts'))
from moan_workstation_gate import Probe,sha
from moan_gate_geometry import ready,viewport,origin,discard
from quil_autosave_gate import boot
from install_program import install
from moan_gate import open_recording
from check_disk_persistence import ppm_rgb
parser=argparse.ArgumentParser(description=__doc__)
for name in ('iso','source-disk','seed-elf','out'):
 parser.add_argument('--'+name,required=True,type=Path)
args=parser.parse_args()
ISO=args.iso.resolve();SEED=args.source_disk.resolve();SEED_ELF=args.seed_elf.resolve()
OUT=args.out.resolve();OUT.mkdir(parents=True,exist_ok=False)
inputs={str(p):sha(p)for p in [ISO,SEED,SEED_ELF,Path(__file__),*[ROOT/'iso_root/programs'/name for name in('moan','files')]]}
for d in('apps/moan','crates/moan-core','crates/moan-plugin','crates/serenade-core','crates/weft','crates/sex-app','crates/sex-pdx','assets/fonts','scripts'):
 for p in(ROOT/d).rglob('*'):
  if p.is_file()and'__pycache__'not in p.parts and'target'not in p.relative_to(ROOT/d).parts:inputs[str(p)]=sha(p)
(OUT/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
(OUT/'source-commit.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True))
disk=OUT/'nvme.img';shutil.copyfile(SEED,disk);install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('moan','files')]+[('moanrack',SEED_ELF.read_bytes())])
wav=OUT/'live.wav';os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0';os.environ['SEXOS_GATE_BOUND']='1800'
g=boot(ISO,disk,OUT,'workspace-live','1280x800');shots=[];phases=[]
class Driver(Probe):
 def chord(self,*keys):
  for down,group in ((True,keys),(False,tuple(reversed(keys)))):
   self.q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':key}}}for key in group]})
   time.sleep(.04 if down else .12)
 def refresh(self,surface):
  start=len(self.text());self.chord('ctrl','shift','l');m=self.g['wait'](r'\[moan.timeline\] x=-?\d+ y=-?\d+ w=\d+ h=\d+ start=\d+ span=\d+',start,60);start+=m.start();tile,size=viewport(self.g,surface,start);return start,origin(tile,size),size
 def click(self,surface,name):
  start,(ox,oy),size=self.refresh(surface)
  matches=list(re.finditer(rf'\[moan.control\] name={re.escape(name)} x=(-?\d+) y=(-?\d+)',self.text()[start:]));assert matches,name
  x,y=map(int,matches[-1].groups());assert 0<=x<size[0] and 0<=y<size[1],(name,x,y,size)
  self.move(ox+x,oy+y);self.button(True);self.button(False)
 def mix(self,surface,track,name,value=None):
  start,(ox,oy),size=self.refresh(surface)
  if track is None:
   pattern=rf'\[moan.mixer.control\] name={name} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) value=(-?\d+)'
  else:pattern=rf'\[moan.mixer.track\] track={track} name={name} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) value=(-?\d+)'
  # viewport is emitted AFTER this frame's complete control records.
  matches=list(re.finditer(pattern,self.text()[start:]));assert matches,pattern
  m=matches[-1];x,y,w,h,v=map(int,m.groups());assert w>0 and h>0 and 0<=x and 0<=y and x+w<=size[0] and y+h<=size[1],m[0]
  if value is None:return (ox+x,oy+y,w,h,v)
  wanted=value*100 if track is None else value
  if v==wanted:return max(0,(wav.stat().st_size-44)//4)
  if name in('mute','solo'):
   since=len(self.text());self.move(ox+x+w/2,oy+y+h/2);self.button(True);self.button(False)
   self.g['wait'](pattern[:-len('(-?\\d+)')]+str(wanted)+r'(?:\s|$)',since,60)
  else:
   lo,hi=(-100,100)if name=='pan'else(0,200)
   self.move(ox+x+8+(w-16)*(value-lo)/(hi-lo),oy+y+h/2);self.button(True);self.button(False)
   # A pointer can already achieve the target. End/Home would then be a no-op
   # and correctly emit no ValueChanged; do not demand a nonexistent receipt.
   fresh,_,_=self.refresh(surface);current=list(re.finditer(pattern,self.text()[fresh:]));assert current
   actual=int(current[-1][5]);actual=actual//100 if track is None else actual
   if actual==value:return max(0,(wav.stat().st_size-44)//4)
   # Pointer resolution can produce odd49, while keyboard step2 preserves
   # parity. Establish a known minimum, then exact bounded key increments.
   step=1 if track is None else 2
   units=(value-lo)//step;assert lo+units*step==value
   commands=[]if actual==lo else[('home',lo)]
   current=lo
   for key,count,delta in (('pgup',units//10,step*10),('right',units%10,step)):
    for _ in range(count):current+=delta;commands.append((key,current))
   if value==hi:commands=[('end',hi)]
   assert commands and commands[-1][1]==value
   for key,expected in commands:
    since=len(self.text());self.chord(key)
    expected=expected*100 if track is None else expected
    self.g['wait'](pattern[:-len('(-?\\d+)')]+str(expected)+r'(?:\s|$)',since,60)
  since,_,_=self.refresh(surface);matches=list(re.finditer(pattern,self.text()[since:]));assert matches
  assert int(matches[-1][5])==wanted,(matches[-1][0],value)
  return max(0,(wav.stat().st_size-44)//4)
 def select_file(self,fsurface,surface,name,since):
  self.g['wait'](r'\[files.app.question\] program=moan mode=Open',since,120);self.focus(fsurface);self.chord('ctrl','h');self.chord('ctrl','f');self.type(name);self.q.key('ret')
  self.g['wait'](rf'\[files.app.search\] base=/ query={name} results=1 .* finished=1',since,120);self.q.key('home');self.q.key('ret');self.g['wait'](rf'\[files.app.found.opened\] path=/{name}',since,120);self.q.key('ret');self.g['wait'](rf'\[files.pick.granted\] row=\d+ path=/{name}',since,120);self.focus(surface)
 def idle(self,surface,seconds):
  target=max(0,(wav.stat().st_size-44)//4)+round(seconds*48000);deadline=time.monotonic()+60
  while max(0,(wav.stat().st_size-44)//4)<target:
   assert time.monotonic()<deadline,'audio capture failed to progress';self.q.cmd('query-status');time.sleep(.1)
 def plate(self,surface,label,ack,expected):
  # Source changes apply only to future blocks; permit existing ring+card tail.
  finish=ack+round(2.5*48000);deadline=time.monotonic()+90
  while max(0,(wav.stat().st_size-44)//4)<finish:
   assert time.monotonic()<deadline,'audio plateau deadline';self.q.cmd('query-status');time.sleep(.1)
  phases.append(dict(name=label,ack_frame=ack,from_frame=ack+round(2.1*48000),to_frame=finish,expected=expected))
try:
 c=Driver(g,OUT,'1280x800');g['wait'](r'\[app.private.retired\] target=41 ',0,180);g['wait'](r'\[bell.sound.ready\]',0,180)
 seed=c.run('run moanrack @/ --seed-live-mixer');g['wait'](r'\[moanrack.live.seed\] ok=1 frames=576000 left_hz=400 right_hz=600 developer_only=1',seed,180);m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) ',seed,120);g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',seed,120)
 start,ft,ftask,fs=c.launch('files','files');g['wait'](r'\[files.app.ready\].*portal=1',start,180)
 start,target,task,surface=c.launch('moan');g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
 for code in('i','b','d'):c.command(surface,'ctrl','shift',code)
 selected=c.command(surface,'ctrl','shift','r');c.q.key('home');g['wait'](r'\[moan.selection\] track=0 ',selected,120)
 start=c.command(surface,'ctrl','t');m=g['wait'](r'\[moan.timeline\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) start=0 span=192000',start,120);tile,size=viewport(g,surface,start);ox,oy=origin(tile,size);tx,ty,tw,th=map(int,m.groups());assert th==188
 def arrange_pixels(width,height,pixels):
  for row,colour in enumerate((0x384a72,0x265e66,0x336045)):
   if row==0:colour=0x48597d
   x,y=ox+tx+round(tw*.93),oy+ty+20+row*56+4;actual=int.from_bytes(pixels[(y*width+x)*3:(y*width+x)*3+3],'big');assert actual==colour,(row,hex(actual),hex(colour))
 shots.append(c.shot(surface,'arrange-three-real-tracks',arrange_pixels,dict(tracks=3,midi_regions=3)))
 c.click(surface,'Instruments');start,_,_=c.refresh(surface);g['wait'](r'\[moan.plugin.status\] state=Idle .* text=Offline render ready',start,120)
 def device_pixels(w,h,p):
  m=list(re.finditer(r'\[moan.plugin.paint\] .*? x=(\d+) y=(\d+) w=(\d+) h=(\d+) hash=(0x[0-9a-f]+) text_hex=([0-9a-f]+)',c.text()))[-1];x,y,ww,hh=map(int,m.groups()[:4]);assert bytes.fromhex(m[6]).decode()=='Offline render ready';tile,size=viewport(g,surface,0);xx,yy=origin(tile,size);value=0xcbf29ce484222325
  for row in range(yy+y,yy+y+hh):
   for byte in p[(row*w+xx+x)*3:(row*w+xx+x+ww)*3]:value=((value^byte)*0x100000001b3)&((1<<64)-1)
  assert value==int(m[5],16),'actual Instruments status strip differs from painted label'
 shots.append(c.shot(surface,'instruments-offline-controls',device_pixels,dict(status='Offline render ready',hosting_scope='ordinary crash-contained offline helper; no restricted-provider claim')))
 c.command(surface,'ctrl','shift','backspace');c.command(surface,'ctrl','j')
 for count,index in enumerate((1,3,7,13),1):
  st=c.command(surface,'ctrl','j');m=g['wait'](r'\[moan.steps\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',st,120);tile,size=viewport(g,surface,st);ox,oy=origin(tile,size);x,y,w,h=map(int,m.groups());since=len(c.text());c.move(ox+x+(64+(index+.5)*1056/16)*w/1120,oy+y+(24+3*22+11)*h/200);c.button(True);c.button(False);c.midi(since,count,0)
 def step_pixels(width,height,p):
  for index in(1,3,7,13):
   xx=ox+x+round((64+(index+.5)*1056/16)*w/1120);yy=oy+y+24+3*22+10;assert int.from_bytes(p[(yy*width+xx)*3:(yy*width+xx)*3+3],'big')==0x388d74
 shots.append(c.shot(surface,'step-shared-region',step_pixels,dict(track=0,steps=[1,3,7,13],bar_labels='bar.beat.eighth',notes=4)))
 start=c.command(surface,'ctrl','n');discard(g,surface,(1280,800),start);g['wait'](r'\[moan.project.new\] tracks=0 clean=1',start,120)
 for i in range(8):st=c.command(surface,'ctrl','shift','t');c.command(surface,'ctrl','shift','l');g['wait'](rf'\[moan.project\] tracks={i+1} ',st,120)
 c.command(surface,'ctrl','m');c.click(surface,'Master');c.mix(surface,None,'master_level',100)
 def mix_pixels(width,height,p):
  fresh=c.text();records=[]
  for index in range(8):
   matches=list(re.finditer(rf'\[moan.mixer.track\] track={index} name=volume x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) value=100',fresh));assert matches;records.append(tuple(map(int,matches[-1].groups())))
  assert len({(a,b)for a,b,_,_ in records})==8,'not eight distinct real strips'
  for xx,yy,ww,hh in records:assert ww>0 and hh>0 and 0<=xx and 0<=yy and xx+ww<=1264 and yy+hh<=730
  colours=set(bytes(p[i:i+3])for i in range(0,len(p),3));assert len(colours)>40
 shots.append(c.shot(surface,'mix-eight-real-tracks-master',mix_pixels,dict(tracks=8,master=True,controls='native level/pan/M-S; horizontal fader toolkit boundary retained')))
 start=c.command(surface,'ctrl','n');discard(g,surface,(1280,800),start);g['wait'](r'\[moan.project.new\] tracks=0 clean=1',start,120)
 for i in range(2):
  st=c.command(surface,'ctrl','o');c.select_file(fs,surface,'live.wav',st);m=c.command(surface,'ctrl','shift','l');g['wait'](rf'\[moan.project\] tracks={i+1} frames=576000 ',m,120)
 c.command(surface,'ctrl','m');c.click(surface,'Master');c.mix(surface,None,'master_level',100)
 # Three bounded passes keep every measured plateau within the12s source.
 for run in range(3):
  before=max(0,(wav.stat().st_size-44)//4);st=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.prefilled\] frames=64512 from=0',st,120);c.idle(surface,1.0)
  phases.append(dict(name=f'baseline-{run}',from_frame=before+int(.3*48000),to_frame=before+int(.8*48000),expected=[2.0,2.0]))
  if run==0:
   ack=c.mix(surface,None,'master_level',50);c.plate(surface,'master50-live',ack,[1.0,1.0])
   ack=c.mix(surface,0,'pan',100);c.plate(surface,'pan0-right-live',ack,[.5,1.0])
  elif run==1:
   ack=c.mix(surface,0,'solo',1);c.plate(surface,'solo0-live',ack,[1.0,1.0])
   ack=c.mix(surface,0,'volume',50);c.plate(surface,'track0-level50-live',ack,[.5,.5])
  else:
   ack=c.mix(surface,0,'mute',1);c.plate(surface,'mute0-live',ack,[1.0,1.0])
   ack=c.mix(surface,0,'mute',0);c.plate(surface,'unmute0-live',ack,[2.0,2.0])
  g['wait'](r'\[moan.finished\] frames=672000',st,180)
  if run<2:
   # One musical undo restores the whole live pass, not just its final mouse move.
   undo=c.command(surface,'ctrl','z');c.refresh(surface);c.mix(surface,None,'master_level',100);c.mix(surface,0,'pan',0);c.mix(surface,0,'mute',0);c.mix(surface,0,'solo',0);c.mix(surface,0,'volume',100)
 c.close(surface,target,task,dirty=True);c.focus(fs);c.q.key('f11');c.q.cmd('quit');g['proc'].wait(30)
finally:g['stop']()
with open_recording(wav)as f:
 assert(f.getnchannels(),f.getsampwidth(),f.getframerate())==(2,2,48000);pcm=array.array('h',f.readframes(f.getnframes()))
if sys.byteorder!='little':pcm.byteswap()
measurements=[]
for phase in phases:
 a,b=phase['from_frame'],phase['to_frame'];assert b*2<=len(pcm),(phase,len(pcm)//2)
 rms=[];hz=[]
 for channel,frequency,amplitude,multiple in zip((0,1),(400,600),(6000,8000),phase['expected']):
  part=pcm[a*2+channel:b*2:2];energy=math.sqrt(sum(v*v for v in part)/len(part));expected=amplitude*multiple/math.sqrt(2);assert abs(energy/expected-1)<.04,('audible live mix',phase,channel,energy,expected)
  crossings=[i for i,(x,y)in enumerate(zip(part,part[1:]))if x<0<=y];measured=(len(crossings)-1)*48000/(crossings[-1]-crossings[0]);assert abs(measured-frequency)<1,(phase,channel,measured)
  # Every10ms interior window remains active: queue updates cannot hide dropouts.
  assert all(sum(v*v for v in part[i:i+480])/480>100**2 for i in range(0,len(part)-480,480)),('dropout',phase,channel)
  rms.append(energy);hz.append(measured)
 measurements.append(dict(**phase,rms=rms,hz=hz))
assert all(sha(Path(name))==digest for name,digest in inputs.items()),'frozen source/input mutated during gate'
result=dict(ok=True,scope=__doc__,source_commit=(OUT/'source-commit.txt').read_text().strip(),inputs_sha256=sha(OUT/'inputs.json'),screenshots=shots,audio=measurements,queue_semantics='Only newly rendered blocks change; gate measures after2.1s ring/card tail. No instant or hard realtime guarantee.',source_edits_remained_locked=True)
(OUT/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(ok=True,screenshots=len(shots),audio_phases=len(measurements))))
