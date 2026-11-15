#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Actual responsive editor/mixer/device controls; metadata/pixels, no audio claim."""
import sys,os,ast,re,json,shutil,hashlib,time,argparse,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'scripts'))
from moan_workstation_gate import Probe,sha
from moan_gate_geometry import ready,viewport,origin
from quil_autosave_gate import boot
from install_program import install
p=argparse.ArgumentParser(description=__doc__)
for name in('iso','source-disk','out'):p.add_argument('--'+name,type=Path,required=True)
a=p.parse_args();OUT=a.out.resolve();OUT.mkdir(exist_ok=False,parents=True)
inputs={str(p):sha(p)for p in [a.iso,a.source_disk,Path(__file__),ROOT/'scripts/moan_workspace_gate.py',*[ROOT/'iso_root/programs'/n for n in('moan','files')]]}
for d in('apps/moan','crates/moan-core','crates/weft','crates/sex-app','crates/sex-pdx','crates/sex-font','assets/fonts','scripts'):
 for f in(ROOT/d).rglob('*'):
  if f.is_file()and'__pycache__'not in f.parts and'target'not in f.relative_to(ROOT/d).parts:inputs[str(f)]=sha(f)
(OUT/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n');(OUT/'source-commit.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True))
# Reuse the exact tested physical-input class without executing its gate body.
source=ROOT/'scripts/moan_workspace_gate.py';node=next(n for n in ast.parse(source.read_text()).body if isinstance(n,ast.ClassDef)and n.name=='Driver')
wav=OUT/'unused-audio-capture.wav';ns=dict(Probe=Probe,re=re,time=time,wav=wav,viewport=viewport,origin=origin);exec(compile(ast.Module(body=[node],type_ignores=[]),str(source),'exec'),ns);Driver=ns['Driver']

def park(c,surface,target):
 # Top-left large jumps are deliberately rejected by Shell's tablet guard.
 # A midpoint of the global bar is outside app pixels without that rejection.
 mid=c.rw//2
 accepted=list(re.finditer(r'\[shell.abs.normalize\] raw_x=\d+ raw_y=\d+ sx=(\d+) sy=(\d+) accepted=1 reason=ok',c.text()))
 old=tuple(map(int,accepted[-1].groups()))if accepted else None
 if old not in[(mid-1,9),(mid,9),(mid-1,10),(mid,10)]:
  since=len(c.text());c.move(mid,10)
  c.g['wait'](rf'\[shell.abs.normalize\] raw_x=\d+ raw_y=\d+ sx=(?:{mid-1}|{mid}) sy=(?:9|10) accepted=1 reason=ok',since,60)
 fresh=c.refresh(surface)
 c.g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w={fresh[2][0]} h={fresh[2][1]}',fresh[0],120)
 return fresh

disk=OUT/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('moan','files')])
os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0';os.environ['SEXOS_GATE_BOUND']='900'
g=boot(a.iso,disk,OUT,'narrow-workspaces','640x480');shots=[]
try:
 c=Driver(g,OUT,'640x480');g['wait'](r'\[app.private.retired\] target=41 ',0,180)
 start,target,task,surface=c.launch('moan');g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
 c.command(surface,'ctrl','shift','i');c.command(surface,'ctrl','shift','backspace');c.command(surface,'ctrl','j')
 st,offset,size=c.refresh(surface);assert size==(624,410),('actual framebuffer/client mismatch',size)
 m=list(re.finditer(r'\[moan.steps\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',c.text()[st:]))[-1];x,y,w,h=map(int,m.groups());assert 0<=x and 0<=y and x+w<=624 and y+h<=410
 ox,oy=offset;since=len(c.text());c.move(ox+x+(64+1.5*1056/16)*w/1120,oy+y+(24+3*22+11)*h/200);c.button(True);c.button(False);c.midi(since,1,0);park(c,surface,target)
 def step_pixel(width,height,rgb):
  assert(width,height)==(640,480)
  xx=ox+x+round((64+1.5*1056/16)*w/1120);yy=oy+y+24+3*22+10;assert int.from_bytes(rgb[(yy*width+xx)*3:(yy*width+xx)*3+3],'big')==0x388d74
 shots.append(c.shot(surface,'narrow-step-pointer',step_pixel,dict(notes=1,key=60,step=1,client=[624,410])))
 c.command(surface,'ctrl','p');st,(ox,oy),size=c.refresh(surface);m=list(re.finditer(r'\[moan.piano\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',c.text()[st:]))[-1];x,y,w,h=map(int,m.groups());assert 0<=x and 0<=y and x+w<=624 and y+h<=410
 def piano_pixel(width,height,rgb):
  xx=ox+x+32+round((w-32)*.08);yy=oy+y+114;assert int.from_bytes(rgb[(yy*width+xx)*3:(yy*width+xx)*3+3],'big')==0x38a574
 shots.append(c.shot(surface,'narrow-piano-same-note',piano_pixel,dict(notes=1,key=60,source='same persisted MIDI region; workspace change adds no note')))
 c.command(surface,'ctrl','m');c.click(surface,'Master');c.mix(surface,None,'master_level',50);c.mix(surface,None,'master_level',100)
 c.mix(surface,0,'mute',1);c.mix(surface,0,'mute',0)
 def mix_pixels(width,height,rgb):
  st,_,size=c.refresh(surface);m=list(re.finditer(r'\[moan.mixer.control\] name=master_level x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) value=10000',c.text()[st:]))[-1];x,y,w,h=map(int,m.groups());assert 0<=x and 0<=y and x+w<=624 and y+h<=410
  assert len(set(bytes(rgb[i:i+3])for i in range(0,len(rgb),3)))>40
 park(c,surface,target);shots.append(c.shot(surface,'narrow-mix-real-controls',mix_pixels,dict(tracks=1,master=100,mute=False,proof='actual control changes and bounded rectangles; no audible claim')))
 c.click(surface,'Instruments');st,(ox,oy),size=c.refresh(surface)
 def device_pixels(width,height,rgb):
  matches=list(re.finditer(r'\[moan.plugin.paint\] .*? x=(\d+) y=(\d+) w=(\d+) h=(\d+) hash=(0x[0-9a-f]+) text_hex=([0-9a-f]+)',c.text()[st:]));assert matches
  m=matches[-1];x,y,w,h=map(int,m.groups()[:4]);assert bytes.fromhex(m[6]).decode()=='Offline render ready';assert 0<=x and 0<=y and x+w<=624 and y+h<=410
  value=0xcbf29ce484222325
  for row in range(oy+y,oy+y+h):
   for byte in rgb[(row*width+ox+x)*3:(row*width+ox+x+w)*3]:value=((value^byte)*0x100000001b3)&((1<<64)-1)
  assert value==int(m[5],16)
 park(c,surface,target);shots.append(c.shot(surface,'narrow-instruments-status',device_pixels,dict(status='Offline render ready',pixel_model='exact rendered label RGB-FNV')))
 st=c.command(surface,'ctrl','shift','l');g['wait'](r'\[moan.project\] tracks=1 frames=192000 selected=0 clips=1',st,120)
 c.close(surface,target,task,dirty=True);c.q.cmd('quit');g['proc'].wait(30)
finally:g['stop']()
assert all(sha(Path(p))==v for p,v in inputs.items()),'input source mutated'
result=dict(ok=True,scope=__doc__,source_commit=(OUT/'source-commit.txt').read_text().strip(),inputs_sha256=sha(OUT/'inputs.json'),screenshots=shots)
(OUT/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(ok=True,screenshots=len(shots))))
